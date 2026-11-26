//! Actual local client/daemon projected-IR fixture, without a socket or builder.
use super::{framed_reconciliation::confirm, milliseconds, source_reconciliation::admission_limits, Result};
use std::{cell::RefCell, future::{poll_fn, Future}, num::NonZeroUsize, path::Path, sync::Arc, task::Poll,
    time::{Duration, Instant}};
use tokio::io::duplex;
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::{AcknowledgedOrdinaryLlvm, SourceReplay, SqliteSourceState}};
use wosy_daemon::{BuildSessionEnd, SourceStorageWorker, SourceStore};
use wosy_protocol::{BuildSessionRefusal, BuildSourceSelection};

const TIME: Duration = Duration::from_secs(15);
const SOURCE_BYTES: usize = 2 << 20;
const PARTITION_BYTES: usize = 128 << 10;

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, profile: &str, mode: &str,
) -> Result<serde_json::Value> {
    let daemon = Arc::new(SourceStore::open(path, 8 << 20)?);
    let workers = NonZeroUsize::new(2).unwrap();
    let bounds = admission_limits(false);
    let mode = mode.to_owned();
    let jobs = RefCell::new(Vec::new());
    let start = Instant::now();
    let output = if mode == "client-limit" { 1 } else { PARTITION_BYTES };
    let answer = client.live_ordinary_llvm(target, profile, TIME, SOURCE_BYTES, output, |selected, replay: Option<SourceReplay>| {
        let replayed = replay.is_some();
        let replay = if mode == "ignore-replay" { None } else { replay.map(|source| source.source.delta) };
        let store = Arc::clone(&daemon);
        let mode = mode.clone();
        let (ours, theirs) = duplex(512);
        jobs.borrow_mut().push((replayed, tokio::spawn(async move {
            let worker = SourceStorageWorker::new(store, tokio::runtime::Handle::current(), workers, workers, workers)?;
            if mode == "diagnostics-only" {
                return worker.serve_recoverable_build_session(theirs, selected, replay, bounds, TIME).await;
            }
            let capacity = if mode == "server-limit" { 1 } else { PARTITION_BYTES };
            worker.serve_recoverable_ordinary_llvm_session(theirs, selected, replay, bounds, capacity, TIME).await
        })));
        std::future::ready(Ok(ours))
    }).await;
    let elapsed = milliseconds(start);
    let mut opens = Vec::new();
    for (replayed, task) in jobs.into_inner() {
        opens.push((replayed, format!("{:?}", task.await??)));
    }
    use AcknowledgedOrdinaryLlvm as Outcome;
    let answer = match answer {
        Ok(Outcome::Partition(partition)) => {
            let projection = partition.projection();
            serde_json::json!({ "kind": "partition", "root": projection.source_root.hex(),
                "profile": projection.profile, "generation": partition.source().generation(),
                "digest": projection.digest.hex(), "compiler_image": projection.compiler_image.hex(),
                "partition": projection.partition })
        }
        Ok(Outcome::Refused(reason)) => {
            assert_eq!(reason, BuildSessionRefusal::OrdinaryLlvmUnavailable);
            serde_json::json!({ "kind": "not-authorized-or-unavailable" })
        }
        Ok(Outcome::ReplayNotAccepted) => serde_json::json!({ "kind": "replay-not-accepted" }),
        Ok(Outcome::WorkspaceChanged) => serde_json::json!({ "kind": "workspace-changed" }),
        Ok(Outcome::ProfileNotSelected) => serde_json::json!({ "kind": "profile-not-selected" }),
        Ok(Outcome::NotAcknowledged) => serde_json::json!({ "kind": "not-acknowledged" }),
        Ok(Outcome::StaleSelection) => serde_json::json!({ "kind": "stale" }),
        Err(error) if mode == "client-limit" => {
            assert_eq!(error.to_string(), "ordinary LLVM partition exceeds the client byte budget");
            serde_json::json!({ "kind": "client-limit" })
        }
        Err(error) => return Err(error),
    };
    Arc::into_inner(daemon).ok_or("fixture daemon not released")?.close().await?;
    Ok(serde_json::json!({ "answer": answer, "opens": opens, "elapsed_ms": elapsed }))
}

pub(super) async fn stale(
    client: &mut SqliteSourceState, project: &Path, path: &Path, target: &str, profile: &str, alternate: String,
) -> Result<serde_json::Value> {
    let initial = client.capture_acknowledged_source(target)?.ok_or("fixture needs its first acknowledged source")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = NonZeroUsize::new(1).unwrap();
    let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), bound, bound, bound)?;
    let (ours, theirs) = duplex(512);
    let mut once = Some(ours);
    let mut request = Box::pin(client.live_ordinary_llvm(target, profile, TIME, SOURCE_BYTES, PARTITION_BYTES, |selected, replay| {
        assert_eq!(&selected, initial.selection());
        assert!(replay.is_none());
        std::future::ready(Ok(once.take().expect("one already accepted connection")))
    }));
    assert!(poll_fn(|cx| Poll::Ready(request.as_mut().poll(cx).is_pending())).await);

    let entry = wosy_project::ProjectContext::load_parse_target(project, Some(target))?.source_path();
    let saved = std::fs::read(&entry)?;
    let mut writer = SqliteSourceState::open(project, ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 })?;
    std::fs::write(&entry, alternate)?;
    let next = writer.stage(target)?.pending.ok_or("fixture missing alternate source attempt")?;
    assert_ne!(next.root(), initial.selection().root);
    confirm(&store, &writer, target, next).await?;
    std::fs::write(&entry, saved)?;
    let same = writer.stage(target)?.pending.ok_or("fixture missing repeated first root")?;
    assert_eq!(same.root(), initial.selection().root);
    confirm(&store, &writer, target, same).await?;

    let selected: BuildSourceSelection = initial.selection().clone();
    let (end, outcome) = tokio::join!(
        worker.serve_recoverable_ordinary_llvm_session(theirs, selected, None, admission_limits(false), PARTITION_BYTES, TIME),
        request,
    );
    assert_eq!(end?, BuildSessionEnd::Disconnected);
    assert!(matches!(outcome?, AcknowledgedOrdinaryLlvm::StaleSelection));
    let latest = client.capture_acknowledged_source(target)?.ok_or("fixture lost selected root")?;
    assert_eq!(latest.selection(), initial.selection());
    assert_eq!(latest.generation(), same.generation());
    assert!(latest.generation() > initial.generation());
    drop(worker);
    Arc::into_inner(store).ok_or("fixture daemon not released")?.close().await?;
    Ok(serde_json::json!({ "kind": "stale", "root": latest.selection().root.hex(),
        "initial_generation": initial.generation(), "latest_generation": latest.generation() }))
}
