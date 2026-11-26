//! Validate one trusted-adapter-bound base CST proposal without publishing it.
use super::{
    admit_checked_namespace_authority, child_edge, file_source, key, read_index, validate_file,
    validate_shape, validate_snapshot_with, write_edge, write_node,
    project_declarations::UnpublishedDeclarations,
    retention_plan::{PreparedAllocation, SourceLease, SourceRead, MAX_RETAINED_DECLARATION_BYTES},
    CheckedFile, Expected, FileTree, Index, PreparedBaseModule, PublishReport, Result,
    SourceStorageWorker, SourceStore, INTERN, NODES,
};
use rowan::{NodeOrToken, WalkEvent};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use surrealkv::{Mode, Transaction};
use wosy_protocol::{
    imports_from_cst, partition_cst_files, BaseCstProposal, BaseCstProposalBinding, BuildSourceSelection,
    CstExpansionBudget, Delta, FileEntry, FileKey, Kind, Node, NodeId, SourceConfig, BASE_CST_DELTA_VERSION,
};
use wosy_syntax::CstNode;

/// Logical bounds for the selected closure plus the proposal, and for actual
/// unfolded occurrences and text inspected in changed files. Not an RSS or
/// wire decoder bound. A framed local adapter must also cap frame payloads.
#[derive(Clone, Copy)]
pub struct BaseCstAdmissionLimits {
    pub records: usize,
    pub record_bytes: usize,
    pub graph_edges: usize,
    pub expanded_record_bytes: usize,
    pub cst_occurrences: usize,
    pub cst_depth: usize,
    pub authored_bytes: usize,
}

pub struct BaseCstAdmission {
    pub base: NodeId,
    pub proposed: NodeId,
    pub attempt: i64,
    pub base_records: usize,
    pub novel_records: usize,
    pub validated_files: usize,
}

/// Scoped identity of this local prepared owner; not a source or published
/// snapshot ID, and not a capability that can be reconstructed from bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedBaseIdentity(NodeId);

impl PreparedBaseIdentity {
    pub fn digest(self) -> NodeId { self.0 }
}

/// An immutable, ordinary base declaration projection of a proposal. Holding
/// it retains its actual predecessor, but does not occupy a worker permit.
/// It is not a protocol Prepared response, fragment IR, or a typed program.
pub struct PreparedBaseDeclarations {
    identity: PreparedBaseIdentity,
    admission: BaseCstAdmission,
    declarations: UnpublishedDeclarations,
    lease: Arc<SourceLease>,
    _read: SourceRead,
    _allocation: PreparedAllocation,
    // A held owner belongs to the same store and quota even after its
    // worker or request is gone; the store cannot be consumed for close.
    _store: Arc<SourceStore>,
}

impl PreparedBaseDeclarations {
    pub fn identity(&self) -> PreparedBaseIdentity { self.identity }
    pub fn admission(&self) -> &BaseCstAdmission { &self.admission }
    pub fn entry_projection(&self) -> NodeId { self.declarations.entry_projection }
    pub fn modules(&self) -> &[PreparedBaseModule] { &self.declarations.modules }
    pub fn unique_bodies(&self) -> usize { self.declarations.unique_bodies }
    /// Canonical serialized bodies once per executable, not total RSS.
    pub fn declaration_bytes(&self) -> usize { self.declarations.declaration_bytes }

    pub fn lease_is_live(&self) -> bool {
        self.lease.deadline().is_some_and(|deadline| Instant::now() < deadline)
    }
}

struct ValidatedProposal {
    admission: BaseCstAdmission,
    entry: FileKey,
    index: Index,
    work: CstWork,
}

fn prepared_identity(
    scope: &BaseCstAdmission, entry: NodeId, image: Option<NodeId>,
) -> PreparedBaseIdentity {
    let mut hash = blake3::Hasher::new_derive_key("wosy local admitted base declaration owner v1");
    hash.update(wosy_compiler::BASE_DECLARATION_ABI.as_bytes())
        .update(wosy_syntax::base_cst_schema())
        .update(&scope.base.0).update(&scope.proposed.0).update(&scope.attempt.to_be_bytes())
        .update(&entry.0);
    if let Some(image) = image { hash.update(&image.0); }
    PreparedBaseIdentity(NodeId(*hash.finalize().as_bytes()))
}

pub(super) enum AdmissionLease {
    Open(Duration),
    Connection(Arc<SourceLease>),
}

impl SourceStorageWorker {
    /// Check only the authenticated local adapter's selected/pending pair.
    /// The proposal cannot open a lease or pick an unrelated base. Nothing
    /// returned here is a lease, Prepared handle, IR, or publication ack.
    pub async fn validate_base_cst_proposal(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, lifetime: Duration,
    ) -> Result<BaseCstAdmission> {
        let (admission, _, _) = self.with_cst_admission(binding, proposal, limits, AdmissionLease::Open(lifetime), |_, _, valid| {
            Ok(valid.admission)
        }).await?;
        Ok(admission)
    }

