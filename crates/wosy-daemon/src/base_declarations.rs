//! Rooted, lazy canonical base declarations. No semantic Build/Check query.
use super::{
    file_source, key, read_comments, read_cst, read_cst_with, read_node,
    source_proposal::CstWork, storage_worker::QueryPhases,
    write_edge, write_node, Edge, Result, SourceStore, INTERN,
};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::hash::Hash;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Mutex as AsyncMutex;
use surrealkv::{Durability, Mode, Transaction};
use wosy_compiler::{
    derive_scalar_declarations_from_cst, Diagnostic, ScalarSourceDeclarations, BASE_DECLARATION_ABI,
};
use wosy_protocol::{FileKey, Kind, Node, NodeId};
use wosy_syntax::{AuthoredSpanMap, ByteSpan, CanonicalCstRoot, CstNode, SourceIdentity};

const BASE_DECLARATION_QUERIES: u8 = 13;
const INLINE_BYTES: usize = 64 * 1024;
const FILES_PER_PHASE: usize = 16;
const INLINE: u8 = 0;
const BLOB: u8 = 1;

type QueryKey = [u8; 33];

pub(super) struct Flights<K = QueryKey, T = ()>(Mutex<HashMap<K, Weak<AsyncMutex<T>>>>);

impl<K, T> Default for Flights<K, T> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

impl<K: Eq + Hash> Flights<K> {
    pub(super) fn acquire(&self, key: K) -> Arc<AsyncMutex<()>> {
        self.try_acquire_with(key, || Some(())).expect("unbounded existing query flight")
    }
}

impl<K: Eq + Hash, T> Flights<K, T> {
    pub(super) fn try_acquire_with(&self, key: K, new: impl FnOnce() -> Option<T>) -> Option<Arc<AsyncMutex<T>>> {
        let mut flights = self.0.lock().expect("declaration query registry");
        if let Some(gate) = flights.get(&key).and_then(Weak::upgrade) {
            return Some(gate);
        }
        flights.retain(|_, gate| gate.strong_count() != 0);
        let gate = Arc::new(AsyncMutex::new(new()?));
        flights.insert(key, Arc::downgrade(&gate));
        Some(gate)
    }

    /// Completed state remains reachable only from callers registered before
    /// this cut. A newer generation of the same key is never removed.
    pub(super) fn detach(&self, key: &K, gate: &Arc<AsyncMutex<T>>) {
        let mut flights = self.0.lock().expect("declaration query registry");
        if flights.get(key).is_some_and(|current| current.ptr_eq(&Arc::downgrade(gate))) {
            flights.remove(key);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryDisposition {
    Derived,
    Reused,
}

/// The canonical declaration shape has executable coordinates and detached
/// IDs. Authored origins/locations belong to the exact root selecting a file.
pub struct BaseDeclarationsView {
    pub id: NodeId,
    pub executable: NodeId,
    pub module: NodeId,
    pub source: SourceIdentity,
    pub disposition: QueryDisposition,
    declarations: ScalarSourceDeclarations,
    spans: AuthoredSpanMap,
}

impl BaseDeclarationsView {
    pub fn executable_declarations(&self) -> &ScalarSourceDeclarations {
        &self.declarations
    }

    pub fn authored_span(&self, executable: ByteSpan) -> ByteSpan {
        self.spans.project(executable)
    }

    /// These are declaration-derivation diagnostics, not type/ownership checks.
    pub fn authored_diagnostics(&self) -> Vec<Diagnostic> {
        authored_declaration_diagnostics(&self.declarations, &self.source, &self.spans)
    }
}

pub(super) fn authored_declaration_diagnostics(
    declarations: &ScalarSourceDeclarations, source: &SourceIdentity, spans: &AuthoredSpanMap,
) -> Vec<Diagnostic> {
    let mut diagnostics = declarations.diagnostics().to_vec();
    for diagnostic in &mut diagnostics {
        for label in &mut diagnostic.labels {
            label.span.source = source.clone();
            label.span.range = spans.project(label.span.range);
        }
    }
    diagnostics
}

pub(super) fn query_key(image: NodeId, executable: NodeId) -> [u8; 33] {
    let mut hasher = blake3::Hasher::new_derive_key(BASE_DECLARATION_ABI);
    hasher.update(wosy_syntax::base_cst_schema()).update(&image.0).update(&executable.0);
    key(BASE_DECLARATION_QUERIES, NodeId(*hasher.finalize().as_bytes()))
}

enum Payload<'a> {
    Inline(&'a [u8]),
    Blob(blake3::Hash),
}

fn decode_payload(data: &[u8]) -> Result<Payload<'_>> {
    let Some((tag, body)) = data.split_first() else {
        return Err("missing canonical declaration payload".into());
    };
    match *tag {
        INLINE => Ok(Payload::Inline(body)),
        BLOB => Ok(Payload::Blob(blake3::Hash::from_bytes(body.try_into()?))),
        _ => Err("invalid canonical declaration payload kind".into()),
    }
}

pub(super) fn cas_digest(data: &[u8]) -> Result<Option<NodeId>> {
    Ok(match decode_payload(data)? {
        Payload::Inline(_) => None,
        Payload::Blob(digest) => Some(NodeId(*digest.as_bytes())),
    })
}

fn detached_identity(executable: NodeId) -> SourceIdentity {
    SourceIdentity::new(String::new(), String::new(), String::new(), executable.hex())
}

fn derive_base(executable: NodeId, cst: CstNode) -> ScalarSourceDeclarations {
    derive_scalar_declarations_from_cst(&CanonicalCstRoot::new(detached_identity(executable), cst))
}

struct BoundedDeclarationBytes<'a> {
    remaining: &'a mut usize,
    encoded: Vec<u8>,
}

impl Write for BoundedDeclarationBytes<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        *self.remaining = self.remaining.checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("prepared base declaration payload exceeds its encoded byte budget"))?;
        self.encoded.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

