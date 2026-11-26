//! Product dispatcher for the leased, diagnostic-only Build projection.
use super::{
    retention_plan::{LeaseGrant, SourceLease}, source_reconciliation::reconcile_on_worker, BaseCstAdmissionLimits,
    storage_worker::{QueryCancellation, QueryWorker},
    PreparedBaseDeclarations, Result, RootedBuildAnalysis, RootedLlvmIdentity, RootedLlvmProjection,
    SavedBuildDiagnostic, SourceReconciliation, SourceStorageWorker,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_compiler::Diagnostic;
use wosy_project::packages::InstalledGitBase;
use wosy_protocol::{
    BaseCstProposal, BaseCstProposalBinding, BaseDeclarationConnectionId, BuildDiagnosticRequest, BuildDiagnosticResponse,
    BuildSourceSelection, Delta, NodeId, BUILD_DIAGNOSTIC_VERSION,
};

/// Bound to one published source selected by a trusted local adapter. No
/// index, compiler artifact or raw node access can be requested by the client.
pub struct BuildDiagnosticSession<'a> {
    worker: &'a SourceStorageWorker,
    lease: Arc<SourceLease>,
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

    pub(super) async fn project_base_declarations(
        &self, binding: BaseCstProposalBinding, proposal: BaseCstProposal,
        limits: BaseCstAdmissionLimits, declaration_bytes: usize,
    ) -> Result<PreparedBaseDeclarations> {
        self.worker.prepare_base_cst_with_lease(binding, proposal, limits, declaration_bytes,
            super::source_proposal::AdmissionLease::Connection(Arc::clone(&self.lease))).await
    }

    pub async fn dispatch(&self, request: BuildDiagnosticRequest) -> Result<BuildDiagnosticResponse<Diagnostic>> {
        self.dispatch_analysis(request, None).await
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

    pub(super) async fn dispatch_framed(&self, request: BuildDiagnosticRequest) -> Result<BuildDiagnosticResponse<Diagnostic>> {
        let (_observer, cancellation) = QueryCancellation::observe_before(self.hard_deadline());
        self.dispatch_analysis(request, Some(cancellation)).await
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
        super::saved_diagnostics::validate_name(name)?;
        let slot = self.worker.try_slot()?;
        let name = name.to_owned();
        let lease = Arc::clone(&self.lease);
        self.worker.spawn_reserved(slot, move |store| {
            super::saved_diagnostics::read(store, &lease, &name, limits, max_report_bytes)
        }).await?
    }

    async fn dispatch_analysis(
        &self, request: BuildDiagnosticRequest, cancellation: Option<QueryCancellation>,
    ) -> Result<BuildDiagnosticResponse<Diagnostic>> {
        let BuildDiagnosticRequest::BuildDiagnostics { version, profile } = request;
        if version != BUILD_DIAGNOSTIC_VERSION { return Err(BuildRequestRejection::Version.into()); }
        if let Some(signal) = &cancellation { signal.check_cutoff()?; }
        let slot = self.worker.try_slot()?;
        let grant = self.lease.grant();
        self.worker.spawn_reserved_query(slot, cancellation, move |worker| async move {
            let analysis = analyze_leased(&worker, grant, &profile).await?;
            Ok(BuildDiagnosticResponse::BuildDiagnostics {
                version, source_root: analysis.source_root, profile,
                preparation: analysis.preparation, receipt: analysis.key.digest(),
                phase: analysis.phase, origin: analysis.origin, diagnostics: analysis.diagnostics,
            })
        }).await?
    }
}

async fn analyze_leased(worker: &QueryWorker, grant: LeaseGrant, profile: &str) -> Result<RootedBuildAnalysis> {
    let snapshot = worker.phase(None, move |store| {
        store.leased_source_snapshot(grant)?.map(|source| source.configured()).transpose()
    }).await?;
    let Some(snapshot) = snapshot else { return Err(BuildRequestRejection::Expired.into()); };
    // SurrealKV commits can schedule blocking work; await on the async task.
    let prepared = worker.store.prepare_configured(snapshot, profile, Some(worker)).await?;
    prepared.build_analysis_with(Some(worker)).await
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