    /// Materialize only the real reachable base declarations in the very
    /// transaction that admitted this proposal. Never write query/CAS data.
    pub async fn prepare_base_cst_declarations(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, declaration_bytes: usize, lifetime: Duration,
    ) -> Result<PreparedBaseDeclarations> {
        self.prepare_base_cst_with_lease(binding, proposal, limits, declaration_bytes, AdmissionLease::Open(lifetime)).await
    }

    pub(super) async fn prepare_base_cst_with_lease(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, declaration_bytes: usize, lease: AdmissionLease,
    ) -> Result<PreparedBaseDeclarations> {
        let (owned, lease, read) = self.with_cst_admission(binding, proposal, limits, lease,
            move |store, tx, valid| {
                let ValidatedProposal { admission, entry, index, mut work } = valid;
                let mut allocation = store.source_reads.reserve_preparation()?;
                let declarations = store.prepare_unpublished_declarations(tx, &entry, index, &mut work,
                    declaration_bytes.min(MAX_RETAINED_DECLARATION_BYTES))?;
                allocation.charge(declarations.declaration_bytes)?;
                let identity = prepared_identity(&admission, declarations.entry_projection, store.derivation_image());
                Ok((identity, admission, declarations, allocation))
            },
        ).await?;
        let (identity, admission, declarations, allocation) = owned;
        Ok(PreparedBaseDeclarations {
            identity, admission, declarations, lease, _read: read, _allocation: allocation,
            _store: self.retained_store(),
        })
    }

    async fn with_cst_admission<T: Send + 'static>(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, admission_lease: AdmissionLease,
        use_validated: impl FnOnce(&SourceStore, &Transaction, ValidatedProposal) -> Result<T> + Send + 'static,
    ) -> Result<(T, Arc<SourceLease>, SourceRead)> {
        if proposal.version != BASE_CST_DELTA_VERSION {
            return Err("unsupported base CST proposal version".into());
        }
        if binding.attempt <= 0 || proposal.attempt != binding.attempt
            || proposal.base != binding.selected.root || proposal.root != binding.proposed
            || proposal.root == proposal.base
        {
            return Err("base CST proposal does not match the bound source attempt".into());
        }
        let slot = self.try_slot()?;
        self.spawn_reserved(slot, move |store| {
            let lease = match admission_lease {
                AdmissionLease::Open(lifetime) => Arc::new(store.lease_build_source(binding.selected.clone(), lifetime)?
                    .ok_or("bound base CST predecessor is not published")?),
                AdmissionLease::Connection(lease) if lease.grant().root == binding.selected.root => lease,
                AdmissionLease::Connection(_) => return Err("base CST scope does not match the connection lease".into()),
            };
            let (mut tx, read) = store.begin_leased_source(lease.grant(), Mode::ReadWrite)?
                .ok_or("base CST predecessor lease expired")?;
            let validated = validate(&mut tx, &binding, &proposal, limits)?;
            let accepted = use_validated(store, &tx, validated)?;
            // No transaction is committed and no snapshot entry is written.
            drop(tx);
            if lease.deadline().is_none_or(|deadline| Instant::now() >= deadline) {
                return Err("base CST predecessor lease expired".into());
            }
            Ok((accepted, lease, read))
        }).await?
    }
}

