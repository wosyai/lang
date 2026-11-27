//! Admit one trusted-adapter-bound base CST proposal.
use super::{
    admit_checked_namespace_authority, check_source_marker, child_edge, file_source, key, read_index, validate_file,
    validate_shape, validate_snapshot_with, write_edge, write_node,
    project_declarations::UnpublishedDeclarations,
    retention_plan::{PreparedAllocation, SourceLease, SourceRead, MAX_RETAINED_DECLARATION_BYTES},
    RootedClosedComptimeProjection, RootedClosedComptimeRefusal, RootedClosedComptimeRequest,
    RootedPairCallGraphComptimeProjection,
    CheckedFile, Expected, FileTree, Index, PreparedBaseModule, PublishReport, Result,
    SourceStorageWorker, SourceStore, INTERN, NODES, SNAPSHOT_ROOTS,
};
use super::grammar_receipts::{GrammarReceipts, VerifiedFileGrammar, VerifiedGrammarBatch};
use rowan::{NodeOrToken, WalkEvent};
use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use surrealkv::{Durability, Mode, Transaction};
use wosy_protocol::{
    imports_from_cst, partition_cst_files, BaseCstProposal, BaseCstProposalBinding, BuildSourceSelection,
    CstExpansionBudget, Delta, FileEntry, FileKey, Kind, Node, NodeId, SourceConfig, SourceDeltaCommit,
    SourceDeltaPublication, BASE_CST_DELTA_VERSION,
};
use wosy_compiler::{ClosedComptimeRejection, CLOSED_COMPTIME_MAX_SOURCE_BYTES};
use wosy_syntax::{CstNode, FreshParseResult, SyntaxKind};

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

/// Authored syntax in the exact selected BASE closure. Present proves
/// one occurrence, not full verification or specialized reachability.
/// Unverified is never an empty fragment set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthoredComptimeCoverage {
    Absent,
    Present,
    Unverified,
}

impl AuthoredComptimeCoverage {
    fn observe(&mut self, authored_comptime: Option<bool>) {
        match authored_comptime {
            Some(true) => *self = Self::Present,
            None if *self != Self::Present => *self = Self::Unverified,
            _ => {}
        }
    }
}

/// An immutable, ordinary base declaration projection of a proposal. Holding
/// it retains its actual predecessor, but does not occupy a worker permit.
/// It is not a protocol Prepared response, fragment IR, or a typed program.
pub struct PreparedBaseDeclarations {
    identity: PreparedBaseIdentity,
    admission: BaseCstAdmission,
    declarations: UnpublishedDeclarations,
    authored_comptime: AuthoredComptimeCoverage,
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

    /// Coverage is bound to this exact prepared proposal and live source
    /// lease. No semantic result, publication, or runner is implied.
    pub fn authored_comptime_coverage(&self) -> Option<AuthoredComptimeCoverage> {
        self.lease.finish_current(|| self.authored_comptime)
    }

    pub fn lease_is_live(&self) -> bool {
        self.lease.deadline().is_some_and(|deadline| Instant::now() < deadline)
    }
}

/// Owned result of the opt-in pending first-initializer projection. Its
/// evaluator text is a preview, not a published root or a Prepared response.
pub struct PreparedPendingFirstComptime {
    base: PreparedBaseDeclarations,
    binding: BaseCstProposalBinding,
    observation: PendingFirstObservation,
}

struct PendingFirstObservation {
    projection: PendingFirstComptimeProjection,
    grammar: Option<VerifiedFileGrammar>,
}

impl From<PendingFirstComptimeProjection> for PendingFirstObservation {
    fn from(projection: PendingFirstComptimeProjection) -> Self { Self { projection, grammar: None } }
}

impl PreparedPendingFirstComptime {
    pub fn base(&self) -> &PreparedBaseDeclarations { &self.base }

    pub fn projection(&self) -> Option<&PendingFirstComptimeProjection> {
        self.base.lease.finish_current(|| &self.observation.projection)
    }
}

/// Read-only owner of one pending startup graph. It cannot be exchanged
/// for a source commit or a semantic result.
pub struct PreparedPendingPairCallGraphComptime {
    base: PreparedBaseDeclarations,
    binding: BaseCstProposalBinding,
    projection: PendingPairCallGraphComptimeProjection,
}

