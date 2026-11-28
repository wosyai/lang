//! Product dispatcher for the leased, diagnostic-only Build projection.
use super::{
    retention_plan::SourceLease, rooted_comptime::SiteScope,
    source_reconciliation::reconcile_on_worker, BaseCstAdmissionLimits,
    storage_worker::{QueryCancellation, QueryWorker},
    PreparedBaseDeclarations, Result, RootedBuildAnalysis, RootedClosedComptimeProjection,
    RootedClosedComptimeRequest, RootedDirectFunctionComptimeProjection, RootedDirectStartupComptimeProjection,
    RootedForwardedPairStartupComptimeProjection, SourceStore,
    RootedLlvmIdentity,
    RootedLlvmProjection, RootedPreparedProject,
    SavedBuildDiagnostic, SavedBuildDiagnosticView, SourcePinPage, SourcePinPageCursor, SourceReconciliation,
    SourceStorageWorker,
};
use std::{num::NonZeroUsize, sync::Arc};
use std::time::{Duration, Instant};
use wosy_compiler::{derive_direct_pair_startup_comptime_plan, derive_direct_startup_comptime_plan, Diagnostic};
use wosy_project::packages::InstalledGitBase;
use wosy_protocol::{
    BaseCstProposal, BaseCstProposalBinding, BaseDeclarationConnectionId, BuildDiagnosticRequest, BuildDiagnosticResponse,
    BuildSourceSelection, Delta, NodeId, OrdinaryLlvmBuilderWitness, SourcePinIntent, BUILD_DIAGNOSTIC_VERSION,
};

/// Bound to one published source selected by a trusted local adapter. No
/// index, compiler artifact or raw node access can be requested by the client.
pub struct BuildDiagnosticSession<'a> {
    worker: &'a SourceStorageWorker,
    lease: Arc<SourceLease>,
}

/// Borrowed exact committed source owner. Dropping the parent session
/// releases authority; this is neither a pending source nor a tool result.
#[cfg(target_os = "linux")]
pub struct SelectedPairComptimeOwner<'a> {
    lease: &'a SourceLease,
    configuration_inputs: NodeId,
    plan: super::RootedPairCallGraphComptimePlan,
    execution_seal: wosy_project::packages::PreparedComptimeExecutionSeal,
}

#[cfg(target_os = "linux")]
pub enum SelectedPairComptimePreparation<'a> {
    Ready(SelectedPairComptimeOwner<'a>),
    Refused(super::RootedClosedComptimeRefusal),
}

/// Validated ordinary LLVM from the exact local selected scope. The
/// client still holds source/watch and configured artifact publication cuts.
#[cfg(target_os = "linux")]
pub struct SelectedPairComptimeEmission {
    pub source_root: NodeId,
    pub result_root: wosy_protocol::ComptimeResultRoot,
    pub partition: wosy_compiler::LlvmPartition,
}

#[cfg(target_os = "linux")]
impl SelectedPairComptimeOwner<'_> {
    pub fn configuration_inputs(&self) -> NodeId { self.configuration_inputs }
    pub fn execution_seal(&self) -> &wosy_project::packages::PreparedComptimeExecutionSeal { &self.execution_seal }
    pub fn plan(&self) -> Option<&super::RootedPairCallGraphComptimePlan> {
        self.lease.finish_current(|| &self.plan)
    }

    /// Use the same owned execution intake as a pending proposal. Do not
    /// turn a committed source into a synthetic base/pending pair.
    pub fn emit_owned_result(
        &self, acknowledged_generation: i64,
        executed: &wosy_project::packages::OwnedLocalComptimeExecution,
    ) -> Result<Option<SelectedPairComptimeEmission>> {
        let Some(plan) = self.plan() else { return Ok(None); };
        let project = std::path::Path::new(&plan.evidence().site().source.project);
        let decoded = super::source_proposal::decode_owned_pair_results(
            plan, project, self.configuration_inputs, acknowledged_generation, &self.execution_seal, executed,
        )?;
        let output = plan.wasip1_output().ok_or("selected paired plan is not ordinary WASIp1")?;
        let partition = output.emit_ordinary_startup(&decoded.values)
            .map_err(|error| format!("ordinary comptime continuation rejected: {error:?}"))?;
        Ok(self.lease.finish_current(|| SelectedPairComptimeEmission {
            source_root: plan.source_root(), result_root: decoded.root, partition,
        }))
    }
}