fn validate(
    tx: &mut Transaction, binding: &BaseCstProposalBinding, proposal: &BaseCstProposal,
    limits: BaseCstAdmissionLimits,
) -> Result<ValidatedProposal> {
    if proposal.files.len() > limits.records {
        return Err("base CST proposal exceeds the record budget".into());
    }
    let mut supplied = BTreeMap::new();
    let mut graph = GraphWork {
        records: limits.records, record_bytes: limits.record_bytes, edges: limits.graph_edges,
    };
    let mut previous_index = None;
    for (id, encoded, is_index) in proposal.files.iter()
        .flat_map(|file| file.added.iter().map(|node| (node.id(), node.encoded(), false)))
        .chain(proposal.source_index.iter().map(|node| (node.id(), node.encoded(), true)))
    {
        graph.payload(encoded.len())?;
        if is_index && previous_index.replace(id).is_some_and(|previous| previous >= id) {
            return Err("base CST proposal source index is not in canonical order".into());
        }
        if supplied.insert(id, encoded).is_some() {
            return Err("duplicate base CST proposal record".into());
        }
    }

    let prior = trace(tx, proposal.base, &mut graph, None, None)?;
    if supplied.keys().any(|id| prior.contains_key(id)) {
        return Err("base CST proposal resends a selected predecessor record".into());
    }
    let added = trace(tx, proposal.root, &mut graph, Some((&prior, &supplied)), None)?;
    if supplied.len() != added.len() {
        return Err("base CST proposal record is not reachable from its proposed root".into());
    }
    let previous = read_index(tx, super::read_node(tx, proposal.base)?)?;
    let mut work = limits.cst_work();
    let mut validated_files = 0;
    let (current, config) = validate_snapshot_with(tx, proposal.root, |config| {
        if !matches_source_scope(config, &binding.selected) {
            return Err("proposed source does not match the bound workspace and target".into());
        }
        Ok(())
    }, |tx, file, categories| {
        let key = wosy_protocol::FileKey::from(&file.metadata.source);
        if previous.files.get(&key).is_some_and(|prior| prior.file == file.file) {
            return Ok(());
        }
        let checked = grammar_checked_file(tx, file, categories, FileTree::Authored, &mut work)?;
        if imports_from_cst(&checked.authored.ok_or("missing checked authored CST")?)? != file.metadata.imports {
            return Err("changed CST does not match canonical namespace provenance".into());
        }
        validated_files += 1;
        Ok(())
    })?;

    let expected = partition_cst_files(&previous.files, &current.files, |id| supplied.get(&id).copied())?;
    if expected.len() != proposal.files.len()
        || !expected.iter().zip(&proposal.files).all(|(plan, change)| {
            plan.file == change.file && plan.old_file_root == change.old_file_root
                && plan.new_file_root == change.new_file_root
                && plan.added.iter().copied().eq(change.added.iter().map(|node| node.id()))
        })
    {
        return Err("base CST proposal does not match the canonical per-file changes".into());
    }

    let admission = BaseCstAdmission {
        base: proposal.base, proposed: proposal.root, attempt: proposal.attempt,
        base_records: prior.len(), novel_records: supplied.len(), validated_files,
    };
    Ok(ValidatedProposal { admission, entry: config.entry, index: current, work })
}

pub(super) fn matches_source_scope(config: &SourceConfig, selected: &BuildSourceSelection) -> bool {
    config.target == selected.target && config.namespaces.get(&config.entry.package)
        .is_some_and(|owner| owner.git.is_none() && owner.project == selected.project)
}

fn grammar_checked_file(
    tx: &Transaction, file: &FileEntry, categories: &[String], tree: FileTree, work: &mut CstWork,
) -> Result<CheckedFile> {
    let checked = validate_file(tx, file, categories, tree, Some(work))?;
    let authored = checked.authored.as_ref().ok_or("source file has no authored CST")?;
    // Source text is reconstructed from the witnessed CST, not shipped or
    // reread from a live workspace. Use the ordinary parser as the oracle.
    let parsed = wosy_syntax::parse(file_source(file)?, authored.text().to_string(), categories);
    if !parsed.is_valid() || !same_cst(authored, &parsed.canonical_cst().root) || parsed.comments != checked.comments {
        return Err("changed CST does not match canonical source grammar or provenance".into());
    }
    Ok(checked)
}

/// Validate and provisionally install only the full canonical graph supplied
/// by the already-acknowledged workspace. Records outside it are not inputs.
pub(super) fn stage_reconciled_closure(
    tx: &mut Transaction, selected: &BuildSourceSelection, full: &Delta,
    limits: BaseCstAdmissionLimits, report: &mut PublishReport,
) -> Result<usize> {
    let mut graph = GraphWork { records: limits.records, record_bytes: limits.record_bytes, edges: limits.graph_edges };
    let mut supplied = BTreeMap::new();
    for (&id, bytes) in &full.nodes {
        graph.payload(bytes.len())?;
        supplied.insert(id, bytes.as_slice());
    }
    let reachable = trace(tx, selected.root, &mut graph, Some((&HashMap::new(), &supplied)), Some(report))?;
    if supplied.len() != reachable.len() {
        return Err("recovery payload includes records outside the acknowledged closure".into());
    }
    let mut work = limits.cst_work();
    let mut validated_files = 0;
    validate_snapshot_with(tx, selected.root, |config| {
        if !matches_source_scope(config, selected) {
            return Err("recovery source does not match the acknowledged workspace and target".into());
        }
        Ok(())
    }, |tx, file, categories| {
        let checked = grammar_checked_file(tx, file, categories, FileTree::Both, &mut work)?;
        admit_checked_namespace_authority(tx, file, checked, report)?;
        validated_files += 1;
        Ok(())
    })?;
    Ok(validated_files)
}

impl BaseCstAdmissionLimits {
    fn cst_work(self) -> CstWork {
        CstWork::new(self.expanded_record_bytes, self.cst_occurrences, self.cst_depth, self.authored_bytes)
    }
}