impl PreparedPendingPairCallGraphComptime {
    pub fn base(&self) -> &PreparedBaseDeclarations { &self.base }
    pub fn binding(&self) -> &BaseCstProposalBinding { &self.binding }

    pub fn projection(&self) -> Option<&PendingPairCallGraphComptimeProjection> {
        self.base.lease.finish_current(|| &self.projection)
    }
}

#[derive(Debug)]
pub enum PendingEntryComptimeProjection<Projection> {
    RequiresSingleModule,
    EntryUnchanged,
    Candidate(Projection),
}

pub type PendingFirstComptimeProjection = PendingEntryComptimeProjection<RootedClosedComptimeProjection>;
pub type PendingPairCallGraphComptimeProjection = PendingEntryComptimeProjection<RootedPairCallGraphComptimeProjection>;

impl PendingFirstComptimeProjection {
    fn retained_bytes(&self) -> usize {
        match self {
            Self::Candidate(RootedClosedComptimeProjection::Emitted(fragment)) => fragment.bytes.len(),
            _ => 0,
        }
    }
}

struct ValidatedProposal {
    admission: BaseCstAdmission,
    config: SourceConfig,
    index: Index,
    work: CstWork,
    grammar: VerifiedGrammarBatch,
    authored_comptime: AuthoredComptimeCoverage,
    selected_changed_parse: Option<(FreshParseResult, bool)>,
    authored_reparsed_files: usize,
}

#[derive(Default)]
struct AdmissionInspection<'a> {
    predecessor_grammar: Option<&'a GrammarReceipts>,
    selected_file: Option<&'a FileKey>,
    prepared_changed: Option<VerifiedFileGrammar>,
}

struct SelectedPendingEntry<'a> {
    artifact_profile: &'a wosy_project::ArtifactProfile,
    file: &'a FileEntry,
    fresh: FreshParseResult,
    authored_comptime: bool,
}

enum PendingEntryRefusal {
    RequiresSingleModule,
    EntryUnchanged,
    Refused(RootedClosedComptimeRefusal),
}

impl PendingEntryRefusal {
    fn project<T>(self, refused: impl FnOnce(RootedClosedComptimeRefusal) -> T) -> PendingEntryComptimeProjection<T> {
        match self {
            Self::RequiresSingleModule => PendingEntryComptimeProjection::RequiresSingleModule,
            Self::EntryUnchanged => PendingEntryComptimeProjection::EntryUnchanged,
            Self::Refused(reason) => PendingEntryComptimeProjection::Candidate(refused(reason)),
        }
    }
}