#[cfg(target_os = "linux")]
pub enum SelectedComptimePreparation<'a> {
    Pair(SelectedPairComptimeOwner<'a>),
    Ordered(SelectedOrderedComptimeOwner<'a>),
    Refused(super::RootedClosedComptimeRefusal),
}

#[cfg(target_os = "linux")]
pub struct SelectedOrderedComptimeOwner<'a> {
    lease: &'a SourceLease,
    configuration_inputs: NodeId,
    plan: super::RootedOrderedComptime,
    execution_seal: wosy_project::packages::PreparedComptimeExecutionSeal,
}

#[cfg(target_os = "linux")]
impl SelectedOrderedComptimeOwner<'_> {
    pub fn configuration_inputs(&self) -> NodeId { self.configuration_inputs }
    pub fn execution_seal(&self) -> &wosy_project::packages::PreparedComptimeExecutionSeal { &self.execution_seal }
    pub fn plan(&self) -> Option<&super::RootedOrderedComptime> {
        self.lease.finish_current(|| &self.plan)
    }

    pub fn emit_owned_results(
        &self, generation: i64, executions: &[wosy_project::packages::OwnedLocalComptimeExecution],
    ) -> Result<Option<SelectedPairComptimeEmission>> {
        let Some(plan) = self.plan() else { return Ok(None); };
        if plan.evaluators().len() != executions.len() {
            return Err("selected comptime set is incomplete".into());
        }
        let mut identities = Vec::with_capacity(executions.len());
        let mut values = Vec::with_capacity(executions.len());
        for (evaluator, executed) in plan.evaluators().iter().zip(executions) {
            let (identity, value) = super::source_proposal::decode_ordered_closed_result(
                plan, evaluator, self.configuration_inputs, generation, &self.execution_seal, executed,
            )?;
            identities.push(identity);
            values.push(value);
        }
        let result_root = wosy_protocol::ComptimeResultRoot::identify(
            plan.source_root(), plan.target(), plan.profile(), identities,
        )?;
        let partition = plan.evaluator.emit_with_values(values)
            .map_err(|error| format!("ordered comptime continuation rejected: {error:?}"))?;
        Ok(self.lease.finish_current(|| SelectedPairComptimeEmission {
            source_root: plan.source_root(), result_root, partition,
        }))
    }
}

pub enum BuildSourceOpening<'a> {
    ReplayRequired { root: NodeId },
    Opened { session: BuildDiagnosticSession<'a>, restored: Option<SourceReconciliation> },
}

impl Drop for BuildDiagnosticSession<'_> {
    // A queued operation or passive owner may still hold the Arc, but only
    // this connection owns continued lease authority.
    fn drop(&mut self) { self.lease.release(); }
}

