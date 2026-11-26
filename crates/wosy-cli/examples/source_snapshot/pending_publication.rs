use super::{leased_build, milliseconds, source_reconciliation::admission_limits, Result};
use std::{num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_cli::{
    source_recovery::ClientSourceLimits,
    source_state::{PendingBasePublication, SqliteSourceState},
};
use wosy_daemon::{SourceReconciliation, SourceStorageWorker, SourceStore};
use wosy_protocol::{FileKey, SourceDeltaPublication};
use wosy_project::PackageRelativePath;

pub(super) async fn run(
    local: &mut SqliteSourceState, project: &Path, path: &Path, target: &str, profile: &str, restage: bool,
) -> Result<serde_json::Value> {
    let prior = local.acknowledged_build_source(target)?.ok_or("fixture needs an acknowledged predecessor")?;
    let first = local.selection(target)?.pending.ok_or("fixture needs a pending edit")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let lifetime = Duration::from_secs(30);
    let limits = admission_limits(false);
    let unchanged = |local: &SqliteSourceState| -> Result<()> {
        let state = local.selection(target)?;
        assert_eq!(state.selected, Some(prior.root));
        assert_eq!(state.pending, Some(first));
        Ok(())
    };
    assert!(matches!(local.publish_pending_base(target, first, |binding, proposal| {
        worker.publish_bound_base_source(binding, proposal, limits, lifetime)
    }).await?, PendingBasePublication::PredecessorMissing));
    unchanged(local)?;
    assert!(store.snapshot(first.root())?.is_none());

    // Reuse the already acknowledged predecessor recovery path, not a
    // second publication mechanism or implicitly refreshed live source.
    let replay = local.replay_acknowledged_source(&prior)?.ok_or("fixture predecessor changed")?;
    assert_eq!(replay.source.parsed, 0);
    assert!(matches!(worker.reconcile_acknowledged_source(prior.clone(), Some(replay.source.delta), limits).await?,
        SourceReconciliation::Restored { .. }));
    let mut tight = limits;
    tight.authored_bytes = 1;
    assert!(local.publish_pending_base(target, first, |binding, proposal| {
        worker.publish_bound_base_source(binding, proposal, tight, lifetime)
    }).await.is_err());
    unchanged(local)?;
    assert!(store.snapshot(first.root())?.is_none());

    let start = Instant::now();
    let receipt = if restage {
        let mut concurrent = SqliteSourceState::open(project, ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 })?;
        let current_store = &store;
        let service = &worker;
        let outcome = local.publish_pending_base(target, first, |binding, proposal| async move {
            let result = service.publish_bound_base_source(binding, proposal, limits, lifetime).await?;
            let SourceDeltaPublication::Committed(receipt) = &result else {
                return Err("fixture lost the admitted predecessor".into());
            };
            assert_eq!(receipt.validated_files, 1);
            assert!(!receipt.already_present && receipt.written_records > 0);
            assert!(current_store.snapshot(first.root())?.is_some());
            assert!(concurrent.abandon(target, first)?);
            let newer = concurrent.stage(target)?;
            assert_eq!((newer.root, newer.parsed, newer.reused), (first.root(), 1, 2));
            assert!(newer.pending.unwrap().generation() > first.generation());
            Ok(result)
        }).await?;
        assert!(matches!(outcome, PendingBasePublication::StaleAttempt));
        let state = local.selection(target)?;
        assert_eq!(state.selected, Some(prior.root));
        let current = state.pending.ok_or("fixture newer attempt disappeared")?;
        assert_eq!(current.root(), first.root());
        let PendingBasePublication::Applied(receipt) = local.publish_pending_base(target, current, |binding, proposal| {
            worker.publish_bound_base_source(binding, proposal, limits, lifetime)
        }).await? else { return Err("fixture expected the later attempt to apply".into()); };
        assert!(receipt.already_present && receipt.written_records == 0);
        receipt
    } else {
        let PendingBasePublication::Applied(receipt) = local.publish_pending_base(target, first, |binding, proposal| {
            worker.publish_bound_base_source(binding, proposal, limits, lifetime)
        }).await? else { return Err("fixture expected the admitted update to apply".into()); };
        assert!(receipt.written_records > 0 && !receipt.already_present);
        receipt
    };
    let publication_ms = milliseconds(start);
    assert_eq!(receipt.validated_files, 1);
    assert_eq!(receipt.binding.proposed, first.root());
    assert_eq!(receipt.binding.selected, prior);
    assert_eq!(local.selection(target)?.pending, None);
    let activated = local.capture_build_diagnostics(target, profile)?.ok_or("fixture omitted current diagnostics")?;
    assert_eq!((activated.selection().root, activated.generation()), (first.root(), receipt.binding.attempt));
    let key = FileKey { package: "std".into(), path: PackageRelativePath::new("src/leaf.w".into())? };
    let authored = |root| -> Result<String> {
        Ok(store.snapshot(root)?.ok_or("fixture lost published source")?.authored(&key)?.cst.root.text().to_string())
    };
    let before = authored(prior.root)?;
    let after = authored(first.root())?;
    assert_ne!(before, after);
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    let presentation = leased_build::report(local, path, target, profile, false).await?;
    Ok(serde_json::json!({
        "root": receipt.binding.proposed.hex(), "attempt": receipt.binding.attempt,
        "validated_files": receipt.validated_files, "written_records": receipt.written_records,
        "already_present": receipt.already_present, "conflicts": receipt.conflicts,
        "stale_receipt_refused": restage, "previous_source": before, "next_source": after,
        "publication_ms": publication_ms, "presentation": presentation,
    }))
}