impl ValidatedProposal {
    fn selected_pending_entry(
        &mut self, request: &RootedClosedComptimeRequest,
    ) -> std::result::Result<SelectedPendingEntry<'_>, PendingEntryRefusal> {
        use PendingEntryRefusal::Refused;
        use RootedClosedComptimeRefusal as Refusal;
        let Some(artifact_profile) = self.config.profiles.get(&request.profile) else {
            return Err(Refused(Refusal::ProfileNotSelected));
        };
        let Some(file) = self.index.files.get(&request.file) else {
            return Err(Refused(Refusal::FileNotRooted));
        };
        if request.file != self.config.entry {
            return Err(Refused(Refusal::NotTargetEntry));
        }
        if self.index.files.len() != 1 {
            return Err(PendingEntryRefusal::RequiresSingleModule);
        }
        if file.metadata.bytes as usize > CLOSED_COMPTIME_MAX_SOURCE_BYTES {
            return Err(Refused(Refusal::Compiler(ClosedComptimeRejection::SourceLimit)));
        }
        let Some((fresh, authored_comptime)) = self.selected_changed_parse.take() else {
            return Err(PendingEntryRefusal::EntryUnchanged);
        };
        Ok(SelectedPendingEntry { artifact_profile, file, fresh, authored_comptime })
    }

    fn pending_first_comptime(
        &mut self, store: &SourceStore, request: &RootedClosedComptimeRequest,
    ) -> Result<PendingFirstObservation> {
        let root = self.admission.proposed;
        let target = self.config.target.clone();
        let entry = match self.selected_pending_entry(request) {
            Ok(entry) => entry,
            Err(reason) => return Ok(reason.project(RootedClosedComptimeProjection::Refused).into()),
        };
        let projected = store.project_first_comptime_from_pending_parse(root, entry.file.authored, target, request, &entry.fresh)?;
        let grammar = matches!(&projected, RootedClosedComptimeProjection::Emitted(_))
            .then(|| VerifiedFileGrammar::for_checked(entry.file, entry.authored_comptime));
        Ok(PendingFirstObservation { projection: PendingFirstComptimeProjection::Candidate(projected), grammar })
    }

    fn pending_pair_call_graph(
        &mut self, store: &SourceStore, request: &RootedClosedComptimeRequest, max_wasip1_stdout: Option<usize>,
    ) -> Result<PendingPairCallGraphComptimeProjection> {
        let root = self.admission.proposed;
        let target = self.config.target.clone();
        let entry = match self.selected_pending_entry(request) {
            Ok(entry) => entry,
            Err(reason) => return Ok(reason.project(RootedPairCallGraphComptimeProjection::Refused)),
        };
        if max_wasip1_stdout.is_some()
            && (entry.artifact_profile.backend != "llvm" || entry.artifact_profile.output != "wasm-wasip1")
        {
            return Ok(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Refused(
                RootedClosedComptimeRefusal::ProfileNotWasip1,
            )));
        }
        let projected = store.project_pair_call_graph_from_pending_parse(
            root, entry.file.authored, target, request, &entry.fresh, max_wasip1_stdout,
        )?;
        Ok(PendingPairCallGraphComptimeProjection::Candidate(projected))
    }
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

enum PublicationLease {
    Open(Duration),
    Prepared { lease: Arc<SourceLease>, read: SourceRead, grammar: VerifiedFileGrammar },
}

/// Local source-only commit and the actual number of canonical authored
/// parser calls on its successful attempt. Not a new wire message.
pub struct PreparedSourcePublication {
    pub source: SourceDeltaPublication,
    pub authored_reparsed_files: usize,
}

impl SourceStorageWorker {
    /// Validate and publish just the canonical base-source delta. The trusted
    /// adapter binds one pending attempt and its acknowledged predecessor.
    /// Uses the existing rooted lease and worker; no data from a missing
    /// predecessor is assumed. An admitted cache commit may outlive its
    /// observer, but a stale local attempt cannot select it.
    pub async fn publish_bound_base_source(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, lifetime: Duration,
    ) -> Result<SourceDeltaPublication> {
        Ok(self.publish_base_source(binding, Arc::new(proposal), limits, PublicationLease::Open(lifetime)).await?.source)
    }

    /// Consume the exact live preview accepted by this local store. The
    /// caller still uses the ordinary client acknowledgement compare-and-set;
    /// this publishes only canonical base source, never the evaluator/result.
    pub async fn publish_prepared_first_comptime_source(
        &self, binding: BaseCstProposalBinding, proposal: impl Into<Arc<BaseCstProposal>>,
        prepared: PreparedPendingFirstComptime, limits: BaseCstAdmissionLimits,
    ) -> Result<PreparedSourcePublication> {
        if binding != prepared.binding || !Arc::ptr_eq(&self.retained_store(), &prepared.base._store) {
            return Err("prepared source owner does not match this pending attempt or store".into());
        }
        let grammar = prepared.observation.grammar.ok_or("prepared source owner has no accepted entry fragment")?;
        let PreparedBaseDeclarations { lease, _read: read, .. } = prepared.base;
        self.publish_base_source(binding, proposal.into(), limits, PublicationLease::Prepared { lease, read, grammar }).await
    }