impl SourceStorageWorker {
    /// Open a process-local session for a root the adapter already selected.
    /// TTL is host policy, not a field supplied with a compiler request.
    pub async fn open_build_diagnostics(
        &self, selected: BuildSourceSelection, lifetime: Duration,
    ) -> Result<Option<BuildDiagnosticSession<'_>>> {
        self.open_build_diagnostics_before(selected, lifetime, None).await
    }

    /// Accept the normal selected-source lease only after verifying the
    /// witnessed subset of one direct installed Git base. This is a local
    /// adapter operation, not a wire action or whole-workspace claim.
    pub async fn open_installed_git_build_scope(
        &self, selected: BuildSourceSelection, base: InstalledGitBase, limits: BaseCstAdmissionLimits,
        lifetime: Duration,
    ) -> Result<Option<BuildDiagnosticSession<'_>>> {
        let slot = self.try_slot()?;
        let lease = self.spawn_reserved(slot, move |store| {
            if base.project().to_str() != Some(selected.project.as_str()) {
                return Err(super::InstalledGitScopeRejection::Ownership.into());
            }
            store.lease_build_source_inspect(selected, lifetime, None, None, |tx, top, config| {
                super::installed_git_scope::verify(tx, top, config, &base, limits)
            })
        }).await??;
        Ok(lease.map(|lease| BuildDiagnosticSession { worker: self, lease: Arc::new(lease) }))
    }

    pub(super) async fn open_build_diagnostics_before(
        &self, selected: BuildSourceSelection, lifetime: Duration, start_deadline: Option<Instant>,
    ) -> Result<Option<BuildDiagnosticSession<'_>>> {
        let slot = self.try_slot()?;
        let lease = self.spawn_reserved(slot, move |store| store.lease_build_source_before(selected, lifetime, start_deadline))
            .await??;
        Ok(lease.map(|lease| BuildDiagnosticSession { worker: self, lease: Arc::new(lease) }))
    }

    /// Open the ordinary rooted lease, requesting replay only if the trusted
    /// adapter's acknowledged source is absent. The adapter owns its SQLite
    /// read, selects whether to replay, and is responsible for a fresh scope
    /// if selection advances. No lease is created before a recovered commit.
    /// A valid cache restoration may survive a later lease refusal or the
    /// observer dropping the request; neither grants a session to it.
    pub async fn open_recoverable_build_diagnostics(
        &self, selected: BuildSourceSelection, replay: Option<Delta>, limits: BaseCstAdmissionLimits,
        lifetime: Duration,
    ) -> Result<BuildSourceOpening<'_>> {
        self.open_recoverable_build_diagnostics_before(selected, replay, limits, lifetime, None, None).await
    }

    pub(super) async fn open_recoverable_build_diagnostics_before(
        &self, selected: BuildSourceSelection, replay: Option<Delta>, limits: BaseCstAdmissionLimits,
        lifetime: Duration, start_deadline: Option<Instant>, hard_deadline: Option<Instant>,
    ) -> Result<BuildSourceOpening<'_>> {
        let slot = self.try_slot()?;
        let root = selected.root;
        let outcome = self.spawn_reserved_query(slot, None, move |worker| async move {
            let selected = Arc::new(selected);
            let initial = Arc::clone(&selected);
            if let Some(lease) = worker.phase(None, move |store| {
                store.lease_build_source_with_cutoff(initial.as_ref().clone(), lifetime, start_deadline, hard_deadline)
            }).await? {
                return Ok::<_, super::Error>(Some((lease, None)));
            }
            let Some(replay) = replay else { return Ok(None) };
            let restoration = reconcile_on_worker(&worker, Arc::clone(&selected), Some(replay), limits).await?;
            let restored = match restoration {
                SourceReconciliation::Known { .. } => None,
                restored @ SourceReconciliation::Restored { .. } => Some(restored),
                SourceReconciliation::ReplayRequired { .. } => return Err("acknowledged replay was not admitted".into()),
            };
            let lease = worker.phase(None, move |store| {
                store.lease_build_source_with_cutoff(selected.as_ref().clone(), lifetime, start_deadline, hard_deadline)
            }).await?.ok_or("restored source is no longer available for a lease")?;
            Ok(Some((lease, restored)))
        }).await??;
        Ok(match outcome {
            None => BuildSourceOpening::ReplayRequired { root },
            Some((lease, restored)) => BuildSourceOpening::Opened {
                session: BuildDiagnosticSession { worker: self, lease: Arc::new(lease) }, restored,
            },
        })
    }
}

