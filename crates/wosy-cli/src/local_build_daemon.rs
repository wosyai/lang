//! Local daemon coordination. Ordinary opt-in Build owns a private
//! compiler child; private builder-plan inspection stays in process.

use crate::source_state::{AcknowledgedSourceGeneration, AuthorizedOrdinaryBuilderPeer, ReplaySource, SqliteSourceState};
use std::{fs::File, num::NonZeroUsize, sync::Arc, time::Duration};
#[cfg(all(test, target_os = "linux"))]
use {crate::{artifact_inputs::PendingBuilderInputs, source_state::SourceReplay},
    tokio::net::UnixStream, wosy_daemon::BuildSessionEnd, wosy_protocol::BuildSourceSelection};
#[cfg(target_os = "linux")]
mod owned_child;
#[cfg(target_os = "linux")]
pub use owned_child::{compare_owned_child, enter_if_requested};
use tokio::{task::JoinSet, time::{timeout_at, Instant}};
use wosy_daemon::{BaseCstAdmissionLimits, SourceStorageWorker, SourceStore};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Acknowledge a new client cut only after its delta commits. For an
/// already acknowledged cut, retaining the graph is only an observation:
/// the scoped child can challenge and replay it on a fresh connection.
pub async fn stage_local_build_source(
    source: &mut SqliteSourceState,
    store: &SourceStore,
    target: &str,
) -> Result<Option<AcknowledgedSourceGeneration>> {
    if let Some(pending) = source.replay(target, ReplaySource::Pending)? {
        let attempt = pending.attempt.ok_or("pending source has no attempt")?;
        store.publish(pending.source.root, &pending.source.delta).await?;
        if !source.acknowledge(target, attempt)? {
            return Ok(None);
        }
    }
    let staged = source.stage(target)?;
    if let Some(attempt) = staged.pending {
        let Some(pending) = source.replay(target, ReplaySource::Pending)? else {
            return Ok(None);
        };
        if pending.attempt != Some(attempt) {
            return Ok(None);
        }
        store.publish(pending.source.root, &pending.source.delta).await?;
        if !source.acknowledge(target, attempt)? {
            return Ok(None);
        }
    }

    Ok(source.capture_acknowledged_source(target)?
        .filter(|selected| selected.selection().root == staged.root))
}

/// Local callers outside the child comparison need the graph present in
/// the same SourceStore in which they will acquire their lease.
pub async fn establish_local_build_source(
    source: &mut SqliteSourceState,
    store: &SourceStore,
    target: &str,
) -> Result<bool> {
    let Some(selected) = stage_local_build_source(source, store, target).await? else {
        return Ok(false);
    };
    let selection = selected.selection();
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

#[cfg(all(test, target_os = "linux"))]
fn open_scoped_session(
    worker: &Arc<SourceStorageWorker>, sessions: &mut JoinSet<Result<BuildSessionEnd>>,
    selection: BuildSourceSelection, replay: Option<SourceReplay>, profile: &str,
    limits: BaseCstAdmissionLimits, max_partition_bytes: usize, deadline: Instant,
) -> Result<UnixStream> {
    let (client, server) = UnixStream::pair()?;
    let worker = Arc::clone(worker);
    let profile = profile.to_owned();
    sessions.spawn(async move {
        worker.serve_scoped_ordinary_llvm_session(
            server, selection, replay.map(|replay| replay.source.delta), profile,
            limits, max_partition_bytes, deadline.into_std(),
        ).await
    });
    Ok(client)
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

#[cfg(all(test, target_os = "linux"))]
mod socket_tests;