    async fn publish_base_source(
        &self, binding: BaseCstProposalBinding, proposal: Arc<BaseCstProposal>,
        limits: BaseCstAdmissionLimits, admission: PublicationLease,
    ) -> Result<PreparedSourcePublication> {
        check_binding(&binding, &proposal)?;
        let slot = self.try_slot()?;
        self.spawn_reserved_query(slot, None, move |worker| async move {
            let binding = Arc::new(binding);
            let (lease, _prepared_read, prepared_changed) = match admission {
                PublicationLease::Open(lifetime) => {
                    let selected = binding.selected.clone();
                    let Some(lease) = worker.phase(None, move |store| store.lease_build_source(selected, lifetime)).await? else {
                        return Ok(PreparedSourcePublication {
                            source: SourceDeltaPublication::PredecessorMissing, authored_reparsed_files: 0,
                        });
                    };
                    (Arc::new(lease), None, None)
                }
                PublicationLease::Prepared { lease, read, grammar } => (lease, Some(read), Some(grammar)),
            };
            let grant = lease.grant();
            for conflicts in 0..=1 {
                let binding = Arc::clone(&binding);
                let proposal = Arc::clone(&proposal);
                let (mut tx, _read, result, grammar, authored_reparsed_files) = worker.phase(None, move |store| {
                    let (mut tx, read) = store.begin_leased_source(grant, Mode::ReadWrite)?
                        .ok_or("base CST predecessor lease expired")?;
                    tx.set_durability(Durability::Eventual);
                    let marker = key(SNAPSHOT_ROOTS, proposal.root);
                    let already_present = if let Some(observed) = tx.get_for_update(&marker)? {
                        check_source_marker(proposal.root, &observed)?;
                        true
                    } else {
                        false
                    };
                    let mut report = PublishReport::default();
                    let validated = validate(&mut tx, &binding, &proposal, limits, Some(&mut report),
                        AdmissionInspection { prepared_changed, ..Default::default() })?;
                    if !already_present { tx.set(&marker, &proposal.root.0)?; }
                    Ok((tx, read, SourceDeltaCommit {
                        binding: binding.as_ref().clone(), validated_files: validated.admission.validated_files,
                        written_records: report.written, already_present, conflicts,
                    }, validated.grammar, validated.authored_reparsed_files))
                }).await?;
                if lease.deadline().is_none_or(|deadline| Instant::now() >= deadline) {
                    return Err("base CST predecessor lease expired".into());
                }
                match tx.commit().await {
                    Ok(()) => {
                        worker.store.grammar_receipts.accept(grammar);
                        return Ok(PreparedSourcePublication {
                            source: SourceDeltaPublication::Committed(result), authored_reparsed_files,
                        });
                    }
                    Err(surrealkv::Error::TransactionWriteConflict) if conflicts == 0 => {}
                    Err(error) => return Err(error.into()),
                }
            }
            unreachable!("a second commit conflict is returned")
        }).await?
    }