struct BaseOutput {
    id: NodeId,
    declarations: Option<ScalarSourceDeclarations>,
    disposition: QueryDisposition,
}

struct BaseAddition {
    query: QueryKey,
    id: NodeId,
    executable: NodeId,
    encoded: Vec<u8>,
}


impl SourceStore {
    /// Resolve only a file inside a currently supported published source
    /// snapshot. Derived queries are optional: snapshot reads never need CAS.
    pub async fn base_declarations(
        &self, root: NodeId, key: &FileKey,
    ) -> Result<Option<BaseDeclarationsView>> {
        let Some(view) = self.snapshot(root)? else { return Ok(None); };
        let Some(file) = view.files.get(key) else { return Ok(None); };
        let module = view.modules[key];
        let file = file.clone();
        // Materialization can start another transaction or restore a CAS
        // file. Retain the root even after releasing the admission read.
        let super::SourceView { tx, read: _read, .. } = view;
        drop(tx);
        self.authored_base_declarations(&file, module, None).await.map(Some)
    }

    pub(super) async fn authored_base_declarations(
        &self, file: &super::FileEntry, module: NodeId, projected: Option<NodeId>,
    ) -> Result<BaseDeclarationsView> {
        let executable = file.executable;
        let source = file_source(file)?;
        let tx = self.tree.begin_with_mode(Mode::ReadOnly)?;
        let spans = AuthoredSpanMap::from_comments(&read_comments(&tx, file.annotation)?);
        drop(tx);
        let phases = QueryPhases::direct(self);
        let image = self.derivation_image();
        let output = match projected {
            Some(id) => self.projected_bases(phases, image, &[(executable, id)]).await?,
            None => self.query_bases(phases, image, &[executable], true).await?,
        }.pop().expect("one requested declaration body");
        Ok(BaseDeclarationsView {
            id: output.id, executable, module, source, disposition: output.disposition,
            declarations: output.declarations.expect("requested declaration payload"), spans,
        })
    }

    pub(super) async fn projected_bodies(
        &self, phases: QueryPhases<'_>, image: Option<NodeId>, files: &[(NodeId, NodeId)],
    ) -> Result<Vec<ScalarSourceDeclarations>> {
        Ok(self.projected_bases(phases, image, files).await?.into_iter()
            .map(|output| output.declarations.expect("requested declaration payload")).collect())
    }

