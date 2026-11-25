use super::{milliseconds, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use wosy_daemon::{SourceStorageWorker, SourceStore, StorageAdmissionError};
use wosy_protocol::NodeId;

pub(super) fn run(path: &Path, scope: NodeId, old: NodeId) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1).enable_time().build()?;
    let handle = runtime.handle().clone();
    runtime.block_on(async {
        let store = Arc::new(SourceStore::open(path, 8 << 20)?);
        let bound = |value| NonZeroUsize::new(value).unwrap();
        let worker = SourceStorageWorker::new(
            Arc::clone(&store), handle.clone(), bound(1), bound(1), bound(20_000),
        )?;
        let start = Instant::now();
        let direct = store.inspect_source_retention(scope, &[old], &[], 20_000)?;
        let direct_ms = milliseconds(start);
        assert!(direct.plan.candidate_only_nodes > 0 && direct.candidate_only.referenced > 0);
        assert!(matches!(
            worker.try_inspect(scope, &[old], &[scope]),
            Err(StorageAdmissionError::SelectionOverBudget)
        ));

        // Deterministically occupy the fixture's only blocking thread before
        // admission. Nothing about this barrier is part of the service.
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, blocked) = mpsc::channel();
        let barrier = handle.spawn_blocking(move || {
            let _ = entered.send(());
            let _ = blocked.recv();
        });
        tokio::time::timeout(Duration::from_secs(15), waiting).await??;
        let cancelled = worker.try_inspect(scope, &[old], &[])?;
        assert!(matches!(worker.try_inspect(scope, &[old], &[]), Err(StorageAdmissionError::Busy)));
        // Poll an actual waiter, then cancel it while its job is still queued.
        let mut observer = Box::pin(cancelled.finish());
        assert!(std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(observer.as_mut(), cx).is_pending())
        }).await);
        drop(observer);
        assert!(matches!(worker.try_inspect(scope, &[old], &[]), Err(StorageAdmissionError::Busy)));
        let heartbeat = tokio::spawn(async {
            tokio::task::yield_now().await;
            true
        }).await?;
        release.send(())?;
        barrier.await?;

        let start = Instant::now();
        let completed = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match worker.try_inspect(scope, &[old], &[]) {
                    Ok(job) => break job.finish().await,
                    Err(StorageAdmissionError::Busy) => tokio::task::yield_now().await,
                    Err(error) => break Err(error.into()),
                }
            }
        }).await??;
        let released_ms = milliseconds(start);
        assert_eq!(completed.inventory, direct);
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let again = store.inspect_source_retention(scope, &[old], &[], 20_000)?;
            let direct_ms = milliseconds(start);
            let worker = worker.try_inspect(scope, &[old], &[])?.finish().await?;
            assert_eq!(worker.inventory, again);
            samples.push([direct_ms, worker.queued.as_secs_f64() * 1_000.0,
                          worker.work.as_secs_f64() * 1_000.0]);
        }
        let small = SourceStorageWorker::new(
            Arc::clone(&store), handle, bound(1), bound(1), bound(2),
        )?;
        let budget_error = small.try_inspect(scope, &[old], &[])?.finish().await
            .err().ok_or("expected fixture node-budget rejection")?.to_string();
        assert_eq!(budget_error, "source retention plan exceeded node budget");

        println!("{}", serde_json::json!({
            "heartbeat_while_blocking_pool_full": heartbeat,
            "queued_cancellation_kept_slot": true, "node_budget_enforced": true, "exact": true,
            "nodes": completed.inventory.plan.nodes_visited,
            "candidate_cas_references": completed.inventory.candidate_only.referenced,
            "protected_cas_references": completed.inventory.protected.referenced,
            "direct_ms": direct_ms, "after_release_ms": released_ms,
            "accepted_queue_ms": completed.queued.as_secs_f64() * 1_000.0,
            "accepted_work_ms": completed.work.as_secs_f64() * 1_000.0,
            "warm_paired_ms": samples,
        }));
        drop((small, worker));
        Arc::try_unwrap(store).map_err(|_| "fixture store still has worker owners")?.close().await
    })
}