    /// Check only the authenticated local adapter's selected/pending pair.
    /// The proposal cannot open a lease or pick an unrelated base. Nothing
    /// returned here is a lease, Prepared handle, IR, or publication ack.
    pub async fn validate_base_cst_proposal(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, lifetime: Duration,
    ) -> Result<BaseCstAdmission> {
        let (admission, _, _) = self.with_cst_admission(binding, proposal, limits, AdmissionLease::Open(lifetime), false, None, |_, _, valid| {
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
        let (base, _) = self.prepare_base_cst_observing(binding, proposal, limits, declaration_bytes, lease,
            None, |_, _| Ok(((), 0))).await?;
        Ok(base)
    }

    /// Local preview for a changed singleton target entry whose first
    /// authored initializer can be compiled with no parameter authority.
    /// The existing selector's layout is a projection, not a builder grant.
    pub async fn prepare_pending_first_comptime(
        &self, binding: BaseCstProposalBinding, proposal: impl Into<Arc<BaseCstProposal>>,
        request: RootedClosedComptimeRequest, declaration_bytes: usize, lifetime: Duration,
    ) -> Result<PreparedPendingFirstComptime> {
        request.check_selector_bytes()?;
        let captured_binding = binding.clone();
        let (base, observation) = self.prepare_base_cst_observing(binding, proposal.into(), request.source_limits,
            declaration_bytes, AdmissionLease::Open(lifetime), Some(request.file.clone()), move |store, valid| {
                let observation = valid.pending_first_comptime(store, &request)?;
                let bytes = observation.projection.retained_bytes();
                Ok((observation, bytes))
            }).await?;
        Ok(PreparedPendingFirstComptime { base, binding: captured_binding, observation })
    }

    /// One local read-only finite pair call graph in the changed singleton entry.
    /// This does not select a builder, runner, new source root or result.
    pub async fn prepare_pending_pair_call_graph_comptime(
        &self, binding: BaseCstProposalBinding, proposal: impl Into<Arc<BaseCstProposal>>,
        request: RootedClosedComptimeRequest, declaration_bytes: usize, lifetime: Duration,
    ) -> Result<PreparedPendingPairCallGraphComptime> {
        self.prepare_pending_pair_graph_delivery(binding, proposal.into(), request, declaration_bytes, lifetime, None).await
    }

    /// Explicit selected llvm/wasm-wasip1 profile preview. Use the same
    /// admitted scalar plan and incumbent canonical stdout emitter. This
    /// does not authorize a tool, launch, result, or source publication.
    pub async fn prepare_pending_pair_call_graph_wasip1_stdout(
        &self, binding: BaseCstProposalBinding, proposal: impl Into<Arc<BaseCstProposal>>,
        request: RootedClosedComptimeRequest, declaration_bytes: usize, lifetime: Duration, max_stdout: usize,
    ) -> Result<PreparedPendingPairCallGraphComptime> {
        self.prepare_pending_pair_graph_delivery(binding, proposal.into(), request, declaration_bytes, lifetime, Some(max_stdout)).await
    }

    async fn prepare_pending_pair_graph_delivery(
        &self, binding: BaseCstProposalBinding, proposal: Arc<BaseCstProposal>,
        request: RootedClosedComptimeRequest, declaration_bytes: usize, lifetime: Duration, max_wasip1_stdout: Option<usize>,
    ) -> Result<PreparedPendingPairCallGraphComptime> {
        request.check_selector_bytes()?;
        let captured_binding = binding.clone();
        let (base, projection) = self.prepare_base_cst_observing(binding, proposal, request.source_limits,
            declaration_bytes, AdmissionLease::Open(lifetime), Some(request.file.clone()), move |store, valid| {
                let projection = valid.pending_pair_call_graph(store, &request, max_wasip1_stdout)?;
                let bytes = match &projection {
                    PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan)) =>
                        plan.evaluators().iter().map(|evaluator| evaluator.bytes().len()).sum(),
                    _ => 0,
                };
                Ok((projection, bytes))
            }).await?;
        Ok(PreparedPendingPairCallGraphComptime { base, binding: captured_binding, projection })
    }

