//! Direct reader retention and read-only census of published source roots.
use super::{
    authority_key_for_encoding, base_declarations, build_receipt, build_session::BuildRequestRejection, check_source_marker,
    key, module_parts, pins, prepared_project::SelectedBuildProfile, project_declarations, projection_file_parts,
    read_node_with, read_source_config_charged, read_verified_node_with, validate_shape, Expected,
    source_reconciliation::checked_published_source_header_with_top,
    Result, SourceStore, FILE_AUTHORITY, SNAPSHOT_ROOTS,
};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use surrealkv::{LSMIterator, Mode, Transaction};
use wosy_protocol::{retained_source_preparation_metadata, syntax_kind, BuildSourceSelection, CstKindEncoding,
    FileKey, Kind, Node, NodeId, SourceConfig, SourceEnvelopeIdentity};

const PROTECTED: u8 = 1;
const CANDIDATE: u8 = 2;
const SHARED: u8 = PROTECTED | CANDIDATE;
const MAX_BUILD_SESSIONS: usize = 64;
const MAX_CENSUS_ENCODED_BYTES: usize = 64 << 20;
const MAX_LIVE_BASE_PREPARATIONS: usize = 8;
pub(super) const MAX_RETAINED_DECLARATION_BYTES: usize = 16 << 20;

/// Direct reads and expiring client sessions share the existing read gate.
/// A started query holds a direct read independently of the session owner.
#[derive(Default)]
pub(super) struct SourceReads(Arc<Mutex<ReadRegistry>>);

#[derive(Default)]
struct ReadRegistry {
    active: BTreeMap<NodeId, usize>,
    sessions: BTreeMap<u64, (NodeId, Instant)>,
    next_session: u64,
    prepared: usize,
    prepared_bytes: usize,
}

/// Count held owners independently of session expiry so pruning expired
/// leases cannot unbound caller-owned passive data.
pub(super) struct PreparedAllocation {
    reads: Arc<Mutex<ReadRegistry>>,
    bytes: usize,
}

impl SourceReads {
    pub(super) fn reserve_preparation(&self) -> Result<PreparedAllocation> {
        let mut reads = self.0.lock().expect("source reader registry");
        if reads.prepared == MAX_LIVE_BASE_PREPARATIONS {
            return Err("live base declaration preparations are at capacity".into());
        }
        reads.prepared += 1;
        Ok(PreparedAllocation { reads: Arc::clone(&self.0), bytes: 0 })
    }
}

impl PreparedAllocation {
    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
        let mut reads = self.reads.lock().expect("source reader registry");
        reads.prepared_bytes = (reads.prepared_bytes - self.bytes).checked_add(bytes)
            .filter(|&total| total <= MAX_RETAINED_DECLARATION_BYTES)
            .ok_or("live base declaration preparations exceed retained declaration bytes")?;
        self.bytes = bytes;
        Ok(())
    }
}

impl Drop for PreparedAllocation {
    fn drop(&mut self) {
        let mut reads = self.reads.lock().expect("source reader registry");
        reads.prepared -= 1;
        reads.prepared_bytes -= self.bytes;
    }
}

#[derive(Clone, Copy)]
pub(super) struct LeaseGrant {
    id: u64,
    pub root: NodeId,
}

pub(super) struct SourceLease {
    reads: Arc<Mutex<ReadRegistry>>,
    grant: LeaseGrant,
    lifetime: Duration,
    hard_deadline: Option<Instant>,
}

impl SourceLease {
    pub fn grant(&self) -> LeaseGrant { self.grant }

    pub fn ordinal(&self) -> u64 { self.grant.id }

    pub fn hard_deadline(&self) -> Option<Instant> { self.hard_deadline }

    pub fn release(&self) {
        let mut reads = self.reads.lock().expect("source reader registry");
        reads.sessions.remove(&self.grant.id);
    }

    pub fn deadline(&self) -> Option<Instant> {
        let reads = self.reads.lock().expect("source reader registry");
        reads.sessions.get(&self.grant.id).map(|(_, expires)| *expires)
    }

    /// Linearize a short, already staged update against this exact live
    /// session. The caller must acquire other locks before entering.
    pub(super) fn finish_current<T>(&self, apply: impl FnOnce() -> T) -> Option<T> {
        let reads = self.reads.lock().expect("source reader registry");
        let &(root, expires) = reads.sessions.get(&self.grant.id)?;
        if root != self.grant.root || Instant::now() >= expires { return None; }
        let result = apply();
        drop(reads);
        Some(result)
    }

    pub fn renew(&self) -> bool {
        let mut reads = self.reads.lock().expect("source reader registry");
        let now = Instant::now();
        let Some((_, expires)) = reads.sessions.get_mut(&self.grant.id) else { return false; };
        if now >= *expires {
            reads.sessions.remove(&self.grant.id);
            return false;
        }
        let Some(next) = now.checked_add(self.lifetime) else { return false; };
        *expires = self.hard_deadline.map_or(next, |cutoff| cutoff.min(next));
        true
    }
}

impl Drop for SourceLease {
    fn drop(&mut self) { self.release(); }
}

#[derive(Clone)]
pub(super) struct SourceRead { _guard: Arc<SourceReadGuard> }

