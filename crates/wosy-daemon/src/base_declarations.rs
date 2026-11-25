//! Rooted, lazy canonical base declarations. No semantic Build/Check query.
use super::{file_source, key, read_comments, read_cst, read_node, write_edge, write_node, Edge, Result, SourceStore, INTERN};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
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
use wosy_syntax::{AuthoredSpanMap, ByteSpan, CanonicalCstRoot, SourceIdentity};

const BASE_DECLARATION_QUERIES: u8 = 13;
const INLINE_BYTES: usize = 64 * 1024;
const INLINE: u8 = 0;
const BLOB: u8 = 1;

type QueryKey = [u8; 33];

#[derive(Default)]
pub(super) struct Flights(Mutex<HashMap<QueryKey, Weak<AsyncMutex<()>>>>);

impl Flights {
    fn acquire(&self, key: QueryKey) -> Arc<AsyncMutex<()>> {
        let mut flights = self.0.lock().expect("declaration query registry");
        if let Some(gate) = flights.get(&key).and_then(Weak::upgrade) {
            return gate;
        }
        flights.retain(|_, gate| gate.strong_count() != 0);
        let gate = Arc::new(AsyncMutex::new(()));
        flights.insert(key, Arc::downgrade(&gate));
        gate
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
        let mut diagnostics = self.declarations.diagnostics().to_vec();
        for diagnostic in &mut diagnostics {
            for label in &mut diagnostic.labels {
                label.span.source = self.source.clone();
                label.span.range = self.authored_span(label.span.range);
            }
        }
        diagnostics
    }
}

fn query_key(executable: NodeId) -> [u8; 33] {
    let mut hasher = blake3::Hasher::new_derive_key(BASE_DECLARATION_ABI);
    hasher.update(wosy_syntax::base_cst_schema()).update(&executable.0);
    key(BASE_DECLARATION_QUERIES, NodeId(*hasher.finalize().as_bytes()))
}

fn detached_identity(executable: NodeId) -> SourceIdentity {
    SourceIdentity::new(String::new(), String::new(), String::new(), executable.hex())
}

impl SourceStore {
    /// Resolve only a file inside a currently supported published source
    /// snapshot. Derived queries are optional: snapshot reads never need CAS.
    pub async fn base_declarations(
        &self, root: NodeId, key: &FileKey,
    ) -> Result<Option<BaseDeclarationsView>> {
        let Some(view) = self.snapshot(root)? else { return Ok(None); };
        let Some(file) = view.files.get(key) else { return Ok(None); };
        let executable = file.executable;
        let module = view.modules[key];
        let query = query_key(executable);
        let source = file_source(file)?;
        let spans = AuthoredSpanMap::from_comments(&read_comments(&view.tx, file.annotation)?);
        drop(view);

        // The admitted file stays immutable; start the query transaction after
        // joining its flight so waiters see an already committed result.
        let gate = self.base_flights.acquire(query);
        let _permit = gate.lock().await;
        let read = self.tree.begin_with_mode(Mode::ReadOnly)?;
        let (id, declarations, disposition) = if let Some((id, declarations)) =
            self.read_base_declarations(&read, &query, executable)?
        {
            (id, declarations, QueryDisposition::Reused)
        } else {
            let cst = CanonicalCstRoot::new(detached_identity(executable), read_cst(&read, executable)?);
            let declarations = derive_scalar_declarations_from_cst(&cst);
            drop(cst);
            drop(read);
            let bytes = serde_json::to_vec(&declarations)?;
            let payload = self.pack_payload(bytes)?;
            let node = Node { kind: Kind::BaseDeclarations, data: payload, children: vec![executable] };
            let encoded = node.encode();
            let id = Node::id(&encoded);
            loop {
                let mut tx = self.tree.begin()?;
                tx.set_durability(Durability::Eventual);
                if let Some(existing) = tx.get(&query)? {
                    // A prior query may point at a lost CAS blob. The identical
                    // interned record now resolves again after pack_payload.
                    if existing.as_slice() != id.0 {
                        return Err("base declaration query changed for an immutable input".into());
                    }
                    break;
                }
                if tx.get(&super::key(INTERN, id))?.is_none() {
                    write_node(&mut tx, id, &encoded)?;
                    write_edge(&mut tx, id, Edge::DeclarationSource, executable)?;
                }
                tx.set(&query, &id.0)?;
                match tx.commit().await {
                    Ok(()) => break,
                    Err(surrealkv::Error::TransactionWriteConflict) => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            (id, declarations, QueryDisposition::Derived)
        };
        Ok(Some(BaseDeclarationsView { id, executable, module, source, disposition, declarations, spans }))
    }

    fn read_base_declarations(
        &self, tx: &Transaction, query: &[u8; 33], executable: NodeId,
    ) -> Result<Option<(NodeId, ScalarSourceDeclarations)>> {
        let Some(id) = tx.get(query)? else { return Ok(None); };
        let id = NodeId(id.as_slice().try_into()?);
        let node = read_node(tx, id)?;
        if node.kind != Kind::BaseDeclarations || node.children != [executable] {
            return Err("invalid canonical declaration query result".into());
        }
        let Some((tag, body)) = node.data.split_first() else {
            return Err("missing canonical declaration payload".into());
        };
        let blob;
        let data = match *tag {
            INLINE => body,
            BLOB => {
                let digest = blake3::Hash::from_bytes(body.try_into()?);
                match fs::read(self.blobs.join(digest.to_hex().as_str())) {
                    Ok(bytes) if blake3::hash(&bytes) == digest => blob = bytes,
                    Ok(_) => return Ok(None),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error.into()),
                }
                blob.as_slice()
            }
            _ => return Err("invalid canonical declaration payload kind".into()),
        };
        let mut parser = serde_json::Deserializer::from_slice(data);
        parser.disable_recursion_limit();
        use serde::Deserialize;
        let declarations = ScalarSourceDeclarations::deserialize(&mut parser)?;
        parser.end()?;
        Ok(Some((id, declarations)))
    }

    fn pack_payload(&self, bytes: Vec<u8>) -> Result<Vec<u8>> {
        if bytes.len() <= INLINE_BYTES {
            let mut payload = Vec::with_capacity(1 + bytes.len());
            payload.push(INLINE);
            payload.extend(bytes);
            return Ok(payload);
        }
        let digest = blake3::hash(&bytes);
        fs::create_dir_all(&self.blobs)?;
        let destination = self.blobs.join(digest.to_hex().as_str());
        write_blob(&self.blobs, &destination, &bytes)?;
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