    async fn prepare_base_cst_observing<T: Send + 'static>(
        &self, binding: BaseCstProposalBinding, proposal: impl Borrow<BaseCstProposal> + Send + 'static,
        limits: BaseCstAdmissionLimits, declaration_bytes: usize, lease: AdmissionLease,
        selected_file: Option<FileKey>,
        observe: impl FnOnce(&SourceStore, &mut ValidatedProposal) -> Result<(T, usize)> + Send + 'static,
    ) -> Result<(PreparedBaseDeclarations, T)> {
        let (owned, lease, read) = self.with_cst_admission(binding, proposal, limits, lease, true, selected_file,
            move |store, tx, mut valid| {
                let mut allocation = store.source_reads.reserve_preparation()?;
                let (projection, evaluator_bytes) = observe(store, &mut valid)?;
                let ValidatedProposal { admission, config, index, mut work, authored_comptime, .. } = valid;
                let declarations = store.prepare_unpublished_declarations(tx, &config.entry, index, &mut work,
                    declaration_bytes.min(MAX_RETAINED_DECLARATION_BYTES))?;
                allocation.charge(declarations.declaration_bytes + evaluator_bytes)?;
                let identity = prepared_identity(&admission, declarations.entry_projection, store.derivation_image());
                Ok((identity, admission, declarations, authored_comptime, projection, allocation))
            },
        ).await?;
        let (identity, admission, declarations, authored_comptime, projection, allocation) = owned;
        Ok((PreparedBaseDeclarations {
            identity, admission, declarations, authored_comptime, lease, _read: read, _allocation: allocation,
            _store: self.retained_store(),
        }, projection))
    }

    async fn with_cst_admission<T: Send + 'static>(
        &self, binding: BaseCstProposalBinding, proposal: impl Borrow<BaseCstProposal> + Send + 'static,
        limits: BaseCstAdmissionLimits, admission_lease: AdmissionLease, observe_comptime: bool,
        selected_file: Option<FileKey>,
        use_validated: impl FnOnce(&SourceStore, &Transaction, ValidatedProposal) -> Result<T> + Send + 'static,
    ) -> Result<(T, Arc<SourceLease>, SourceRead)> {
        check_binding(&binding, proposal.borrow())?;
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
            let validated = validate(&mut tx, &binding, proposal.borrow(), limits, None, AdmissionInspection {
                predecessor_grammar: observe_comptime.then_some(&store.grammar_receipts),
                selected_file: selected_file.as_ref(), prepared_changed: None,
            })?;
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

fn check_binding(binding: &BaseCstProposalBinding, proposal: &BaseCstProposal) -> Result<()> {
    if proposal.version != BASE_CST_DELTA_VERSION { return Err("unsupported base CST proposal version".into()); }
    if binding.attempt <= 0 || proposal.attempt != binding.attempt
        || proposal.base != binding.selected.root || proposal.root != binding.proposed || proposal.root == proposal.base
    {
        return Err("base CST proposal does not match the bound source attempt".into());
    }
    Ok(())
}

fn validate(
    tx: &mut Transaction, binding: &BaseCstProposalBinding, proposal: &BaseCstProposal,
    limits: BaseCstAdmissionLimits, mut publish: Option<&mut PublishReport>,
    inspection: AdmissionInspection<'_>,
) -> Result<ValidatedProposal> {
    let AdmissionInspection { predecessor_grammar, selected_file, prepared_changed } = inspection;
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
    let added = trace(tx, proposal.root, &mut graph, Some((&prior, &supplied)), publish.as_deref_mut())?;
    if supplied.len() != added.len() {
        return Err("base CST proposal record is not reachable from its proposed root".into());
    }
    let previous = read_index(tx, super::read_node(tx, proposal.base)?)?;
    let mut work = limits.cst_work();
    let mut validated_files = 0;
    let mut grammar = VerifiedGrammarBatch::default();
    let mut selected_changed_parse = None;
    let mut authored_reparsed_files = 0;
    let mut authored_comptime = if predecessor_grammar.is_some() { AuthoredComptimeCoverage::Absent }
        else { AuthoredComptimeCoverage::Unverified };
    let (current, config) = validate_snapshot_with(tx, proposal.root, |config| {
        if !matches_source_scope(config, &binding.selected) {
            return Err("proposed source does not match the bound workspace and target".into());
        }
        Ok(())
    }, |tx, file, categories| {
        let key = wosy_protocol::FileKey::from(&file.metadata.source);
        if previous.files.get(&key).is_some_and(|prior| prior.file == file.file) {
            if authored_comptime != AuthoredComptimeCoverage::Present {
                if let Some(memo) = predecessor_grammar {
                    authored_comptime.observe(memo.authored_comptime(file));
                }
            }
            return Ok(());
        }
        let (checked, present) = if let Some(proof) = prepared_changed {
            let present = proof.observe(file).ok_or("prepared grammar does not match the exact changed file")?;
            let checked = validate_file(tx, file, categories, FileTree::Executable, Some(&mut work))?;
            (checked, present)
        } else {
            let tree = if publish.is_some() { FileTree::Both } else { FileTree::Authored };
            let (checked, fresh, present) = grammar_checked_file(tx, file, categories, tree, &mut work)?;
            authored_reparsed_files += 1;
            if selected_file == Some(&key) && file.metadata.bytes as usize <= CLOSED_COMPTIME_MAX_SOURCE_BYTES {
                selected_changed_parse = Some((fresh, present));
            }
            (checked, present)
        };
        if predecessor_grammar.is_some() { authored_comptime.observe(Some(present)); }
        if let Some(report) = publish.as_deref_mut() {
            admit_checked_namespace_authority(tx, file, checked, report)?;
            grammar.verified(file, present);
        } else if imports_from_cst(&checked.authored.ok_or("missing checked authored CST")?)? != file.metadata.imports {
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
    Ok(ValidatedProposal {
        admission, config, index: current, work, grammar, authored_comptime, selected_changed_parse, authored_reparsed_files,
    })
}

pub(super) fn matches_source_scope(config: &SourceConfig, selected: &BuildSourceSelection) -> bool {
    config.target == selected.target && config.namespaces.get(&config.entry.package)
        .is_some_and(|owner| owner.git.is_none() && owner.project == selected.project)
}

fn grammar_checked_file(
    tx: &Transaction, file: &FileEntry, categories: &[String], tree: FileTree, work: &mut CstWork,
) -> Result<(CheckedFile, FreshParseResult, bool)> {
    let checked = validate_file(tx, file, categories, tree, Some(work))?;
    let (fresh, present) = verified_authored_parse(file, categories, &checked)?
        .ok_or("changed CST does not match canonical source grammar or provenance")?;
    Ok((checked, fresh, present))
}

pub(super) fn verified_authored_comptime(
    file: &FileEntry, categories: &[String], checked: &CheckedFile,
) -> Result<Option<bool>> {
    Ok(verified_authored_parse(file, categories, checked)?.map(|(_, present)| present))
}

pub(super) fn verified_authored_parse(
    file: &FileEntry, categories: &[String], checked: &CheckedFile,
) -> Result<Option<(wosy_syntax::FreshParseResult, bool)>> {
    let authored = checked.authored.as_ref().ok_or("source file has no authored CST")?;
    // Use the ordinary parser and the checked Rowan text. A lease alone is
    // not grammar authority; compiler consumers get only the matching parse.
    let fresh = wosy_syntax::parse_with_snapshot(file_source(file)?, authored.text().to_string(), categories);
    let parsed = fresh.result();
    if !parsed.is_valid() { return Ok(None); }
    let Some(present) = same_cst(authored, &parsed.canonical_cst().root) else { return Ok(None); };
    Ok((parsed.comments == checked.comments).then_some((fresh, present)))
}

/// Validate and provisionally install only the full canonical graph supplied
/// by the already-acknowledged workspace. Records outside it are not inputs.
/// A caller may retain the returned grammar evidence only after commit.
pub(super) fn stage_reconciled_closure(
    tx: &mut Transaction, selected: &BuildSourceSelection, full: &Delta,
    limits: BaseCstAdmissionLimits, report: &mut PublishReport,
) -> Result<(usize, VerifiedGrammarBatch)> {
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
    let mut grammar = VerifiedGrammarBatch::default();
    validate_snapshot_with(tx, selected.root, |config| {
        if !matches_source_scope(config, selected) {
            return Err("recovery source does not match the acknowledged workspace and target".into());
        }
        Ok(())
    }, |tx, file, categories| {
        let (checked, _, present) = grammar_checked_file(tx, file, categories, FileTree::Both, &mut work)?;
        admit_checked_namespace_authority(tx, file, checked, report)?;
        grammar.verified(file, present);
        validated_files += 1;
        Ok(())
    })?;
    Ok((validated_files, grammar))
}

impl BaseCstAdmissionLimits {
    pub(super) fn cst_work(self) -> CstWork {
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

pub(super) fn take(remaining: &mut usize, amount: usize, reason: &'static str) -> Result<()> {
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

fn same_cst(expected: &CstNode, actual: &CstNode) -> Option<bool> {
    let mut authored_comptime = false;
    let mut wanted = expected.preorder_with_tokens();
    let mut observed = actual.preorder_with_tokens();
    loop {
        match (wanted.next(), observed.next()) {
            (None, None) => return Some(authored_comptime),
            (Some(WalkEvent::Enter(NodeOrToken::Node(a))), Some(WalkEvent::Enter(NodeOrToken::Node(b))))
                if a.kind() == b.kind() => { authored_comptime |= a.kind() == SyntaxKind::ComptimeExpr; }
            (Some(WalkEvent::Enter(NodeOrToken::Token(a))), Some(WalkEvent::Enter(NodeOrToken::Token(b))))
                if a.kind() == b.kind() && a.text() == b.text() => {}
            (Some(WalkEvent::Leave(NodeOrToken::Node(_))), Some(WalkEvent::Leave(NodeOrToken::Node(_))))
            | (Some(WalkEvent::Leave(NodeOrToken::Token(_))), Some(WalkEvent::Leave(NodeOrToken::Token(_)))) => {}
            _ => return None,
        }
    }
}
