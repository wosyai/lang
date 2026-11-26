//! Direct reader retention and read-only census of published source roots.
use super::{
    authority_key, base_declarations, build_receipt, build_session::BuildRequestRejection, check_source_marker,
    edge, key, pins, project_declarations, published_source, read_node,
    source_reconciliation::checked_published_source_header,
    source_top, Edge, Result, SourceStore, EDGES_OUT, FILE_AUTHORITY, SNAPSHOT_ROOTS,
};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use surrealkv::{LSMIterator, Mode, Transaction};
use wosy_protocol::{BuildSourceSelection, FileKey, Kind, NodeId};

const PROTECTED: u8 = 1;
const CANDIDATE: u8 = 2;
const SHARED: u8 = PROTECTED | CANDIDATE;
const MAX_BUILD_SESSIONS: usize = 64;
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
}

impl SourceLease {
    pub fn grant(&self) -> LeaseGrant { self.grant }

    pub fn ordinal(&self) -> u64 { self.grant.id }

    pub fn release(&self) {
        let mut reads = self.reads.lock().expect("source reader registry");
        reads.sessions.remove(&self.grant.id);
    }

    pub fn deadline(&self) -> Option<Instant> {
        let reads = self.reads.lock().expect("source reader registry");
        reads.sessions.get(&self.grant.id).map(|(_, expires)| *expires)
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
        *expires = next;
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

fn walk(
    tx: &Transaction, roots: impl IntoIterator<Item = NodeId>, image: Option<NodeId>, class: u8,
    limit: usize, marks: &mut Marks,
) -> Result<()> {
    let mut todo: Vec<_> = roots.into_iter().collect();
    while let Some(id) = todo.pop() {
        let at_limit = marks.nodes.len() >= limit;
        let entry = marks.nodes.entry(id);
        match &entry {
            Entry::Occupied(existing) if existing.get().classes & class != 0 => continue,
            Entry::Vacant(_) if at_limit => return Err("source retention plan exceeded node budget".into()),
            _ => {}
        }
        let node = read_node(tx, id)?;
        entry.or_insert_with(|| Mark {
            classes: 0, encoded_bytes: node.encoded_len(), is_projection: node.kind == Kind::ModuleDeclarations,
        }).classes |= class;

        match node.kind {
            // Other source schemas may have different auxiliary edges.
            Kind::SourcePreparation => {
                let parts = wosy_protocol::source_preparation_parts(&node)
                    .ok_or("unsupported published root in retention census")?;
                if let Some(image) = image {
                    let inputs = build_receipt::BuildInputs {
                        config: parts.config, executable: parts.executable, dependencies: parts.dependencies,
                    };
                    let scope = marks.receipt_scopes.entry(inputs.scope(image))
                        .or_insert(ReceiptScope { inputs, classes: 0 });
                    scope.classes |= class;
                }
            }
            Kind::File => {
                let authority = tx.get(&authority_key(FILE_AUTHORITY, id))?
                    .ok_or("missing namespace authority for source retention")?;
                todo.push(decode_id(authority.as_ref())?);
                if let Some(image) = image {
                    if let Some(body) = tx.get(&base_declarations::query_key(image, node.children[1]))? {
                        todo.push(decode_id(body.as_ref())?);
                    }
                }
            }
            Kind::Module => {
                if let Some(image) = image {
                    if let Some(context) = tx.get(&project_declarations::projection_key(image, id))? {
                        todo.push(decode_id(context.as_ref())?);
                    }
                }
            }
            Kind::BaseDeclarations => {
                if let Some(blob) = base_declarations::cas_digest(&node.data)? {
                    *marks.cas.entry(blob).or_default() |= class;
                }
            }
            _ => {}
        }
        todo.extend(node.children);
    }
    Ok(())
}

fn current_entry(
    tx: &Transaction, image: NodeId, inputs: build_receipt::BuildInputs, projection: NodeId,
    config_entry: &mut Option<FileKey>,
) -> Result<bool> {
    let declarations = read_node(tx, projection)?;
    let Some(&module) = declarations.children.first() else { return Ok(false); };
    // The exact module must occur in this dependency root, not just somewhere
    // in the same protection class. Both links are existing indexed lookups.
    if tx.get(&edge(EDGES_OUT, inputs.dependencies, Edge::ProjectionEntry, module))?.is_none()
        || !tx.get(&project_declarations::projection_key(image, module))?
            .is_some_and(|current| current.as_slice() == projection.0)
    {
        return Ok(false);
    }
    #[derive(serde::Deserialize)]
    struct ConfigEntry { entry: FileKey }
    let entry = match config_entry {
        Some(entry) => entry,
        None => {
            let config = read_node(tx, inputs.config)?;
            config_entry.insert(serde_json::from_slice::<ConfigEntry>(&config.data)?.entry)
        }
    };
    let module = read_node(tx, module)?;
    let Some(&local) = module.children.first() else { return Ok(false); };
    let local = read_node(tx, local)?;
    Ok(serde_json::from_slice::<FileKey>(&local.data)? == *entry)
}

fn mark_receipts(tx: &Transaction, image: NodeId, limit: usize, marks: &mut Marks) -> Result<()> {
    let mut remaining = limit;
    for (&scope, entry) in &marks.receipt_scopes {
        let mut resolved = None;
        let mut config_entry = None;
        build_receipt::attached(tx, scope, entry.inputs, &mut remaining, |id, projection, encoded_bytes| {
            let Some(prior) = marks.nodes.get(&projection).filter(|mark| mark.is_projection) else {
                return Ok(());
            };
            let classes = prior.classes & entry.classes;
            if classes == 0 { return Ok(()); }
            // Only the exact entry projection can anchor this scope's
            // receipt. Even an admitted sibling cannot extend its authority.
            match resolved {
                Some(expected) if expected != projection => return Ok(()),
                Some(_) => {}
                None if current_entry(tx, image, entry.inputs, projection, &mut config_entry)? => {
                    resolved = Some(projection);
                }
                None => return Ok(()),
            }
            let at_limit = marks.nodes.len() >= limit;
            match marks.nodes.entry(id) {
                Entry::Occupied(mut mark) => mark.get_mut().classes |= classes,
                Entry::Vacant(_) if at_limit => return Err("source retention plan exceeded node budget".into()),
                Entry::Vacant(mark) => { mark.insert(Mark { classes, encoded_bytes, is_projection: false }); }
            }
            Ok(())
        })?;
    }
    Ok(())
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
        if lifetime.is_zero() { return Err("a Build diagnostic lease must have a positive lifetime".into()); }
        if start_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(BuildRequestRejection::Expired.into());
        }
        let root = selection.root;
        let Some((tx, read)) = self.begin_source_read(root)? else { return Ok(None); };
        let config = checked_published_source_header(&tx, root)?;
        let owner = config.namespaces.get(&config.entry.package)
            .ok_or("Build diagnostic scope is missing its entry namespace")?;
        if owner.git.is_some() || owner.project != selection.project {
            return Err("Build diagnostic selection has a different workspace".into());
        }
        if config.target != selection.target { return Err("Build diagnostic selection has a different target".into()); }
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
            reads.sessions.insert(id, (root, expires));
            SourceLease { reads: Arc::clone(&self.source_reads.0), grant, lifetime }
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
        let tx = self.tree.begin_with_mode(mode)?;
        let Some(marker) = tx.get(&key(SNAPSHOT_ROOTS, root))? else { return Ok(None); };
        check_source_marker(root, &marker)?;
        *reads.active.entry(root).or_default() += 1;
        Ok(Some((tx, SourceRead::new(&self.source_reads.0, root))))
    }

