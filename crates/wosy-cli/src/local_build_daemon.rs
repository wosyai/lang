//! An ordinary, in-process daemon peer. The framed stream is paired directly
//! with our compiled server code; it cannot be supplied by an external process.

use crate::{
    artifact_inputs::PendingBuilderInputs,
    source_state::{AcknowledgedSourceGeneration, AuthorizedOrdinaryBuilderPeer, ReplaySource, SqliteSourceState},
};
use std::{cell::RefCell, fs::File, num::NonZeroUsize, sync::Arc, time::Duration};
use tokio::{task::JoinSet, time::{timeout_at, Instant}};
use wosy_daemon::{BaseCstAdmissionLimits, SourceStorageWorker, SourceStore};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Acknowledge only a committed daemon source root. A pending client
/// proposal left by an interrupted build is replayable, not a trusted result.
pub async fn establish_local_build_source(
    source: &mut SqliteSourceState,
    store: &SourceStore,
    target: &str,
) -> Result<bool> {
    if let Some(pending) = source.replay(target, ReplaySource::Pending)? {
        let attempt = pending.attempt.ok_or("pending source has no attempt")?;
        store.publish(pending.source.root, &pending.source.delta).await?;
        if !source.acknowledge(target, attempt)? {
            return Ok(false);
        }
    }
    let staged = source.stage(target)?;
    if let Some(attempt) = staged.pending {
        let Some(pending) = source.replay(target, ReplaySource::Pending)? else {
            return Ok(false);
        };
        if pending.attempt != Some(attempt) {
            return Ok(false);
        }
        store.publish(pending.source.root, &pending.source.delta).await?;
        if !source.acknowledge(target, attempt)? {
            return Ok(false);
        }
    }

    let Some(selected) = source.capture_acknowledged_source(target)? else {
        return Ok(false);
    };
    let selection = selected.selection();
    if selection.root != staged.root {
        return Ok(false);
    }
    if store.snapshot(selection.root)?.is_none() {
        let Some(replay) = source.replay_acknowledged_at(selection, Some(selected.generation()))? else {
            return Ok(false);
        };
        store.publish(selection.root, &replay.source.delta).await?;
    }
    let Some(current) = source.capture_acknowledged_source(target)? else {
        return Ok(false);
    };
    Ok(current.selection() == selection
        && current.generation() == selected.generation()
        && store.snapshot(selection.root)?.is_some())
}

pub async fn compare_local_build(
    source: &mut SqliteSourceState,
    store: Arc<SourceStore>,
    target: &str,
    profile: &str,
    ordinary: &PendingBuilderInputs,
    limits: BaseCstAdmissionLimits,
    max_partition_bytes: usize,
    timeout: Duration,
) -> Result<Option<AcknowledgedSourceGeneration>> {
    let deadline = Instant::now().checked_add(timeout).ok_or("ordinary comparison deadline overflow")?;
    timeout_at(deadline, async {
        let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(open_running_image()?)?;
        let one = NonZeroUsize::MIN;
        let worker = Arc::new(SourceStorageWorker::new(store, tokio::runtime::Handle::current(), one, one, one)?);
        // A dropped comparison (including timeout) aborts all framed sessions.
        let sessions = RefCell::new(JoinSet::new());
        let answer = source.compare_selected_ordinary_llvm(
            target, profile, deadline.saturating_duration_since(Instant::now()),
            limits.authored_bytes, max_partition_bytes, &expected, ordinary,
            |selection, replay| {
                let worker = Arc::clone(&worker);
                let (client, server) = tokio::io::duplex(4096);
                sessions.borrow_mut().spawn(async move {
                    worker.serve_recoverable_ordinary_llvm_session(
                        server, selection, replay.map(|replay| replay.source.delta),
                        limits, max_partition_bytes, deadline.saturating_duration_since(Instant::now()),
                    ).await
                });
                std::future::ready(Ok(client))
            },
        ).await;
        let mut sessions = sessions.into_inner();
        if answer.is_err() {
            sessions.shutdown().await;
            return answer;
        }
        while let Some(result) = sessions.join_next().await {
            if let Err(error) = result.map_err(Into::into).and_then(|session| session) {
                sessions.shutdown().await;
                return Err(error);
            }
        }
        answer
    }).await?
}

/// Select the ordinary authored IR from our own linked daemon. The result
/// remains a plan; this function neither launches nor caches external tools.
pub async fn prepare_local_builder_inputs(
    source: &mut SqliteSourceState, store: Arc<SourceStore>, target: &str, profile: &str,
    output: std::path::PathBuf, limits: BaseCstAdmissionLimits, timeout: Duration,
) -> Result<Option<crate::source_state::AcknowledgedOrdinaryBuilderInputs>> {
    use crate::source_state::AcknowledgedOrdinaryLlvm;
    let deadline = Instant::now().checked_add(timeout).ok_or("ordinary preparation deadline overflow")?;
    timeout_at(deadline, async {
        let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(open_running_image()?)?;
        let one = NonZeroUsize::MIN;
        let worker = Arc::new(SourceStorageWorker::new(store, tokio::runtime::Handle::current(), one, one, one)?);
        let mut sessions = JoinSet::new();
        let maximum = wosy_protocol::MAX_SESSION_FRAME_BYTES;
        let answer = source.live_ordinary_builder_inputs(
            target, profile, deadline.saturating_duration_since(Instant::now()),
            limits.authored_bytes, maximum, &expected, output,
            |selected| {
                let worker = Arc::clone(&worker);
                let (client, server) = tokio::io::duplex(4096);
                sessions.spawn(async move {
                    worker.serve_known_ordinary_builder_input_session(
                        server, selected, limits, maximum, deadline.saturating_duration_since(Instant::now()),
                    ).await
                });
                std::future::ready(Ok(client))
            },
        ).await;
        if answer.is_err() {
            sessions.shutdown().await;
        }
        while let Some(result) = sessions.join_next().await {
            if let Err(error) = result.map_err(Into::into).and_then(|session| session) {
                sessions.shutdown().await;
                return Err(error);
            }
        }
        Ok(match answer? {
            AcknowledgedOrdinaryLlvm::Partition(plan) => Some(plan),
            _ => None,
        })
    }).await?
}

#[cfg(target_os = "linux")]
fn open_running_image() -> std::io::Result<File> {
    File::open("/proc/self/exe")
}

#[cfg(not(target_os = "linux"))]
fn open_running_image() -> std::io::Result<File> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "running compiler image is unavailable"))
}