    async fn projected_bases(
        &self, phases: QueryPhases<'_>, image: Option<NodeId>, files: &[(NodeId, NodeId)],
    ) -> Result<Vec<BaseOutput>> {
        let mut result = Vec::with_capacity(files.len());
        for chunk in files.chunks(FILES_PER_PHASE) {
            let cached = if image.is_some() {
                let files = chunk.to_vec();
                phases.run(move |store| {
                    let read = store.tree.begin_with_mode(Mode::ReadOnly)?;
                    files.into_iter().map(|(executable, id)| store.read_base_declarations(&read, id, executable)
                        .map(|body| body.map(|declarations| BaseOutput {
                            id, declarations: Some(declarations), disposition: QueryDisposition::Reused,
                        }))).collect::<Result<Vec<_>>>()
                }).await?
            } else {
                (0..chunk.len()).map(|_| None).collect()
            };
            let missing: Vec<_> = chunk.iter().zip(&cached)
                .filter_map(|(&(executable, _), hit)| hit.is_none().then_some(executable)).collect();
            let mut repaired = self.query_bases(phases, image, &missing, true).await?.into_iter();
            for ((_, expected), hit) in chunk.iter().zip(cached) {
                let output = hit.unwrap_or_else(|| repaired.next().expect("requested missing declaration"));
                if output.id != *expected {
                    return Err("recomputed declarations differ from the rooted projection".into());
                }
                result.push(output);
            }
        }
        Ok(result)
    }

    /// Compose a graph using IDs, without decoding already derived bodies.
    pub(super) async fn ensure_base_ids(
        &self, phases: QueryPhases<'_>, image: Option<NodeId>, executables: &[NodeId],
    ) -> Result<Vec<(NodeId, QueryDisposition)>> {
        Ok(self.query_bases(phases, image, executables, false).await?.into_iter()
            .map(|output| (output.id, output.disposition)).collect())
    }

