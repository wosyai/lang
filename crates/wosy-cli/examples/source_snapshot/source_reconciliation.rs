use super::{milliseconds, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use wosy_cli::source_state::SqliteSourceState;
use wosy_daemon::{BaseCstAdmissionLimits, SourceReconciliation, SourceStorageWorker, SourceStore};

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, limited: bool,
) -> Result<serde_json::Value> {
    let selected = client.acknowledged_build_source(target)?.ok_or("no acknowledged source for this target")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let number = |u| NonZeroUsize::new(u).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), number(2), number(8), number(50_000),
    )?;
    let limits = BaseCstAdmissionLimits {
        records: 25_000, record_bytes: 16 << 20, graph_edges: 100_000,
        expanded_record_bytes: 32 << 20, cst_occurrences: 100_000, cst_depth: 128,
        authored_bytes: if limited { 0 } else { 2 << 20 },
    };
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