struct SourceReadGuard {
    reads: Arc<Mutex<ReadRegistry>>,
    root: NodeId,
}

impl SourceRead {
    fn new(reads: &Arc<Mutex<ReadRegistry>>, root: NodeId) -> Self {
        Self { _guard: Arc::new(SourceReadGuard { reads: Arc::clone(reads), root }) }
    }
}

impl Drop for SourceReadGuard {
    fn drop(&mut self) {
        let mut reads = self.reads.lock().expect("source reader registry");
        let count = reads.active.get_mut(&self.root).expect("admitted source reader");
        *count -= 1;
        if *count == 0 { reads.active.remove(&self.root); }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SourceRetentionPlan {
    /// Exact logical protection inputs observed in a single MVCC transaction.
    /// This is not an authorization or token for a subsequent sweep.
    pub protection_epoch: NodeId,
    pub nominated_roots: usize,
    pub protected_roots: usize,
    /// Distinct roots held by live sessions or in-flight reads.
    pub active_read_roots: usize,
    pub candidate_roots: usize,
    pub nodes_visited: usize,
    pub shared_nodes: usize,
    pub candidate_only_nodes: usize,
    /// Encoded node values only, not disk bytes or a compaction prediction.
    pub candidate_only_node_bytes: usize,
    /// Optional payload identities referenced only by candidate closures.
    /// Actual files and sizes are not part of this MVCC census.
    pub candidate_only_cas_references: usize,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct SourceCasInventory {
    pub referenced: usize,
    /// Files absent at inspection time. Availability is not a checksum or
    /// durability check; another query can restore a recomputable file.
    pub missing: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SourceRetentionInventory {
    pub plan: SourceRetentionPlan,
    /// Includes CAS references shared with nominated roots.
    pub protected: SourceCasInventory,
    /// References that no protected root currently uses.
    pub candidate_only: SourceCasInventory,
}

struct Mark {
    classes: u8,
    graph_classes: [u8; 2],
    encoded_bytes: usize,
    is_projection: bool,
}

#[derive(Default)]
struct Marks {
    nodes: HashMap<NodeId, Mark>,
    cas: HashMap<NodeId, u8>,
    receipt_scopes: BTreeMap<NodeId, ReceiptScope>,
}

struct ReceiptScope {
    inputs: build_receipt::BuildInputs,
    classes: u8,
}

/// One MVCC view meters encoded bytes before all Wosy graph/config decodes.
/// The engine's raw fetch allocation and total RSS are not part of this cap.
struct CensusRead<'a> {
    tx: &'a Transaction,
    remaining_bytes: usize,
    remaining_profiles: usize,
    configs: HashMap<NodeId, Arc<CensusConfig>>,
    admitted_roots: HashMap<NodeId, CensusRoot>,
}

struct CensusConfig {
    encoded_bytes: usize,
    envelope: SourceEnvelopeIdentity,
    entry: FileKey,
    owner: String,
    profiles: Vec<SelectedBuildProfile<String>>,
    other_profile_names: BTreeSet<String>,
}

struct CensusRoot {
    encoded_bytes: usize,
    encoding: CstKindEncoding,
    inputs: build_receipt::BuildInputs,
    children: Vec<NodeId>,
}

impl<'a> CensusRead<'a> {
    fn new(tx: &'a Transaction, limit: usize) -> Self {
        Self { tx, remaining_bytes: MAX_CENSUS_ENCODED_BYTES, remaining_profiles: limit,
            configs: HashMap::new(), admitted_roots: HashMap::new() }
    }

    fn record(&mut self, id: NodeId) -> Result<Node> {
        read_node_with(self.tx, id, |bytes| charge_bytes(&mut self.remaining_bytes, bytes))
    }

    fn verified(&mut self, id: NodeId) -> Result<Node> {
        read_verified_node_with(self.tx, id, |bytes| charge_bytes(&mut self.remaining_bytes, bytes))
    }

    fn config(&mut self, id: NodeId) -> Result<Arc<CensusConfig>> {
        match self.configs.entry(id) {
            Entry::Occupied(cached) => Ok(Arc::clone(cached.get())),
            Entry::Vacant(slot) => {
                let mut encoded_bytes = 0;
                let config = read_source_config_charged(self.tx, id, |length| {
                    charge_bytes(&mut self.remaining_bytes, length)?;
                    encoded_bytes = length;
                    Ok(())
                })?;
                self.remaining_profiles = self.remaining_profiles.checked_sub(config.profiles.len())
                    .ok_or("source retention plan exceeded configured profile budget")?;
                let owner = pins::workspace_of_source_config(&config)?.to_owned();
                let envelope = config.envelope_identity();
                let mut profiles = Vec::new();
                let mut other_profile_names = BTreeSet::new();
                for (name, profile) in config.profiles {
                    match SelectedBuildProfile::try_configured(name, &profile) {
                        Ok(selected) => profiles.push(selected),
                        Err((name, _)) => { other_profile_names.insert(name); }
                    }
                }
                let candidate = Arc::new(CensusConfig {
                    encoded_bytes, envelope, owner, entry: config.entry, profiles, other_profile_names,
                });
                slot.insert(Arc::clone(&candidate));
                Ok(candidate)
            }
        }
    }

    fn load_root(&mut self, id: NodeId) -> Result<CensusRoot> {
        let node = self.verified(id)?;
        let parts = retained_source_preparation_metadata(&node)
            .ok_or("unsupported published root in retention census")?;
        if parts.envelope_identity() != self.config(parts.config)?.envelope {
            return Err("published root and configuration parser differ in retention census".into());
        }
        let inputs = build_receipt::BuildInputs {
            config: parts.config, executable: parts.executable, dependencies: parts.dependencies,
        };
        let encoded_bytes = node.encoded_len();
        let encoding = parts.kind_encoding();
        Ok(CensusRoot { encoded_bytes, encoding, inputs, children: node.children })
    }

    fn root_owner(&mut self, id: NodeId) -> Result<Arc<CensusConfig>> {
        let config = if let Some(root) = self.admitted_roots.get(&id) {
            root.inputs.config
        } else {
            let root = self.load_root(id)?;
            let config = root.inputs.config;
            self.admitted_roots.insert(id, root);
            config
        };
        self.config(config)
    }

    fn root_for_walk(&mut self, id: NodeId) -> Result<CensusRoot> {
        match self.admitted_roots.remove(&id) { Some(root) => Ok(root), None => self.load_root(id) }
    }
}

fn charge_bytes(remaining: &mut usize, bytes: usize) -> Result<()> {
    *remaining = remaining.checked_sub(bytes).ok_or("source retention plan exceeded encoded node read budget")?;
    Ok(())
}

#[derive(Clone, Copy)]
enum WalkOrigin {
    Published,
    Config,
    Graph(CstKindEncoding),
}

fn walk(
    reader: &mut CensusRead<'_>, roots: impl IntoIterator<Item = (NodeId, WalkOrigin)>,
    image: Option<NodeId>, class: u8, limit: usize, marks: &mut Marks,
) -> Result<()> {
    let mut todo: Vec<_> = roots.into_iter().collect();
    while let Some((id, origin)) = todo.pop() {
        let at_limit = marks.nodes.len() >= limit;
        let entry = marks.nodes.entry(id);
        let epoch = match origin {
            WalkOrigin::Graph(CstKindEncoding::Current) => Some(0),
            WalkOrigin::Graph(CstKindEncoding::FrozenBaseV3) => Some(1),
            _ => None,
        };
        if matches!(&entry, Entry::Occupied(existing)
            if epoch.map_or(existing.get().classes, |index| existing.get().graph_classes[index]) & class != 0)
        { continue; }
        let (encoded_bytes, is_projection) = match origin {
            WalkOrigin::Config => (reader.config(id)?.encoded_bytes, false),
            WalkOrigin::Published => {
                let root = reader.root_for_walk(id)?;
                if let Some(image) = image.filter(|_| root.encoding == CstKindEncoding::Current) {
                    let scope = marks.receipt_scopes.entry(root.inputs.scope(image))
                        .or_insert(ReceiptScope { inputs: root.inputs, classes: 0 });
                    scope.classes |= class;
                }
                let encoding = root.encoding;
                todo.extend(root.children.into_iter().skip(1).map(|child| (child, WalkOrigin::Graph(encoding))));
                todo.push((root.inputs.config, WalkOrigin::Config));
                (root.encoded_bytes, false)
            }
            WalkOrigin::Graph(encoding) => {
                let archive = encoding == CstKindEncoding::FrozenBaseV3;
                let image = image.filter(|_| !archive);
                let node = if archive { reader.verified(id)? } else { reader.record(id)? };
                match node.kind {
                    // A generic link can never mark or follow a receipt or
                    // confer the more specific authority of a root or config.
                    Kind::BuildDiagnosticReceipt | Kind::SavedBuildDiagnostic | Kind::SourcePreparation | Kind::Config => continue,
                    Kind::File => {
                        let authority = reader.tx.get(&authority_key_for_encoding(FILE_AUTHORITY, id, encoding))?
                            .ok_or("missing namespace authority for source retention")?;
                        todo.push((decode_id(authority.as_ref())?, origin));
                        let [_, executable, _] = node.children.as_slice() else { return Err("invalid retained file shape".into()); };
                        if let Some(image) = image {
                            if let Some(body) = reader.tx.get(&base_declarations::query_key(image, *executable))? {
                                todo.push((decode_id(body.as_ref())?, origin));
                            }
                        }
                    }
                    Kind::Module => {
                        if let Some(image) = image {
                            if let Some(context) = reader.tx.get(&project_declarations::projection_key(image, id))? {
                                todo.push((decode_id(context.as_ref())?, origin));
                            }
                        }
                    }
                    Kind::BaseDeclarations => {
                        if archive { return Err("compiler body outside the historical source closure".into()); }
                        if let Some(blob) = base_declarations::cas_digest(&node.data)? {
                            *marks.cas.entry(blob).or_default() |= class;
                        }
                    }
                    Kind::ModuleDeclarations if archive => return Err("compiler projection outside the historical source closure".into()),
                    Kind::Syntax | Kind::Token if archive && !encoding.allows(syntax_kind(&node)?) =>
                        return Err("historical source syntax kind is outside its parser epoch".into()),
                    _ => {}
                }
                let encoded_bytes = node.encoded_len();
                todo.extend(node.children.into_iter().map(|child| (child, origin)));
                (encoded_bytes, node.kind == Kind::ModuleDeclarations)
            }
        };
        if matches!(&entry, Entry::Vacant(_) if at_limit) {
            return Err("source retention plan exceeded node budget".into());
        }
        let mark = entry.or_insert_with(|| Mark { classes: 0, graph_classes: [0; 2], encoded_bytes, is_projection });
        mark.classes |= class;
        if let Some(epoch) = epoch { mark.graph_classes[epoch] |= class; }
    }
    Ok(())
}

struct ReceiptAuthority {
    config: Arc<CensusConfig>,
    modules: BTreeSet<NodeId>,
    executable_files: BTreeSet<NodeId>,
}

impl ReceiptAuthority {
    fn read(reader: &mut CensusRead<'_>, inputs: build_receipt::BuildInputs) -> Result<Self> {
        let config = reader.config(inputs.config)?;
        let mut members = |id, kind| -> Result<BTreeSet<NodeId>> {
            let node = reader.verified(id)?;
            Expected::Exact(kind).check(node.kind)?;
            validate_shape(&node)?;
            Ok(node.children.into_iter().collect())
        };
        let modules = members(inputs.dependencies, Kind::Dependencies)?;
        let executable_files = members(inputs.executable, Kind::Executables)?;
        Ok(Self { config, modules, executable_files })
    }

    fn current_entry(&self, reader: &mut CensusRead<'_>, image: NodeId, projection: NodeId) -> Result<bool> {
        let Some(parts) = project_declarations::verified_projection_parts(reader.tx, projection,
            |bytes| charge_bytes(&mut reader.remaining_bytes, bytes))? else { return Ok(false); };
        if !self.modules.contains(&parts.source)
            || !reader.tx.get(&project_declarations::projection_key(image, parts.source))?
                .is_some_and(|current| current.as_slice() == projection.0)
        {
            return Ok(false);
        }
        let module = reader.verified(parts.source)?;
        let (local, imports) = module_parts(&module)?;
        if !self.executable_files.contains(&local) || !parts.has_import_arity(imports.len()) { return Ok(false); }
        let local = reader.verified(local)?;
        let (key, _) = projection_file_parts(&local, Kind::ExecutableFile)?;
        Ok(key == self.config.entry)
    }
}

fn mark_receipts(reader: &mut CensusRead<'_>, image: NodeId, limit: usize, marks: &mut Marks) -> Result<()> {
    let mut remaining_rows = limit;
    let mut remaining_profile_keys = limit;
    for (&scope, entry) in &marks.receipt_scopes {
        let mut resolved = None;
        let mut authority = None;
        let mut rejected = BTreeSet::new();
        build_receipt::attached_rows(reader.tx, scope, &mut remaining_rows, |digest, pointer| {
            let Some(receipt) = build_receipt::stored_receipt(reader.tx, pointer, digest, &entry.inputs.ids(),
                |bytes| charge_bytes(&mut reader.remaining_bytes, bytes))? else { return Ok(()); };
            let projection = receipt.projection;
            let Some(prior) = marks.nodes.get(&projection).filter(|mark| mark.is_projection) else {
                return Ok(());
            };
            let classes = prior.classes & entry.classes;
            if classes == 0 { return Ok(()); }
            if let Some((expected, _)) = &resolved {
                if *expected != projection { return Ok(()); }
            } else {
                if rejected.contains(&projection) { return Ok(()); }
                let authority = match authority.as_ref() {
                    Some(authority) => authority,
                    None => authority.insert(ReceiptAuthority::read(reader, entry.inputs)?),
                };
                if !authority.current_entry(reader, image, projection)? {
                    rejected.insert(projection);
                    return Ok(());
                }
                remaining_profile_keys = remaining_profile_keys.checked_sub(authority.config.profiles.len())
                    .ok_or("source retention plan exceeded receipt profile key budget")?;
                let keys = authority.config.profiles.iter().map(|profile|
                    profile.identity(Some(image), entry.inputs, projection).1.digest)
                    .collect::<BTreeSet<_>>();
                resolved = Some((projection, keys));
            }
            let (_, keys) = resolved.as_ref().expect("entry confirmed above");
            if !keys.contains(&receipt.digest) || !receipt.has_payload() { return Ok(()); }
            let at_limit = marks.nodes.len() >= limit;
            match marks.nodes.entry(receipt.id) {
                Entry::Occupied(mut mark) => mark.get_mut().classes |= classes,
                Entry::Vacant(_) if at_limit => return Err("source retention plan exceeded node budget".into()),
                Entry::Vacant(mark) => {
                    mark.insert(Mark { classes, graph_classes: [0; 2], encoded_bytes: receipt.encoded_bytes, is_projection: false });
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn assert_bounded_semantic_census_read(store: &SourceStore, root: NodeId, config: NodeId, receipt: NodeId) {
    let tx = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
    let length = |id| tx.get(&key(super::NODES, id)).unwrap().unwrap().len();
    let config_bytes = length(config);
    let root_bytes = length(root);

    let mut reader = CensusRead::new(&tx, 100);
    reader.remaining_bytes = root_bytes + config_bytes - 1;
    assert!(reader.root_owner(root).err().unwrap().to_string().contains("encoded node read budget"));
    assert!(reader.configs.is_empty());
    assert_eq!(reader.remaining_bytes, config_bytes - 1);

    let mut reader = CensusRead::new(&tx, 100);
    reader.remaining_bytes = root_bytes + config_bytes;
    let first = reader.root_owner(root).unwrap();
    let profiles = reader.remaining_profiles;
    assert_eq!(first.encoded_bytes, config_bytes);
    assert!(Arc::ptr_eq(&first, &reader.config(config).unwrap()));
    assert!(Arc::ptr_eq(&first, &reader.root_owner(root).unwrap()));
    assert_eq!(reader.root_for_walk(root).unwrap().encoded_bytes, root_bytes);
    assert_eq!(reader.remaining_profiles, profiles);
    assert_eq!(reader.remaining_bytes, 0);
    assert!(reader.record(receipt).unwrap_err().to_string().contains("encoded node read budget"));

    // A genuine receipt record is not itself an ordinary graph authority.
    // Exercise the graph walker on this existing immutable node; no store
    // corruption or mutable links are needed for this invariant.
    reader.remaining_bytes = length(receipt);
    let mut marks = Marks::default();
    walk(&mut reader, [(receipt, WalkOrigin::Graph(CstKindEncoding::Current))], None, PROTECTED, 100, &mut marks).unwrap();
    assert_eq!(reader.remaining_bytes, 0);
    assert!(marks.nodes.is_empty());
}

fn decode_id(bytes: &[u8]) -> Result<NodeId> {
    Ok(NodeId(bytes.try_into()?))
}

fn rows(
    tx: &Transaction, tag: u8, mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    let mut iter = tx.range([tag].as_slice(), [tag + 1].as_slice())?;
    iter.seek_first()?;
    while iter.valid() {
        visit(iter.key().user_key(), iter.value()?.as_ref())?;
        iter.next()?;
    }
    Ok(())
}

fn framed_hash(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_be_bytes()).update(bytes);
}

impl SourceStore {
    pub(super) fn lease_build_source(
        &self, selection: BuildSourceSelection, lifetime: Duration,
    ) -> Result<Option<SourceLease>> {
        self.lease_build_source_before(selection, lifetime, None)
    }

    pub(super) fn lease_build_source_before(
        &self, selection: BuildSourceSelection, lifetime: Duration, start_deadline: Option<Instant>,
    ) -> Result<Option<SourceLease>> {
        self.lease_build_source_with_cutoff(selection, lifetime, start_deadline, None)
    }

    pub(super) fn lease_build_source_with_cutoff(
        &self, selection: BuildSourceSelection, lifetime: Duration, start_deadline: Option<Instant>,
        hard_deadline: Option<Instant>,
    ) -> Result<Option<SourceLease>> {
        self.lease_build_source_inspect(selection, lifetime, start_deadline, hard_deadline, |_, _, _| Ok(()))
    }

    pub(super) fn lease_build_source_inspect(
        &self, selection: BuildSourceSelection, lifetime: Duration, start_deadline: Option<Instant>,
        hard_deadline: Option<Instant>, inspect: impl FnOnce(&Transaction, &Node, &SourceConfig) -> Result<()>,
    ) -> Result<Option<SourceLease>> {
        if lifetime.is_zero() { return Err("a Build diagnostic lease must have a positive lifetime".into()); }
        let start_deadline = start_deadline.into_iter().chain(hard_deadline).min();
        if start_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(BuildRequestRejection::Expired.into());
        }
        let root = selection.root;
        let Some((tx, read)) = self.begin_source_read(root)? else { return Ok(None); };
        let (top, config) = checked_published_source_header_with_top(&tx, root)?;
        let owner = config.namespaces.get(&config.entry.package)
            .ok_or("Build diagnostic scope is missing its entry namespace")?;
        if owner.git.is_some() || owner.project != selection.project {
            return Err("Build diagnostic selection has a different workspace".into());
        }
        if config.target != selection.target { return Err("Build diagnostic selection has a different target".into()); }
        inspect(&tx, &top, &config)?;
        drop(tx);
        let lease = {
            let mut reads = self.source_reads.0.lock().expect("source reader registry");
            let now = Instant::now();
            if start_deadline.is_some_and(|deadline| now >= deadline) {
                return Err(BuildRequestRejection::Expired.into());
            }
            let expires = now.checked_add(lifetime).ok_or("Build diagnostic lease expiry is out of range")?;
            reads.sessions.retain(|_, (_, expires)| *expires > now);
            if reads.sessions.len() == MAX_BUILD_SESSIONS { return Err("Build diagnostic session quota exceeded".into()); }
            let id = reads.next_session.checked_add(1).ok_or("Build diagnostic lease IDs exhausted")?;
            reads.next_session = id;
            let grant = LeaseGrant { id, root };
            let expires = hard_deadline.map_or(expires, |cutoff| cutoff.min(expires));
            reads.sessions.insert(id, (root, expires));
            SourceLease { reads: Arc::clone(&self.source_reads.0), grant, lifetime, hard_deadline }
        };
        // The ordinary read prevents a gap between validating the identity
        // and registering the expiring session under the same retention gate.
        drop(read);
        Ok(Some(lease))
    }

    pub(super) fn leased_source_snapshot(&self, grant: LeaseGrant) -> Result<Option<super::SourceView>> {
        let Some((tx, read)) = self.begin_leased_source(grant, Mode::ReadOnly)? else { return Ok(None); };
        self.snapshot_with_read(grant.root, tx, read)
    }

    pub(super) fn begin_leased_source(
        &self, grant: LeaseGrant, mode: Mode,
    ) -> Result<Option<(Transaction, SourceRead)>> {
        let mut reads = self.source_reads.0.lock().expect("source reader registry");
        let Some(&(root, expires)) = reads.sessions.get(&grant.id) else { return Ok(None); };
        if Instant::now() >= expires {
            reads.sessions.remove(&grant.id);
            return Ok(None);
        }
        if root != grant.root { return Ok(None); }
        let mut tx = self.tree.begin_with_mode(mode)?;
        let marker_key = key(SNAPSHOT_ROOTS, root);
        let marker = if matches!(mode, Mode::ReadWrite) { tx.get_for_update(&marker_key)? } else { tx.get(&marker_key)? };
        let Some(marker) = marker else { return Ok(None); };
        check_source_marker(root, &marker)?;
        *reads.active.entry(root).or_default() += 1;
        Ok(Some((tx, SourceRead::new(&self.source_reads.0, root))))
    }

    pub(super) fn begin_source_read(&self, root: NodeId) -> Result<Option<(Transaction, SourceRead)>> {
        Ok(self.begin_source_reads([root])?.map(|(tx, [read])| (tx, read)))
    }

    /// Protect an already published closure while a durable pin update is
    /// in flight. The same marker read also joins optimistic validation.
    pub(super) fn begin_pin_update(&self, root: NodeId) -> Result<(Transaction, Option<SourceRead>)> {
        let mut reads = self.source_reads.0.lock().expect("source reader registry");
        let mut tx = self.tree.begin_with_mode(Mode::ReadWrite)?;
        let read = if let Some(marker) = tx.get_for_update(&key(SNAPSHOT_ROOTS, root))? {
            check_source_marker(root, &marker)?;
            *reads.active.entry(root).or_default() += 1;
            Some(SourceRead::new(&self.source_reads.0, root))
        } else {
            None
        };
        Ok((tx, read))
    }

    pub(super) fn begin_source_reads<const N: usize>(
        &self, roots: [NodeId; N],
    ) -> Result<Option<(Transaction, [SourceRead; N])>> {
        let mut reads = self.source_reads.0.lock().expect("source reader registry");
        let tx = self.tree.begin_with_mode(Mode::ReadOnly)?;
        for root in roots {
            let Some(marker) = tx.get(&key(SNAPSHOT_ROOTS, root))? else { return Ok(None); };
            check_source_marker(root, &marker)?;
        }
        let admitted = roots.map(|root| {
            *reads.active.entry(root).or_default() += 1;
            SourceRead::new(&self.source_reads.0, root)
        });
        Ok(Some((tx, admitted)))
    }

    /// Conservative, direct fixture inspection. All other published roots are
    /// protected by default. The supplied keep list adds fixture read roots;
    /// direct readers and current process-local session leases are protected
    /// automatically. This confers no sweep or cross-process retention right.
    pub fn plan_source_retention(
        &self, scope: NodeId, nominated: &[NodeId], keep: &[NodeId], limit: usize,
    ) -> Result<SourceRetentionPlan> {
        Ok(self.source_census(scope, nominated, keep, limit)?.0)
    }

    /// Check filesystem presence once per optional payload in the same
    /// marked closure. Does not derive, restore, remove, or validate a blob.
    /// The plan is MVCC-consistent; file presence is observed after the cut.
    pub fn inspect_source_retention(
        &self, scope: NodeId, nominated: &[NodeId], keep: &[NodeId], limit: usize,
    ) -> Result<SourceRetentionInventory> {
        let (plan, cas) = self.source_census(scope, nominated, keep, limit)?;
        let mut protected = SourceCasInventory::default();
        let mut candidate_only = SourceCasInventory::default();
        for (digest, classes) in cas {
            let group = if classes & PROTECTED != 0 { &mut protected } else { &mut candidate_only };
            group.referenced += 1;
            match std::fs::metadata(self.blob_path(digest)) {
                Ok(metadata) if metadata.is_file() => {}
                Ok(_) => return Err("source CAS payload path is not a file".into()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => group.missing += 1,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(SourceRetentionInventory { plan, protected, candidate_only })
    }

    fn source_census(
        &self, scope: NodeId, nominated: &[NodeId], keep: &[NodeId], limit: usize,
    ) -> Result<(SourceRetentionPlan, HashMap<NodeId, u8>)> {
        // Lease admission and the census cut share one short gate. Reads and
        // filesystem inspection never hold it during graph traversal.
        let (tx, active) = {
            let mut reads = self.source_reads.0.lock().expect("source reader registry");
            if reads.active.len() > limit { return Err("source retention plan exceeded reader root budget".into()); }
            let tx = self.tree.begin_with_mode(Mode::ReadOnly)?;
            let now = Instant::now();
            reads.sessions.retain(|_, (_, expires)| *expires > now);
            let mut active: BTreeSet<_> = reads.active.keys().copied().collect();
            for &(root, _) in reads.sessions.values() {
                active.insert(root);
                if active.len() > limit { return Err("source retention plan exceeded reader root budget".into()); }
            }
            (tx, active)
        };
        if nominated.len().max(keep.len()) > limit {
            return Err("source retention selection exceeded root budget".into());
        }
        let nominated: BTreeSet<_> = nominated.iter().copied().collect();
        let keep: BTreeSet<_> = keep.iter().copied().chain([scope]).collect();

        let mut epoch = blake3::Hasher::new_derive_key("wosy source protection census v3");
        epoch.update(&scope.0);
        let image = self.derivation_image();
        if let Some(image) = image { framed_hash(&mut epoch, &image.0); }
        else { framed_hash(&mut epoch, &[]); }
        for selection in [&nominated, &keep, &active] {
            epoch.update(&(selection.len() as u64).to_be_bytes());
            for id in selection { epoch.update(&id.0); }
        }

        let mut published = BTreeSet::new();
        rows(&tx, SNAPSHOT_ROOTS, |catalog_key, value| {
            let root = decode_id(&catalog_key[1..])?;
            if value != root.0 { return Err("incomplete published source marker".into()); }
            if published.len() >= limit { return Err("source retention plan exceeded root budget".into()); }
            published.insert(root);
            framed_hash(&mut epoch, catalog_key);
            framed_hash(&mut epoch, value);
            Ok(())
        })?;
        if !published.contains(&scope) { return Err("unknown source retention scope".into()); }
        if !keep.is_subset(&published) || !nominated.is_subset(&published) {
            return Err("retention selection names an unpublished root".into());
        }

        let mut reader = CensusRead::new(&tx, limit);
        let owner = reader.root_owner(scope)?;
        let mut marks = Marks::default();
        let mut saved_roots = BTreeMap::new();
        let mut pinned = BTreeSet::new();
        let mut pin_count = 0;
        rows(&tx, pins::PINS, |pin_key, value| {
            if pin_count >= limit { return Err("source retention plan exceeded pin budget".into()); }
            pin_count += 1;
            let pin = pins::decode_pin_value(value)?;
            if let Some(root) = pin.retained_root() {
                if !published.contains(&root) { return Err("durable pin is outside published roots".into()); }
                pinned.insert(root);
                if let Some(record) = pin.saved_diagnostic() {
                    let config = reader.root_owner(root)?;
                    if pin.owner() != config.owner.as_bytes()
                        || !pins::Owner::new(config.owner.clone()).owns_named_key(pin_key)
                    {
                        return Err("saved diagnostic pin has the wrong workspace".into());
                    }
                    match saved_roots.entry(record) {
                        std::collections::btree_map::Entry::Occupied(entry) if *entry.get() != root =>
                            return Err("saved diagnostic has conflicting authored roots".into()),
                        std::collections::btree_map::Entry::Occupied(_) => (),
                        std::collections::btree_map::Entry::Vacant(attachment) => {
                            if marks.nodes.len() >= limit { return Err("source retention plan exceeded node budget".into()); }
                            let (report, encoded_bytes) = super::saved_diagnostics::read_record(
                                reader.tx, record, root, super::saved_diagnostics::MAX_REPORT_BYTES,
                                |bytes| charge_bytes(&mut reader.remaining_bytes, bytes))?;
                            if !config.other_profile_names.contains(&report.profile)
                                && !config.profiles.iter().any(|profile| profile.name() == report.profile)
                            {
                                return Err("saved diagnostic profile is outside its source".into());
                            }
                            attachment.insert(root);
                            marks.nodes.insert(record, Mark {
                                classes: PROTECTED, graph_classes: [0; 2], encoded_bytes, is_projection: false,
                            });
                        }
                    }
                }
            }
            framed_hash(&mut epoch, pin_key);
            framed_hash(&mut epoch, value);
            Ok(())
        })?;

        let mut candidates = BTreeSet::new();
        for &root in &nominated {
            if reader.root_owner(root)?.owner != owner.owner {
                return Err("retention candidate belongs to another workspace".into());
            }
            if !keep.contains(&root) && !pinned.contains(&root) && !active.contains(&root) {
                candidates.insert(root);
            }
        }

        walk(&mut reader, published.difference(&candidates).map(|&root| (root, WalkOrigin::Published)),
            image, PROTECTED, limit, &mut marks)?;
        walk(&mut reader, candidates.iter().map(|&root| (root, WalkOrigin::Published)),
            image, CANDIDATE, limit, &mut marks)?;
        if let Some(image) = image { mark_receipts(&mut reader, image, limit, &mut marks)?; }

        let mut report = SourceRetentionPlan {
            protection_epoch: NodeId(*epoch.finalize().as_bytes()),
            nominated_roots: nominated.len(),
            protected_roots: published.len() - candidates.len(),
            active_read_roots: active.len(),
            candidate_roots: candidates.len(),
            nodes_visited: marks.nodes.len(),
            shared_nodes: 0,
            candidate_only_nodes: 0,
            candidate_only_node_bytes: 0,
            candidate_only_cas_references: marks.cas.values().filter(|&&class| class == CANDIDATE).count(),
        };
        for mark in marks.nodes.values() {
            match mark.classes {
                SHARED => report.shared_nodes += 1,
                CANDIDATE => {
                    report.candidate_only_nodes += 1;
                    report.candidate_only_node_bytes += mark.encoded_bytes;
                }
                _ => {}
            }
        }
        Ok((report, marks.cas))
    }
}

#[cfg(test)]
mod lease_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn native_lease_renewal_never_extends_the_absolute_cut() {
        let reads = Arc::new(Mutex::new(ReadRegistry::default()));
        let root = NodeId([1; 32]);
        let grant = LeaseGrant { root, id: 1 };
        let now = Instant::now();
        let cutoff = now + Duration::from_secs(10);
        reads.lock().unwrap().sessions.insert(grant.id, (root, cutoff));
        let mut lease = SourceLease { reads, grant, lifetime: Duration::from_secs(30), hard_deadline: Some(cutoff) };
        assert!(lease.renew());
        assert_eq!(lease.deadline(), Some(cutoff));
        // Model the same non-renewable authority just after its cutoff,
        // without a real socket, timer sleep or mutable user store.
        lease.hard_deadline = Some(now);
        lease.reads.lock().unwrap().sessions.insert(grant.id, (root, now));
        assert!(!lease.renew());
        assert_eq!(lease.deadline(), None);
    }

    #[test]
    fn passive_preparation_charges_share_the_live_store_budget() {
        let reads = SourceReads::default();
        let mut first = reads.reserve_preparation().unwrap();
        let mut second = reads.reserve_preparation().unwrap();
        first.charge(MAX_RETAINED_DECLARATION_BYTES).unwrap();
        assert!(second.charge(1).is_err());
        drop(first);
        second.charge(MAX_RETAINED_DECLARATION_BYTES).unwrap();
        drop(second);
        let registry = reads.0.lock().unwrap();
        assert_eq!((registry.prepared, registry.prepared_bytes), (0, 0));
    }

    #[test]
    #[ignore = "requires a disposable store with two acknowledged source roots"]
    fn expired_session_does_not_release_inflight_owner() {
        let path = PathBuf::from(std::env::var_os("WOSY_TEST_IMAGE_STORE").unwrap());
        let old = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").unwrap()).unwrap();
        let current = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_OTHER_ROOT").unwrap()).unwrap();
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let store = SourceStore::open(&path, 8 << 20).unwrap();
            let census = || store.plan_source_retention(current, &[old], &[], 20_000).unwrap();
            assert_eq!(census().candidate_roots, 1);

            let workspace = path.parent().unwrap().join("project").canonicalize().unwrap();
            let selected = BuildSourceSelection { root: old, project: workspace.to_str().unwrap().into(), target: "app".into() };
            let open = || store.lease_build_source(selected.clone(), Duration::from_secs(60));

            for (var, reason) in [
                ("WOSY_TEST_LEASE_OTHER_TARGET", "Build diagnostic selection has a different target"),
                ("WOSY_TEST_LEASE_OTHER_WORKSPACE", "Build diagnostic selection has a different workspace"),
            ] {
                let wrong = NodeId::parse_hex(&std::env::var(var).unwrap()).unwrap();
                let refusal = store.lease_build_source(BuildSourceSelection { root: wrong, ..selected.clone() },
                    Duration::from_secs(60)).err().unwrap();
                assert_eq!(refusal.to_string(), reason);
            }

            let lease = open().unwrap().unwrap();
            assert!(lease.renew());
            assert_eq!((census().candidate_roots, census().active_read_roots), (0, 1));
            let in_flight = store.leased_source_snapshot(lease.grant()).unwrap().unwrap();
            // Set only our fixture lease's clock state; this tests expiry at
            // the real admission boundary without a wall-clock sleep.
            let grant = lease.grant();
            {
                let mut reads = store.source_reads.0.lock().unwrap();
                reads.sessions.get_mut(&grant.id).unwrap().1 = Instant::now();
            }
            assert!(store.leased_source_snapshot(grant).unwrap().is_none());
            assert!(!lease.renew());
            drop(lease);
            assert_eq!((census().candidate_roots, census().active_read_roots), (0, 1));
            let prepared = store.prepare_configured(in_flight.configured().unwrap(), "dev", None, None).await.unwrap();
            let response = prepared.build_analysis().await.unwrap();
            assert_eq!(response.source_root, old);
            assert_eq!(census().candidate_roots, 0);
            drop(prepared);
            assert_eq!((census().candidate_roots, census().active_read_roots), (1, 0));

            let mut slots = (0..MAX_BUILD_SESSIONS).map(|_| open().unwrap().unwrap()).collect::<Vec<_>>();
            let quota = || assert_eq!(open().err().unwrap().to_string(), "Build diagnostic session quota exceeded");
            assert_eq!(census().active_read_roots, 1);
            quota();
            drop(slots.pop().unwrap());
            slots.push(open().unwrap().unwrap());
            quota();
            {
                let mut reads = store.source_reads.0.lock().unwrap();
                reads.sessions.get_mut(&slots[0].grant().id).unwrap().1 = Instant::now();
            }
            let replacement = open().unwrap().unwrap();
            assert!(!slots[0].renew());
            quota();
            drop(replacement);
            drop(slots);
            assert_eq!(census().active_read_roots, 0);
            store.close().await.unwrap();
        });
    }
}
