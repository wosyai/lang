//! Only the fixture writes or invokes anything from a prepared builder plan.
use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{cell::RefCell, fs::File, num::NonZeroUsize, path::{Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
use tokio::io::duplex;
use wosy_cli::source_state::{AcknowledgedOrdinaryLlvm, AuthorizedOrdinaryBuilderPeer, SqliteSourceState};
use wosy_daemon::{BuildSessionEnd, SourceStorageWorker, SourceStore};

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, profile: &str, output: PathBuf, expected: &Path,
) -> Result<serde_json::Value> {
    let daemon = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = NonZeroUsize::new(2).unwrap();
    let limits = admission_limits(false);
    let timeout = Duration::from_secs(15);
    let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(File::open(expected)?)?;
    let selection = client.acknowledged_build_source(target)?.ok_or("fixture needs an acknowledged source")?;
    let known = daemon.snapshot(selection.root)?.is_some();
    let mut jobs = RefCell::new(Vec::new());
    let start = Instant::now();
    let answer = client.live_ordinary_builder_inputs(target, profile, timeout, 2 << 20, 128 << 10, &expected, output,
        |selected| {
            assert_eq!(selected, selection);
            let daemon = Arc::clone(&daemon);
            let (our, peer) = duplex(512);
            jobs.get_mut().push(tokio::spawn(async move {
                let worker = SourceStorageWorker::new(daemon, tokio::runtime::Handle::current(), bound, bound, bound)?;
                worker.serve_known_ordinary_builder_input_session(
                    peer, selected, limits, 128 << 10, timeout,
                ).await
            }));
            std::future::ready(Ok(our))
        }).await;
    let elapsed = milliseconds(start);
    let mut jobs = jobs.into_inner();
    assert_eq!(jobs.len(), 1);
    let close = jobs.pop().unwrap().await??;
    assert_eq!(daemon.snapshot(selection.root)?.is_some(), known);
    Arc::into_inner(daemon).ok_or("fixture daemon not released")?.close().await?;
    if !known {
        assert!(matches!(answer?, AcknowledgedOrdinaryLlvm::ReplayNotAccepted));
        assert_eq!(close, BuildSessionEnd::ReplayRequired);
        return Ok(serde_json::json!({"kind": "replay-required", "opens": 1, "known": false}));
    }
    // Compare orderly close even when the expected image won't match.
    assert_eq!(close, BuildSessionEnd::Disconnected);
    let AcknowledgedOrdinaryLlvm::Partition(plan) = answer? else { return Err("fixture expected builder inputs".into()); };
    Ok(serde_json::json!({
        "root": plan.source().selection().root.hex(), "generation": plan.source().generation(),
        "project": plan.project(), "partition": plan.partition(), "runtime": plan.runtime(),
        "pending": serde_json::to_string_pretty(&plan.inputs().manifest)?,
        "arguments": plan.paths().arguments(), "command": plan.inputs().builder.command,
        "configured_arguments": plan.inputs().builder.args, "elapsed_ms": elapsed,
    }))
}