impl BuildDiagnosticSession<'_> {
    /// An expired or released lease cannot be renewed.
    pub fn renew(&self) -> bool { self.lease.renew() }

    pub(super) fn deadline(&self) -> Option<Instant> { self.lease.deadline() }

    pub(super) fn hard_deadline(&self) -> Option<Instant> { self.lease.hard_deadline() }

    pub(super) fn connection_id(&self) -> BaseDeclarationConnectionId {
        BaseDeclarationConnectionId(self.lease.ordinal())
    }

    /// Drop the client session; already started reads retain their own roots.
    pub fn release(self) {}

    /// Page through committed base-source pins of this leased workspace.
    /// No socket request, CLI routing or new retention authority.
    pub async fn source_pin_page(
        &self, cursor: Option<SourcePinPageCursor>, page_rows: NonZeroUsize,
        limits: BaseCstAdmissionLimits,
    ) -> Result<SourcePinPage> {
        if cursor.as_ref().is_some_and(|cursor| !cursor.is_for(&self.lease)) {
            return Err("source pin cursor belongs to another opened session".into());
        }
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| {
            store.leased_source_pin_page(&lease, cursor, page_rows, limits)
        }).await?
    }

    /// Match one bounded local intent page against committed mirror rows.
    /// A result is true only for the exact reflected intent in this read
    /// cut, in input order. It does not reconcile or confirm durability.
    pub async fn observe_source_pin_intents(
        &self, intents: Vec<SourcePinIntent>, limits: BaseCstAdmissionLimits,
    ) -> Result<Vec<bool>> {
        if intents.len() > super::pins::MAX_LIST_ROWS {
            return Err("source pin observation exceeds the page window".into());
        }
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| {
            store.observe_leased_pin_intents(&lease, &intents, limits)
        }).await?
    }

    pub(super) async fn project_base_declarations(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, declaration_bytes: usize,
    ) -> Result<PreparedBaseDeclarations> {
        self.worker.prepare_base_cst_with_lease(binding, proposal, limits, declaration_bytes,
            super::source_proposal::AdmissionLease::Connection(Arc::clone(&self.lease))).await
    }

    /// Examine one candidate with this local session's published-source lease.
    /// No framed action or live workspace configuration participates.
    pub async fn project_closed_comptime(
        &self, request: RootedClosedComptimeRequest,
    ) -> Result<RootedClosedComptimeProjection> {
        self.project_closed_comptime_in(request, SiteScope::AuthoredFile).await
    }

    /// Project just the first authored initialization of the actual target
    /// entry. A valid program reaching implicit main enters this binding
    /// before any authored predecessor. No general reachability or result
    /// authority, and no new framed or native command.
    pub async fn project_first_entry_comptime(
        &self, request: RootedClosedComptimeRequest,
    ) -> Result<RootedClosedComptimeProjection> {
        self.project_closed_comptime_in(request, SiteScope::FirstTargetInitializer).await
    }

    /// Derive one direct evaluator and its typed literal inputs from the
    /// actual leased singleton target entry. No framed action or runner.
    pub async fn project_direct_function_comptime(
        &self, request: RootedClosedComptimeRequest,
    ) -> Result<RootedDirectFunctionComptimeProjection> {
        request.check_selector_bytes()?;
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| {
            store.project_direct_function_comptime_for_lease(&lease, request)
        }).await?
    }

    /// Derive a bounded startup plan from the exact local singleton lease.
    /// The byte cap is aggregate LLVM egress, not total compiler memory.
    pub async fn project_direct_startup_comptime(
        &self, request: RootedClosedComptimeRequest,
    ) -> Result<RootedDirectStartupComptimeProjection> {
        self.project_startup_comptime_using(request, |store, lease, request| {
            store.project_direct_startup_comptime_for_lease(lease, request, derive_direct_startup_comptime_plan)
        }).await
    }

    /// The same local leased plan projection for two literal formals. Unequal
    /// typed tuples remain distinct even when their evaluator LLVM is equal.
    pub async fn project_direct_pair_startup_comptime(
        &self, request: RootedClosedComptimeRequest,
    ) -> Result<RootedDirectStartupComptimeProjection<2>> {
        self.project_startup_comptime_using(request, |store, lease, request| {
            store.project_direct_startup_comptime_for_lease(lease, request, derive_direct_pair_startup_comptime_plan)
        }).await
    }

    /// Inspect the verified one-hop forwarding plan on the local singleton
    /// lease, using the same aggregate text budget as direct startup.
    pub async fn project_forwarded_pair_startup_comptime(
        &self, request: RootedClosedComptimeRequest,
    ) -> Result<RootedForwardedPairStartupComptimeProjection> {
        self.project_startup_comptime_using(request, SourceStore::project_forwarded_pair_startup_comptime_for_lease).await
    }

    /// Prepare the exact already-committed singleton, including first Build.
    /// No synthetic predecessor, inherited tool result, or runtime execution.
    #[cfg(target_os = "linux")]
    pub async fn prepare_selected_pair_wasip1(
        &self, request: RootedClosedComptimeRequest, max_stdout: usize,
    ) -> Result<SelectedPairComptimePreparation<'_>> {
        let answer = self.project_startup_comptime_using(request, move |store, lease, request| {
            store.project_selected_pair_wasip1_for_lease(lease, request, max_stdout, false)
        }).await?;
        self.selected_pair_owner(answer)
    }

    /// Discover exactly one authored site in the authenticated selected
    /// singleton, without interpreting source text in the CLI.
    #[cfg(target_os = "linux")]
    pub async fn prepare_unique_selected_pair_wasip1(
        &self, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize, max_stdout: usize,
    ) -> Result<SelectedPairComptimePreparation<'_>> {
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        let answer = self.worker.spawn_reserved(slot, move |store| {
            store.project_unique_selected_pair_for_lease(&lease, profile, limits, output_bytes, max_stdout)
        }).await??;
        self.selected_pair_owner(answer)
    }

    #[cfg(target_os = "linux")]
    pub async fn prepare_selected_wasip1_comptime(
        &self, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize,
    ) -> Result<SelectedComptimePreparation<'_>> {
        use super::rooted_comptime::RootedSelectedComptime;
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        let answer = self.worker.spawn_reserved(slot, move |store| {
            store.project_selected_wasip1_for_lease(&lease, profile, limits, output_bytes)
        }).await??;
        Ok(match answer {
            Err(reason) => SelectedComptimePreparation::Refused(reason),
            Ok((configuration_inputs, plan)) => {
                self.lease.finish_current(|| ()).ok_or("selected comptime source lease expired")?;
                let execution_seal = wosy_project::packages::PreparedComptimeExecutionSeal::fresh();
                match plan {
                    RootedSelectedComptime::Pair(plan) => SelectedComptimePreparation::Pair(SelectedPairComptimeOwner {
                        lease: self.lease.as_ref(), configuration_inputs, plan, execution_seal,
                    }),
                    RootedSelectedComptime::Ordered(plan) => SelectedComptimePreparation::Ordered(SelectedOrderedComptimeOwner {
                        lease: self.lease.as_ref(), configuration_inputs, plan, execution_seal,
                    }),
                }
            }
        })
    }

    #[cfg(target_os = "linux")]
    fn selected_pair_owner(&self, answer: std::result::Result<(NodeId, super::RootedPairCallGraphComptimePlan),
        super::RootedClosedComptimeRefusal>) -> Result<SelectedPairComptimePreparation<'_>> {
        Ok(match answer {
            Err(reason) => SelectedPairComptimePreparation::Refused(reason),
            Ok((configuration_inputs, plan)) => {
                self.lease.finish_current(|| ()).ok_or("selected comptime source lease expired")?;
                SelectedPairComptimePreparation::Ready(SelectedPairComptimeOwner {
                    lease: self.lease.as_ref(), configuration_inputs, plan,
                    execution_seal: wosy_project::packages::PreparedComptimeExecutionSeal::fresh(),
                })
            }
        })
    }

    async fn project_startup_comptime_using<T: Send + 'static>(
        &self, request: RootedClosedComptimeRequest,
        project: impl FnOnce(&SourceStore, &SourceLease, RootedClosedComptimeRequest) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        request.check_selector_bytes()?;
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| project(store, &lease, request)).await?
    }

    async fn project_closed_comptime_in(
        &self, request: RootedClosedComptimeRequest, scope: SiteScope,
    ) -> Result<RootedClosedComptimeProjection> {
        request.check_selector_bytes()?;
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| {
            store.project_closed_comptime_for_lease(&lease, request, scope)
        }).await?
    }

    pub async fn dispatch(&self, request: BuildDiagnosticRequest) -> Result<BuildDiagnosticResponse<Diagnostic>> {
        let (_observer, cancellation) = QueryCancellation::observe_before(self.hard_deadline());
        self.dispatch_analysis(request, Some(cancellation)).await
    }

    /// Explicit local projection through a separate fresh ordinary Build
    /// analysis. The diagnostic-only framed command cannot request this.
    pub async fn project_ordinary_llvm(
        &self, profile: String, source_limits: BaseCstAdmissionLimits, output_bytes: usize,
    ) -> Result<RootedLlvmProjection> {
        self.project_llvm(profile, source_limits, output_bytes, RootedLlvmIdentity::Authored).await
    }

    /// Explicit local projection; Executable uses verified executable CST
    /// identity and the worker's Tokio time driver for shared-flight waiting.
    /// The ordinary CLI, exported labels and wire stay unchanged.
    pub async fn project_llvm(
        &self, profile: String, source_limits: BaseCstAdmissionLimits, output_bytes: usize,
        identity: RootedLlvmIdentity,
    ) -> Result<RootedLlvmProjection> {
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        if identity == RootedLlvmIdentity::Executable {
            let (_observer, cancellation) = QueryCancellation::observe();
            return self.worker.spawn_reserved_query(slot, Some(cancellation), move |worker| {
                super::rooted_llvm::project_executable_on_worker(worker, lease, profile, source_limits, output_bytes)
            }).await?;
        }
        self.worker.spawn_reserved(slot, move |store| {
            store.project_ll_for_lease(&lease, profile, source_limits, output_bytes, identity)
        }).await?
    }

    pub(super) async fn project_ordinary_llvm_framed(
        &self, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize, builder_inputs: bool, executable: bool,
    ) -> Result<(RootedLlvmProjection, Option<OrdinaryLlvmBuilderWitness>)> {
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        let (_observer, cancellation) = QueryCancellation::observe_before(self.hard_deadline());
        self.worker.spawn_reserved_query(slot, Some(cancellation.clone()), move |worker| async move {
            worker.phase(None, move |store| store.project_framed_ll(&lease, profile, limits,
                output_bytes, builder_inputs, executable, || QueryCancellation::check(Some(&cancellation)))).await
        }).await?
    }

    pub(super) async fn dispatch_framed(&self, request: BuildDiagnosticRequest) -> Result<BuildDiagnosticResponse<Diagnostic>> {
        self.dispatch(request).await
    }

    /// Save the actual ordinary Build diagnostic outcome for this exact
    /// leased revision. An archive is historical evidence, never a reusable
    /// Build/Check verdict, artifact or caller-supplied diagnostic.
    pub async fn save_build_diagnostic(
        &self, profile: &str, name: &str, limits: BaseCstAdmissionLimits, max_report_bytes: usize,
    ) -> Result<SavedBuildDiagnostic> {
        super::saved_diagnostics::validate_name(name)?;
        super::saved_diagnostics::validate_profile(profile, max_report_bytes)?;
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        let (_observer, cancellation) = QueryCancellation::observe_before(self.hard_deadline());
        let (profile, name) = (profile.to_owned(), name.to_owned());
        self.worker.spawn_reserved_query(slot, Some(cancellation), move |worker| async move {
            super::saved_diagnostics::save(&worker, lease, &name, profile, limits, max_report_bytes).await
        }).await?
    }

    /// Read a named historical diagnostic belonging to this lease's
    /// workspace, using the archive's exact authored revision.
    pub async fn saved_build_diagnostic(
        &self, name: &str, limits: BaseCstAdmissionLimits, max_report_bytes: usize,
    ) -> Result<Option<SavedBuildDiagnostic>> {
        self.read_saved_build_diagnostic(name, limits, max_report_bytes, false).await
            .map(|view| view.map(SavedBuildDiagnosticView::into_report))
    }

    /// Read the same archive with its bounded, verified authored label
    /// sources. A historical root need not be (or become) an active snapshot.
    pub async fn saved_build_diagnostic_view(
        &self, name: &str, limits: BaseCstAdmissionLimits, max_report_bytes: usize,
    ) -> Result<Option<SavedBuildDiagnosticView>> {
        self.read_saved_build_diagnostic(name, limits, max_report_bytes, true).await
    }

    async fn read_saved_build_diagnostic(
        &self, name: &str, limits: BaseCstAdmissionLimits, max_report_bytes: usize,
        include_authored: bool,
    ) -> Result<Option<SavedBuildDiagnosticView>> {
        super::saved_diagnostics::validate_name(name)?;
        let slot = self.worker.try_slot()?;
        let name = name.to_owned();
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| {
            super::saved_diagnostics::read(store, &lease, &name, limits, max_report_bytes, include_authored)
        }).await?
    }

    async fn dispatch_analysis(
        &self, request: BuildDiagnosticRequest, cancellation: Option<QueryCancellation>,
    ) -> Result<BuildDiagnosticResponse<Diagnostic>> {
        let BuildDiagnosticRequest::BuildDiagnostics { version, profile } = request;
        if version != BUILD_DIAGNOSTIC_VERSION { return Err(BuildRequestRejection::Version.into()); }
        if let Some(signal) = &cancellation { signal.check_cutoff()?; }
        let slot = self.worker.try_slot()?;
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved_query(slot, cancellation, move |worker| async move {
            let analysis = analyze_leased(&worker, &lease, &profile).await?;
            Ok(BuildDiagnosticResponse::BuildDiagnostics {
                version, source_root: analysis.source_root, profile,
                preparation: analysis.preparation, receipt: analysis.key.digest(),
                phase: analysis.phase, origin: analysis.origin, diagnostics: analysis.diagnostics,
            })
        }).await?
    }
}