    async fn query_bases(
        &self, phases: QueryPhases<'_>, image: Option<NodeId>, executables: &[NodeId], load: bool,
    ) -> Result<Vec<BaseOutput>> {
        let mut results = Vec::with_capacity(executables.len());
        for chunk in executables.chunks(FILES_PER_PHASE) {
            let tagged: Vec<_> = chunk.iter().map(|&executable| {
                (executable, image.map(|image| query_key(image, executable)))
            }).collect();
            let mut keys: Vec<_> = tagged.iter().filter_map(|(_, query)| *query).collect();
            keys.sort_unstable();
            keys.dedup();
            // All multi-file queries acquire the same per-executable flights
            // in key order; store reads and both publication phases stay inside
            // those flights. Holding one never blocks a Tokio worker thread.
            let mut permits = Vec::with_capacity(keys.len());
            for key in keys {
                permits.push(phases.lock_owned(self.base_flights.acquire(key)).await?);
            }
            let permits = Arc::new(permits);
            let active = Arc::clone(&permits);
            let (outputs, additions) = phases.run(move |store| {
                let _active = active;
                let read = store.tree.begin_with_mode(Mode::ReadOnly)?;
                let mut output = Vec::with_capacity(tagged.len());
                let mut additions = Vec::new();
                for (executable, query) in tagged {
                    let (value, added) = store.plan_base(&read, executable, query, load)?;
                    output.push(value);
                    if let Some(added) = added { additions.push(added); }
                }
                Ok((output, additions))
            }).await?;
            if !additions.is_empty() {
                let additions = Arc::new(additions);
                loop {
                    let batch = Arc::clone(&additions);
                    let active = Arc::clone(&permits);
                    let Some(mut tx) = phases.run(move |store| {
                        let _active = active;
                        store.base_transaction(&batch)
                    }).await? else { break; };
                    match tx.commit().await {
                        Ok(()) => break,
                        Err(surrealkv::Error::TransactionWriteConflict) => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            results.extend(outputs);
            drop(permits);
        }
        Ok(results)
    }

    fn plan_base(
        &self, read: &Transaction, executable: NodeId, query: Option<QueryKey>, load: bool,
    ) -> Result<(BaseOutput, Option<BaseAddition>)> {
        let cached = query.map(|query| read.get(&query)).transpose()?.flatten();
        if let Some(id) = cached {
            let id = NodeId(id.as_slice().try_into()?);
            if !load {
                return Ok((BaseOutput { id, declarations: None, disposition: QueryDisposition::Reused }, None));
            }
            if let Some(declarations) = self.read_base_declarations(read, id, executable)? {
                return Ok((BaseOutput { id, declarations: Some(declarations), disposition: QueryDisposition::Reused }, None));
            }
        }
        let declarations = derive_base(executable, read_cst(read, executable)?);
        let (id, encoded) = self.encode_base(executable, serde_json::to_vec(&declarations)?, query.is_some())?;
        let addition = query.map(|query| BaseAddition { query, id, executable, encoded });
        Ok((BaseOutput { id, declarations: load.then_some(declarations), disposition: QueryDisposition::Derived }, addition))
    }

    /// Derive the same detached declaration type and record identity as the
    /// published query, without creating a cache row or a CAS file.
    pub(super) fn prepare_base_body(
        &self, read: &Transaction, executable: NodeId, work: &mut CstWork, remaining: &mut usize,
    ) -> Result<(NodeId, ScalarSourceDeclarations)> {
        let declarations = derive_base(executable, read_cst_with(read, executable, Some(work))?);
        let mut bytes = BoundedDeclarationBytes { remaining, encoded: Vec::new() };
        serde_json::to_writer(&mut bytes, &declarations)?;
        let (id, _) = self.encode_base(executable, bytes.encoded, false)?;
        Ok((id, declarations))
    }

    fn encode_base(&self, executable: NodeId, bytes: Vec<u8>, persist: bool) -> Result<(NodeId, Vec<u8>)> {
        let data = self.pack_payload(bytes, persist)?;
        let encoded = Node { kind: Kind::BaseDeclarations, data, children: vec![executable] }.encode();
        Ok((Node::id(&encoded), encoded))
    }

    fn base_transaction(&self, additions: &[BaseAddition]) -> Result<Option<Transaction>> {
        let mut tx = self.tree.begin()?;
        tx.set_durability(Durability::Eventual);
        let mut changed = false;
        for addition in additions {
            let BaseAddition { query, id, executable, encoded } = addition;
            if let Some(existing) = tx.get(query)? {
                // A previously indexed body may have needed only its lost
                // CAS file restored by plan_base, under the same flight.
                if existing.as_slice() != id.0 {
                    return Err("base declaration query changed for an immutable input".into());
                }
                continue;
            }
            if tx.get(&key(INTERN, *id))?.is_none() {
                write_node(&mut tx, *id, encoded)?;
                write_edge(&mut tx, *id, Edge::DeclarationSource, *executable)?;
            }
            tx.set(query, &id.0)?;
            changed = true;
        }
        Ok(changed.then_some(tx))
    }

    fn read_base_declarations(
        &self, tx: &Transaction, id: NodeId, executable: NodeId,
    ) -> Result<Option<ScalarSourceDeclarations>> {
        let node = read_node(tx, id)?;
        if node.kind != Kind::BaseDeclarations || node.children != [executable] {
            return Err("invalid canonical declaration query result".into());
        }
        let blob;
        let data = match decode_payload(&node.data)? {
            Payload::Inline(body) => body,
            Payload::Blob(digest) => {
                match fs::read(self.blob_path(NodeId(*digest.as_bytes()))) {
                    Ok(bytes) if blake3::hash(&bytes) == digest => blob = bytes,
                    Ok(_) => return Ok(None),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error.into()),
                }
                blob.as_slice()
            }
        };
        let mut parser = serde_json::Deserializer::from_slice(data);
        parser.disable_recursion_limit();
        use serde::Deserialize;
        let declarations = ScalarSourceDeclarations::deserialize(&mut parser)?;
        parser.end()?;
        Ok(Some(declarations))
    }

    fn pack_payload(&self, bytes: Vec<u8>, persist: bool) -> Result<Vec<u8>> {
        if bytes.len() <= INLINE_BYTES {
            let mut payload = Vec::with_capacity(1 + bytes.len());
            payload.push(INLINE);
            payload.extend(bytes);
            return Ok(payload);
        }
        let digest = blake3::hash(&bytes);
        if persist {
            fs::create_dir_all(&self.blobs)?;
            let destination = self.blob_path(NodeId(*digest.as_bytes()));
            write_blob(&self.blobs, &destination, &bytes)?;
        }
        let mut payload = vec![BLOB];
        payload.extend_from_slice(digest.as_bytes());
        Ok(payload)
    }
}

fn write_blob(dir: &Path, destination: &Path, data: &[u8]) -> Result<()> {
    static NONCE: AtomicU64 = AtomicU64::new(0);
    let (temp, mut file) = loop {
        let serial = NONCE.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".projection-{}-{serial}.tmp", std::process::id()));
        match OpenOptions::new().create_new(true).write(true).open(&temp) {
            Ok(file) => break (temp, file),
            // A reused PID can encounter an older incomplete temp file. It
            // belongs to that writer; reserve a different name, never unlink it.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    let write = file.write_all(data);
    drop(file);
    // These bytes are recomputable. The rename gives readers a complete file
    // but does not claim immediate crash durability.
    let result = write.and_then(|_| fs::rename(&temp, destination));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(result?)
}