struct GraphWork {
    records: usize,
    record_bytes: usize,
    edges: usize,
}

impl GraphWork {
    fn payload(&mut self, bytes: usize) -> Result<()> {
        take(&mut self.records, 1, "base CST admission exceeds the record budget")?;
        take(&mut self.record_bytes, bytes, "base CST admission exceeds the encoded byte budget")
    }
}

pub(super) type CstWork = CstExpansionBudget;

fn take(remaining: &mut usize, amount: usize, reason: &'static str) -> Result<()> {
    *remaining = remaining.checked_sub(amount).ok_or(reason)?;
    Ok(())
}

type Candidates<'a> = (&'a HashMap<NodeId, Kind>, &'a BTreeMap<NodeId, &'a [u8]>);

/// First pass follows only the actual selected root. In the second pass,
/// existing global intern records outside that closure have no authority.
fn trace(
    tx: &mut Transaction, root: NodeId, work: &mut GraphWork, candidate: Option<Candidates<'_>>,
    mut publish: Option<&mut PublishReport>,
) -> Result<HashMap<NodeId, Kind>> {
    enum Step { Enter(NodeId, Expected), Leave(NodeId) }
    let mut pending = vec![Step::Enter(root, Expected::Exact(Kind::SourcePreparation))];
    let mut entering = HashMap::new();
    let mut done = HashMap::new();
    while let Some(step) = pending.pop() {
        let (id, expected) = match step {
            Step::Enter(id, expected) => (id, expected),
            Step::Leave(id) => {
                done.insert(id, entering.remove(&id).expect("entered rooted source node"));
                continue;
            }
        };
        if let Some(&kind) = done.get(&id) {
            expected.check(kind)?;
            continue;
        }
        if entering.contains_key(&id) { return Err("cyclic base CST source closure".into()); }
        if let Some(kind) = candidate.and_then(|(known, _)| known.get(&id)) {
            expected.check(*kind)?;
            continue;
        }

        let stored;
        let bytes: &[u8] = match candidate {
            Some((_, added)) => added.get(&id).copied().ok_or("base CST proposal is not closed over its selected predecessor")?,
            None => {
                stored = tx.get(&key(NODES, id))?.ok_or("selected base CST record is missing")?;
                work.payload(stored.len())?;
                &stored
            }
        };
        if Node::id(bytes) != id { return Err("base CST admission record hash mismatch".into()); }
        let node = Node::decode(bytes).ok_or("invalid base CST admission record")?;
        expected.check(node.kind)?;
        validate_shape(&node)?;
        take(&mut work.edges, node.children.len(), "base CST admission exceeds the graph edge budget")?;
        if let Some(report) = publish.as_deref_mut() {
            // Prove the supplied body independently even on a dedup hit.
            // Unrelated stored nodes never satisfy a missing submitted node.
            if tx.get(&key(INTERN, id))?.is_some() {
                if tx.get(&key(NODES, id))?.as_deref() != Some(bytes) {
                    return Err("stored source node differs from canonical recovery bytes".into());
                }
            } else {
                write_node(tx, id, bytes)?;
                for (index, &child) in node.children.iter().enumerate() {
                    write_edge(tx, id, child_edge(node.kind, index).0, child)?;
                }
                report.written += 1;
            }
        } else if candidate.is_some() {
            // Sparse preparation uses only a rollback-only source overlay.
            tx.set(&key(NODES, id), bytes)?;
        }
        entering.insert(id, node.kind);
        pending.push(Step::Leave(id));
        pending.extend(node.children.into_iter().enumerate().rev().map(|(index, child)| {
            Step::Enter(child, child_edge(node.kind, index).1)
        }));
    }
    Ok(done)
}

fn same_cst(expected: &CstNode, actual: &CstNode) -> bool {
    let mut wanted = expected.preorder_with_tokens();
    let mut observed = actual.preorder_with_tokens();
    loop {
        match (wanted.next(), observed.next()) {
            (None, None) => return true,
            (Some(WalkEvent::Enter(NodeOrToken::Node(a))), Some(WalkEvent::Enter(NodeOrToken::Node(b))))
                if a.kind() == b.kind() => {}
            (Some(WalkEvent::Enter(NodeOrToken::Token(a))), Some(WalkEvent::Enter(NodeOrToken::Token(b))))
                if a.kind() == b.kind() && a.text() == b.text() => {}
            (Some(WalkEvent::Leave(NodeOrToken::Node(_))), Some(WalkEvent::Leave(NodeOrToken::Node(_))))
            | (Some(WalkEvent::Leave(NodeOrToken::Token(_))), Some(WalkEvent::Leave(NodeOrToken::Token(_)))) => {}
            _ => return false,
        }
    }
}