    pub(super) fn begin_source_read(&self, root: NodeId) -> Result<Option<(Transaction, SourceRead)>> {
        Ok(self.begin_source_reads([root])?.map(|(tx, [read])| (tx, read)))
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
        let scope_top = published_source(&tx, scope)?.ok_or("unknown source retention scope")?;
        let owner = pins::workspace_of_config(&tx, scope_top.children[0])?;
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
        if !keep.is_subset(&published) || !nominated.is_subset(&published) {
            return Err("retention selection names an unpublished root".into());
        }

        let mut pinned = BTreeSet::new();
        let mut pin_count = 0;
        rows(&tx, pins::PINS, |pin_key, value| {
            if pin_count >= limit { return Err("source retention plan exceeded pin budget".into()); }
            pin_count += 1;
            let (root, _) = pins::decode_pin_value(value)?;
            if !published.contains(&root) { return Err("durable pin is outside published roots".into()); }
            pinned.insert(root);
            framed_hash(&mut epoch, pin_key);
            framed_hash(&mut epoch, value);
            Ok(())
        })?;

        let mut candidates = BTreeSet::new();
        for &root in &nominated {
            let top = source_top(&tx, root)?.ok_or("unsupported retention candidate")?;
            if pins::workspace_of_config(&tx, top.children[0])? != owner {
                return Err("retention candidate belongs to another workspace".into());
            }
            if !keep.contains(&root) && !pinned.contains(&root) && !active.contains(&root) {
                candidates.insert(root);
            }
        }

        let mut marks = Marks::default();
        walk(&tx, published.difference(&candidates).copied(), image, PROTECTED, limit, &mut marks)?;
        walk(&tx, candidates.iter().copied(), image, CANDIDATE, limit, &mut marks)?;
        if let Some(image) = image { mark_receipts(&tx, image, limit, &mut marks)?; }

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
            let prepared = store.prepare_configured(in_flight.configured().unwrap(), "dev", None).await.unwrap();
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
