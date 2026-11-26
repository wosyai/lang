//! Product dispatcher for the leased, diagnostic-only Build projection.
use super::{
    retention_plan::SourceLease, BaseCstAdmissionLimits, PreparedBaseDeclarations, Result, SourceStorageWorker,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_compiler::Diagnostic;
use wosy_protocol::{
    BaseCstProposal, BaseCstProposalBinding, BaseDeclarationConnectionId, BuildDiagnosticRequest, BuildDiagnosticResponse,
    BuildSourceSelection, BUILD_DIAGNOSTIC_VERSION,
};

/// Bound to one published source selected by a trusted local adapter. No
/// index, compiler artifact or raw node access can be requested by the client.
pub struct BuildDiagnosticSession<'a> {
    worker: &'a SourceStorageWorker,
    lease: Arc<SourceLease>,
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
        let slot = self.try_slot()?;
        let lease = self.spawn_reserved(slot, move |store| store.lease_build_source(selected, lifetime))
            .await??;
        Ok(lease.map(|lease| BuildDiagnosticSession { worker: self, lease: Arc::new(lease) }))
    }
}

impl BuildDiagnosticSession<'_> {
    /// An expired or released lease cannot be renewed.
    pub fn renew(&self) -> bool { self.lease.renew() }

    pub(super) fn deadline(&self) -> Option<Instant> { self.lease.deadline() }

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
        let BuildDiagnosticRequest::BuildDiagnostics { version, profile } = request;
        if version != BUILD_DIAGNOSTIC_VERSION { return Err(BuildRequestRejection::Version.into()); }
        let slot = self.worker.try_slot()?;
        let grant = self.lease.grant();
        self.worker.spawn_reserved_query(slot, move |worker| async move {
            let snapshot = worker.phase(None, move |store| {
                store.leased_source_snapshot(grant)?.map(|source| source.configured()).transpose()
            }).await?;
            let Some(snapshot) = snapshot else { return Err(BuildRequestRejection::Expired.into()); };
            // SurrealKV commits may schedule their own blocking work. Keep
            // all awaited queries on the async task, never on a pool thread.
            let prepared = worker.store.prepare_configured(snapshot, &profile, Some(&worker)).await?;
            let analysis = prepared.build_analysis_with(Some(&worker)).await?;
            Ok(BuildDiagnosticResponse::BuildDiagnostics {
                version, source_root: analysis.source_root, profile,
                preparation: analysis.preparation, receipt: analysis.key.digest(),
                phase: analysis.phase, origin: analysis.origin, diagnostics: analysis.diagnostics,
            })
        }).await?
    }
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
