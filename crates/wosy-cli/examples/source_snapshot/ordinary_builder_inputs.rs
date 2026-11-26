//! Only the fixture writes or invokes anything from a prepared builder plan.
use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{cell::RefCell, fs::File, num::NonZeroUsize, path::{Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
use tokio::{io::{duplex, AsyncReadExt, DuplexStream}, sync::oneshot};
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::{
    AcknowledgedOrdinaryBuilderInputs, AcknowledgedOrdinaryLlvm, AuthorizedOrdinaryBuilderPeer, SqliteSourceState,
}};
use wosy_compiler::Diagnostic;
use wosy_daemon::{BuildSessionEnd, SourceStorageWorker, SourceStore};
use wosy_protocol::{encode_session_frame, write_encoded_session_frame, BuildSessionCommand, BuildSessionCommandId,
    BuildSessionReply, BuildSessionRequest, BuildSessionResponse, NodeId, SessionFrameReader,
    ORDINARY_LLVM_BUILDER_INPUT_VERSION};

const SOURCE_BYTES: usize = 2 << 20;
const PARTITION_BYTES: usize = 128 << 10;
const TIME: Duration = Duration::from_secs(15);

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, profile: &str, output: PathBuf, expected: &Path,
) -> Result<serde_json::Value> {
    let daemon = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = NonZeroUsize::new(2).unwrap();
    let limits = admission_limits(false);
    let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(File::open(expected)?)?;
    let selection = client.acknowledged_build_source(target)?.ok_or("fixture needs an acknowledged source")?;
    let known = daemon.snapshot(selection.root)?.is_some();
    let mut jobs = RefCell::new(Vec::new());
    let start = Instant::now();
    let answer = client.live_ordinary_builder_inputs(target, profile, TIME, SOURCE_BYTES, PARTITION_BYTES, &expected, output,
        |selected| {
            assert_eq!(selected, selection);
            let daemon = Arc::clone(&daemon);
            let (our, peer) = duplex(512);
            jobs.get_mut().push(tokio::spawn(async move {
                let worker = SourceStorageWorker::new(daemon, tokio::runtime::Handle::current(), bound, bound, bound)?;
                worker.serve_known_ordinary_builder_input_session(
                    peer, selected, limits, PARTITION_BYTES, TIME,
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
    fields(plan, elapsed)
}

fn fields(plan: AcknowledgedOrdinaryBuilderInputs, elapsed: f64) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "root": plan.source().selection().root.hex(), "generation": plan.source().generation(),
        "project": plan.project(), "partition": plan.partition(), "runtime": plan.runtime(),
        "pending": serde_json::to_string_pretty(&plan.inputs().manifest)?,
        "arguments": plan.paths().arguments(), "command": plan.inputs().builder.command,
        "configured_arguments": plan.inputs().builder.args, "elapsed_ms": elapsed,
    }))
}

async fn frame<T: serde::Serialize>(stream: &mut DuplexStream, message: &T) -> Result<()> {
    let encoded = encode_session_frame(message)?;
    assert!(write_encoded_session_frame(stream, &encoded, || true).await?);
    Ok(())
}

// Forward the actual handshake and Project request, stop the real emitted
// reply before the caller sees it, then retire only when that caller drops.
async fn hold_project_reply(
    mut downstream: DuplexStream, mut upstream: DuplexStream, root: NodeId, profile: &str,
    ready: oneshot::Sender<()>,
) -> Result<()> {
    let mut request = SessionFrameReader::default();
    let mut reply = SessionFrameReader::default();
    let start: BuildSessionCommand = request.read(&mut downstream).await?.ok_or("missing fixture Start")?;
    assert_eq!(start.id, BuildSessionCommandId(1));
    assert!(matches!(&start.request, BuildSessionRequest::StartRecoverable { .. }));
    frame(&mut upstream, &start).await?;
    let started: BuildSessionResponse<Diagnostic> = reply.read(&mut upstream).await?.ok_or("missing fixture Started")?;
    assert_eq!(started.id, start.id);
    assert!(matches!(&started.reply, BuildSessionReply::RecoverableStarted { source_root, .. } if *source_root == root));
    frame(&mut downstream, &started).await?;
    let project: BuildSessionCommand = request.read(&mut downstream).await?.ok_or("missing fixture Project")?;
    assert_eq!(project.id, BuildSessionCommandId(2));
    assert!(matches!(&project.request, BuildSessionRequest::ProjectOrdinaryLlvm { version, profile: selected }
        if *version == ORDINARY_LLVM_BUILDER_INPUT_VERSION && selected == profile));
    frame(&mut upstream, &project).await?;
    let emitted: BuildSessionResponse<Diagnostic> = reply.read(&mut upstream).await?.ok_or("missing fixture Project response")?;
    assert_eq!(emitted.id, project.id);
    assert!(matches!(&emitted.reply, BuildSessionReply::OrdinaryLlvmProjection { projection }
        if projection.version == ORDINARY_LLVM_BUILDER_INPUT_VERSION && projection.profile == profile
        && projection.source_root == root && projection.builder.is_some() && !projection.partition.is_empty()));
    ready.send(()).map_err(|_| "fixture canceled before the IR rendezvous")?;
    let mut byte = [0];
    assert_eq!(downstream.read(&mut byte).await?, 0, "canceled planner must not send a terminal request");
    Ok(())
}

pub(super) async fn cancel_concurrent(
    client: &mut SqliteSourceState, project: &Path, path: &Path, target: &str, held_profile: &str,
    sibling_profile: &str, output: PathBuf, expected: &Path,
) -> Result<serde_json::Value> {
    let start = Instant::now();
    let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(File::open(expected)?)?;
    let source = client.capture_acknowledged_source(target)?.ok_or("fixture needs its acknowledged source")?;
    let selection = source.selection();
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    assert!(store.snapshot(selection.root)?.is_some());
    let bound = NonZeroUsize::new(2).unwrap();
    let limits = admission_limits(false);
    let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), bound, bound, bound)?;
    let mut sibling = SqliteSourceState::open(project, ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 })?;
    let (downstream, relay_down) = duplex(512);
    let (relay_up, upstream) = duplex(512);
    let (ready, wait) = oneshot::channel();
    let pending = hold_project_reply(relay_down, relay_up, selection.root, held_profile, ready);
    let server = worker.serve_known_ordinary_builder_input_session(upstream, selection.clone(), limits, PARTITION_BYTES, TIME);
    let held_output = output.with_extension("held");
    assert_ne!(held_output, output);
    let mut downstream = Some(downstream);
    let mut canceled = Box::pin(client.live_ordinary_builder_inputs(
        target, held_profile, TIME, SOURCE_BYTES, PARTITION_BYTES, &expected, held_output.clone(), |selected| {
            assert_eq!(&selected, selection);
            std::future::ready(Ok(downstream.take().expect("only one known connection")))
        },
    ));
    let coordinate = async {
        tokio::select! {
            _ = canceled.as_mut() => return Err("held planner returned despite its missing Project reply".into()),
            signal = wait => signal?,
        }
        let (ours, theirs) = duplex(512);
        let mut ours = Some(ours);
        let plan = sibling.live_ordinary_builder_inputs(
            target, sibling_profile, TIME, SOURCE_BYTES, PARTITION_BYTES, &expected, output.clone(), |selected| {
                assert_eq!(&selected, selection);
                std::future::ready(Ok(ours.take().expect("only one known connection")))
            },
        );
        let sibling_end = worker.serve_known_ordinary_builder_input_session(
            theirs, selection.clone(), limits, PARTITION_BYTES, TIME,
        );
        let (answer, sibling_end) = tokio::try_join!(plan, sibling_end)?;
        assert_eq!(sibling_end, BuildSessionEnd::Disconnected);
        let AcknowledgedOrdinaryLlvm::Partition(plan) = answer else {
            return Err("live sibling did not receive the known plan".into());
        };
        assert_eq!(plan.source().selection(), selection);
        assert_eq!(plan.source().generation(), source.generation());
        drop(canceled);
        Ok(plan)
    };
    let (end, (), plan) = tokio::try_join!(server, pending, coordinate)?;
    assert_eq!(end, BuildSessionEnd::PeerClosed);
    assert!(!output.exists() && !held_output.exists());
    let current = client.capture_acknowledged_source(target)?.ok_or("fixture lost its acknowledged source")?;
    assert_eq!(current.selection(), selection);
    assert_eq!(current.generation(), source.generation());
    drop(worker);
    Arc::into_inner(store).ok_or("fixture daemon not released")?.close().await?;
    let elapsed = milliseconds(start);
    Ok(serde_json::json!({
        "canceled": "PeerClosed", "sibling": "Disconnected", "plan": fields(plan, elapsed)?,
    }))
}
