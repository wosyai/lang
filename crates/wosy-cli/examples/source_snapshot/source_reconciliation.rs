use super::{milliseconds, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_cli::source_state::SqliteSourceState;
use wosy_daemon::{BaseCstAdmissionLimits, BuildSourceOpening, SourceReconciliation, SourceStorageWorker, SourceStore};
use wosy_protocol::{BuildDiagnosticRequest, BuildDiagnosticResponse, BUILD_DIAGNOSTIC_VERSION};

pub(super) fn admission_limits(limited: bool) -> BaseCstAdmissionLimits {
    BaseCstAdmissionLimits {
        records: 25_000, record_bytes: 16 << 20, graph_edges: 100_000,
        expanded_record_bytes: 32 << 20, cst_occurrences: 100_000, cst_depth: 128,
        authored_bytes: if limited { 0 } else { 2 << 20 },
    }
}

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, limited: bool,
) -> Result<serde_json::Value> {
    let selected = client.acknowledged_build_source(target)?.ok_or("no acknowledged source for this target")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let number = |u| NonZeroUsize::new(u).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), number(2), number(8), number(50_000),
    )?;
    let limits = admission_limits(limited);
    let start = Instant::now();
    let probe = worker.reconcile_acknowledged_source(selected.clone(), None, limits).await?;
    let probe_ms = milliseconds(start);
    let result = match probe {
        SourceReconciliation::Known { root } => serde_json::json!({
            "kind": "known", "root": root.hex(), "probe_ms": probe_ms, "client_replayed": false,
            "daemon_validated_files": 0, "written": 0,
        }),
        SourceReconciliation::ReplayRequired { root } => {
            assert!(store.snapshot(root)?.is_none());
            let start = Instant::now();
            let replay = client.replay_acknowledged_source(&selected)?.ok_or("acknowledged selection changed")?;
            let replay_ms = milliseconds(start);
            let start = Instant::now();
            let outcome = worker.reconcile_acknowledged_source(selected, Some(replay.source.delta), limits).await;
            let recover_ms = milliseconds(start);
            if outcome.is_err() { assert!(store.snapshot(root)?.is_none()); }
            match outcome? {
                SourceReconciliation::Restored { root, report, validated_files, conflicts } => serde_json::json!({
                    "kind": "restored", "root": root.hex(), "probe_ms": probe_ms, "replay_ms": replay_ms,
                    "recover_ms": recover_ms, "client_replayed": true, "client_parsed": replay.source.parsed,
                    "daemon_validated_files": validated_files, "written": report.written,
                    "namespace_derived": report.namespace_derived, "conflicts": conflicts,
                }),
                _ => return Err("fixture needed an actual restoration".into()),
            }
        }
        _ => return Err("a probe cannot restore data".into()),
    };
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns source store").close().await?;
    Ok(result)
}

pub(super) async fn run_build(
    client: &mut SqliteSourceState, path: &Path, target: &str, profile: &str, mode: &str,
) -> Result<serde_json::Value> {
    let selected = client.acknowledged_build_source(target)?.ok_or("no acknowledged source for this target")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let number = |u| NonZeroUsize::new(u).unwrap();
    // A single slot prevents nested admission of a second source job.
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), number(1), number(8), number(50_000),
    )?;
    let limits = admission_limits(mode.ends_with("-limited"));
    let lifetime = Duration::from_secs(30);
    let root = selected.root;
    let probe_time = Instant::now();
    let mut recovery = None;
    let (session, probe_ms) = match worker.open_recoverable_build_diagnostics(selected.clone(), None, limits, lifetime).await? {
        BuildSourceOpening::Opened { session, restored: None } => (session, milliseconds(probe_time)),
        BuildSourceOpening::ReplayRequired { root: missing } => {
            let probe_ms = milliseconds(probe_time);
            assert_eq!(missing, root);
            assert!(store.snapshot(root)?.is_none());
            let replay_time = Instant::now();
            let replay = client.replay_acknowledged_source(&selected)?.ok_or("acknowledged selection changed")?;
            let replay_ms = milliseconds(replay_time);
            assert_eq!(replay.source.parsed, 0);
            let restore_time = Instant::now();
            let opening = worker.open_recoverable_build_diagnostics(
                selected.clone(), Some(replay.source.delta), limits, lifetime,
            ).await;
            let restore_and_open_ms = milliseconds(restore_time);
            if opening.is_err() { assert!(store.snapshot(root)?.is_none()); }
            let BuildSourceOpening::Opened {
                session, restored: Some(SourceReconciliation::Restored { root: recovered, report, validated_files, conflicts }),
            } = opening? else { return Err("fixture did not restore and lease its source".into()); };
            assert_eq!(recovered, root);
            recovery = Some(serde_json::json!({
                "replay_ms": replay_ms, "restore_and_open_ms": restore_and_open_ms,
                "written": report.written, "validated_files": validated_files, "conflicts": conflicts,
            }));
            (session, probe_ms)
        }
        _ => return Err("Build source probe should not restore a payload".into()),
    };
    assert!(session.renew());
    let dispatch_time = Instant::now();
    let response = session.dispatch(BuildDiagnosticRequest::BuildDiagnostics {
        version: BUILD_DIAGNOSTIC_VERSION, profile: profile.to_owned(),
    }).await?;
    let dispatch_ms = milliseconds(dispatch_time);
    let BuildDiagnosticResponse::BuildDiagnostics { source_root, .. } = &response;
    assert_eq!(*source_root, root);
    session.release();

    let mut quota = false;
    if mode.ends_with("-quota") {
        let mut sessions = Vec::new();
        for _ in 0..64 {
            sessions.push(worker.open_build_diagnostics(selected.clone(), lifetime).await?.ok_or("expected a lease")?);
        }
        let failure = worker.open_recoverable_build_diagnostics(selected.clone(), None, limits, lifetime).await;
        let Err(error) = failure else { return Err("recovery opening bypassed the existing lease quota".into()); };
        assert_eq!(error.to_string(), "Build diagnostic session quota exceeded");
        sessions.pop().expect("a held session").release();
        let BuildSourceOpening::Opened { session, restored: None } = worker.open_recoverable_build_diagnostics(
            selected.clone(), None, limits, lifetime,
        ).await? else { return Err("release did not restore ordinary lease capacity".into()); };
        session.release();
        drop(sessions);
        let mut wrong = selected.clone();
        wrong.project = path.to_string_lossy().into_owned();
        let result = worker.open_recoverable_build_diagnostics(wrong, None, limits, lifetime).await;
        let Err(error) = result else { return Err("opened a source for a different workspace".into()); };
        assert_eq!(error.to_string(), "Build diagnostic selection has a different workspace");
        let mut wrong = selected;
        wrong.target.push_str("-not-selected");
        let result = worker.open_recoverable_build_diagnostics(wrong, None, limits, lifetime).await;
        let Err(error) = result else { return Err("opened a source for a different target".into()); };
        assert_eq!(error.to_string(), "Build diagnostic selection has a different target");
        quota = true;
    }
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns source store").close().await?;
    Ok(serde_json::json!({
        "probe_ms": probe_ms, "recovery": recovery, "dispatch_ms": dispatch_ms,
        "lease_quota_and_scope": quota, "response": response,
    }))
}