async fn analyze_leased(worker: &QueryWorker, lease: &SourceLease, profile: &str) -> Result<RootedBuildAnalysis> {
    prepare_leased(worker, lease, profile).await?.build_analysis_with(Some(worker)).await
}

async fn prepare_leased<'a>(
    worker: &'a QueryWorker, lease: &SourceLease, profile: &str,
) -> Result<RootedPreparedProject<'a>> {
    let grant = lease.grant();
    let snapshot = worker.phase(None, move |store| {
        store.leased_source_snapshot(grant)?.map(|source| source.configured()).transpose()
    }).await?;
    let Some(snapshot) = snapshot else { return Err(BuildRequestRejection::Expired.into()); };
    // SurrealKV commits can schedule blocking work; await on the async task.
    worker.store.prepare_configured(snapshot, profile, Some(worker), Some(lease)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildAnalysisOrigin, SourceStore, StorageAdmissionError};
    use wosy_protocol::NodeId;
    use std::future::{poll_fn, Future};
    use std::num::NonZeroUsize;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc};
    use std::task::Poll;

    #[test]
    #[ignore = "requires two published revisions of a small legitimate workspace"]
    fn leased_resolution_reuses_only_compiler_owned_exact_positive_state() {
        use wosy_compiler::{analyze_scalar_build, ScalarProject};
        let project = PathBuf::from(std::env::var_os("WOSY_RESOLUTION_PROJECT").unwrap());
        let store_path = PathBuf::from(std::env::var_os("WOSY_RESOLUTION_STORE").unwrap());
        let root = |field| NodeId::parse_hex(&std::env::var(field).unwrap()).unwrap();
        let first = root("WOSY_RESOLUTION_FIRST");
        let second = root("WOSY_RESOLUTION_ANNOTATED");
        assert_ne!(first, second);
        let fresh: Vec<_> = std::env::var("WOSY_RESOLUTION_PRESSURE").unwrap().split(',')
            .map(|hex| NodeId::parse_hex(hex).unwrap()).collect();
        assert_eq!(fresh.len(), 5);
        let oversized = root("WOSY_RESOLUTION_OVERSIZED");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        runtime.block_on(async {
            for reopen in [false, true] {
                let store = Arc::new(SourceStore::open(&store_path, 8 << 20).unwrap());
                let one = NonZeroUsize::new(1).unwrap();
                let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
                let selection = |root| BuildSourceSelection { root, project: project.to_str().unwrap().into(), target: "app".into() };
                let open = |root| worker.open_build_diagnostics(selection(root), Duration::from_secs(60));

                async fn query(session: &BuildDiagnosticSession<'_>, profile: &'static str)
                    -> Result<(bool, bool, ScalarProject, Vec<Diagnostic>, Duration)> {
                    let slot = session.worker.try_slot()?;
                    let lease = Arc::clone(&session.lease);
                    let (_observer, signal) = QueryCancellation::observe_before(session.hard_deadline());
                    session.worker.spawn_reserved_query(slot, Some(signal), move |worker| async move {
                        let view = prepare_leased(&worker, &lease, profile).await?;
                        let prepared = view.prepared().ok_or("expected a positively prepared compiler project")?;
                        Ok((view.report.reused_live_resolution, view.report.reused_live_preparation,
                            prepared.project().clone(), analyze_scalar_build(prepared).diagnostics().to_vec(),
                            view.report.compiler_preparation + view.report.materialized_bodies))
                    }).await?
                }

                let a = open(first).await.unwrap().unwrap();
                let b = open(second).await.unwrap().unwrap();
                for profile in ["dev", "native"] {
                    let (hit, live, project, diagnostics, cold_time) = query(&a, profile).await.unwrap();
                    assert!(!hit && !live && diagnostics.is_empty());
                    let (hit, live, same, repeated, warm_time) = query(&a, profile).await.unwrap();
                    assert!(hit && !live);
                    assert_eq!((same, repeated), (project.clone(), diagnostics.clone()));
                    let plain = store.rooted_prepared_project(first, profile).await.unwrap().unwrap();
                    assert!(!plain.report.reused_live_resolution);
                    let ordinary = plain.prepared().unwrap();
                    assert_eq!(ordinary.project(), &project);
                    assert_eq!(analyze_scalar_build(ordinary).diagnostics(), diagnostics);
                    drop(plain);
                    let (hit, live, changed, rederived, _) = query(&b, profile).await.unwrap();
                    // The exact authored root changed. Its executable semantics
                    // match, but no previous-root positive was consulted.
                    assert!(!hit && !live);
                    assert_eq!((changed, rederived), (project, diagnostics));
                    let another = open(first).await.unwrap().unwrap();
                    let (hit, live, _, _, _) = query(&another, profile).await.unwrap();
                    assert!(hit && !live);
                    another.lease.release();
                    let error = query(&another, profile).await.unwrap_err();
                    assert!(matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)));
                    println!("{profile} reopen={reopen}: fresh={}µs handoff={}µs",
                        cold_time.as_micros(), warm_time.as_micros());
                }
                for &root in &fresh {
                    let extra = open(root).await.unwrap().unwrap();
                    let (hit, live, _, diagnostics, _) = query(&extra, "dev").await.unwrap();
                    assert!(!hit && !live && diagnostics.is_empty());
                }
                // Initial slots were A.dev, B.dev, A.native, B.native.
                // Five later inserts evict only A.dev despite its warm reads.
                assert!(query(&a, "native").await.unwrap().0);
                assert!(!query(&a, "dev").await.unwrap().0);
                let big = open(oversized).await.unwrap().unwrap();
                for _ in 0..2 {
                    let (hit, live, _, diagnostics, _) = query(&big, "dev").await.unwrap();
                    assert!(!hit && !live && diagnostics.is_empty());
                }
                big.release();
                let slot = a.worker.try_slot().unwrap();
                let (observer, signal) = QueryCancellation::observe();
                drop(observer);
                let open_lease = Arc::clone(&a.lease);
                let stopped = a.worker.spawn_reserved_query(slot, Some(signal), move |worker| async move {
                    // A detached observer cannot warm-access another valid
                    // session's already-retained project preparation.
                    prepare_leased(&worker, &open_lease, "native").await.map(|_| ())
                }).await.unwrap();
                assert!(stopped.is_err());
                assert!(query(&a, "native").await.unwrap().0);
                println!("genuine eighth/ninth FIFO, oversize bypass and observer refusal; reopen={reopen}");
                a.release();
                b.release();
                drop(worker);
                Arc::into_inner(store).ok_or("worker still held").unwrap().close().await.unwrap();
            }
        });
    }

    #[test]
    #[ignore = "requires a disposable store with a published source root"]
    fn cancelled_queued_session_does_not_publish_and_shares_worker_capacity() {
        let path = PathBuf::from(std::env::var_os("WOSY_TEST_IMAGE_STORE").unwrap());
        let root = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").unwrap()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().max_blocking_threads(1).build().unwrap();
        let handle = runtime.handle().clone();
        runtime.block_on(async {
            let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
            let bound = |value| NonZeroUsize::new(value).unwrap();
            let worker = SourceStorageWorker::new(
                Arc::clone(&store), handle.clone(), bound(1), bound(1), bound(20_000),
            ).unwrap();
            let workspace = path.parent().unwrap().join("project").canonicalize().unwrap();
            let selected = BuildSourceSelection { root, project: workspace.to_str().unwrap().into(), target: "app".into() };
            assert!(worker.open_build_diagnostics(BuildSourceSelection { root: NodeId([91; 32]), ..selected.clone() },
                Duration::from_secs(60)).await.unwrap().is_none());
            let open = || worker.open_build_diagnostics(selected.clone(), Duration::from_secs(60));
            let session = open().await.unwrap().unwrap();
            let request = || BuildDiagnosticRequest::BuildDiagnostics {
                version: BUILD_DIAGNOSTIC_VERSION, profile: "dev".into(),
            };
            let bad_version = session.dispatch(BuildDiagnosticRequest::BuildDiagnostics {
                version: BUILD_DIAGNOSTIC_VERSION + 1, profile: "dev".into(),
            }).await.unwrap_err();
            assert_eq!(bad_version.to_string(), "unsupported Build diagnostic protocol version");

            // Hold the actual blocking worker while a request waits for the
            // sole admitted slot. This is a fixture barrier, not a second queue.
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, blocked) = mpsc::channel();
            let barrier = handle.spawn_blocking(move || {
                entered.send(()).unwrap();
                blocked.recv().unwrap();
            });
            waiting.await.unwrap();
            let mut pending = Box::pin(session.dispatch(request()));
            assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await);
            assert!(matches!(worker.try_inspect(root, &[], &[]), Err(StorageAdmissionError::Busy)));
            drop(pending);
            session.release();
            assert!(matches!(worker.try_inspect(root, &[], &[]), Err(StorageAdmissionError::Busy)));
            release.send(()).unwrap();
            barrier.await.unwrap();
            let fresh = loop {
                match open().await {
                    Ok(Some(session)) => break session,
                    Err(error) if error.downcast_ref::<StorageAdmissionError>() == Some(&StorageAdmissionError::Busy) => {
                        tokio::task::yield_now().await;
                    }
                    _ => panic!("expected a new lease after queue drain"),
                }
            };
            assert!(fresh.dispatch(BuildDiagnosticRequest::BuildDiagnostics {
                version: BUILD_DIAGNOSTIC_VERSION, profile: "not-configured".into(),
            }).await.is_err());
            let response = fresh.dispatch(request()).await.unwrap();
            let BuildDiagnosticResponse::BuildDiagnostics { source_root, origin, .. } = response;
            assert_eq!((source_root, origin), (root, BuildAnalysisOrigin::Derived));
            fresh.release();
            drop(worker);
            Arc::try_unwrap(store).ok().expect("owned store").close().await.unwrap();
        });
    }
}

#[derive(Debug)]
pub(super) enum BuildRequestRejection {
    Version,
    Expired,
}

impl std::fmt::Display for BuildRequestRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Version => "unsupported Build diagnostic protocol version",
            Self::Expired => "Build diagnostic session lease expired",
        })
    }
}

impl std::error::Error for BuildRequestRejection {}
