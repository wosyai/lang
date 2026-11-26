use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use tokio::io::{self, AsyncWriteExt, DuplexStream};
use wosy_cli::source_state::{ReplaySource, SqliteSourceState};
use wosy_compiler::Diagnostic;
use wosy_daemon::{
    encode_session_frame, BuildSessionEnd, SessionFrameReader, SourceStorageWorker, SourceStore,
};
use wosy_protocol::{
    BuildDiagnosticRequest, BuildDiagnosticResponse, BuildSessionCommand, BuildSessionCommandId,
    BuildSessionRefusal as Refusal, BuildSessionReply as Reply, BuildSessionRequest as Request,
    BuildSessionResponse, BuildSourceSelection, Delta, BUILD_DIAGNOSTIC_VERSION, BUILD_RECONCILIATION_VERSION,
};

#[derive(Clone, Copy)]
enum Expected {
    ReplayRequired,
    Refused(Refusal),
    Analysis,
}

async fn command(stream: &mut DuplexStream, id: u64, request: Request) -> Result<()> {
    stream.write_all(&encode_session_frame(&BuildSessionCommand { id: BuildSessionCommandId(id), request })?).await?;
    Ok(())
}

async fn reply(stream: &mut DuplexStream, frames: &mut SessionFrameReader, id: u64) -> Result<Reply<Diagnostic>> {
    let message = frames.read::<_, BuildSessionResponse<Diagnostic>>(stream).await?.ok_or("missing framed reply")?;
    assert_eq!(message.id, BuildSessionCommandId(id));
    Ok(message.reply)
}

async fn exchange(
    worker: &SourceStorageWorker, selected: BuildSourceSelection, replay: Option<Delta>,
    expected: Expected, version: u16, profile: &str,
) -> Result<Option<BuildDiagnosticResponse<Diagnostic>>> {
    let (mut client, peer) = io::duplex(256);
    let root = selected.root;
    let (end, result) = tokio::join!(
        worker.serve_recoverable_build_session(peer, selected, replay, admission_limits(false), Duration::from_secs(30)),
        async {
            let mut frames = SessionFrameReader::default();
            command(&mut client, 1, Request::StartRecoverable { version }).await?;
            let started = reply(&mut client, &mut frames, 1).await?;
            let (status, response) = match (expected, started) {
                (Expected::ReplayRequired, Reply::ReplayRequired { version: BUILD_RECONCILIATION_VERSION, source_root }) => {
                    assert_eq!(source_root, root);
                    (BuildSessionEnd::ReplayRequired, None)
                }
                (Expected::Refused(expected), Reply::Refused { reason }) => {
                    assert_eq!(expected, reason);
                    (BuildSessionEnd::Refused, None)
                }
                (Expected::Analysis, Reply::RecoverableStarted { version: BUILD_RECONCILIATION_VERSION, source_root }) => {
                    assert_eq!(source_root, root);
                    command(&mut client, 2, Request::StartRecoverable { version }).await?;
                    assert!(matches!(reply(&mut client, &mut frames, 2).await?, Reply::Refused { reason: Refusal::AlreadyStarted }));
                    command(&mut client, 3, Request::Renew).await?;
                    assert!(matches!(reply(&mut client, &mut frames, 3).await?, Reply::Renewed));
                    command(&mut client, 4, Request::Analyze { request: BuildDiagnosticRequest::BuildDiagnostics {
                        version: BUILD_DIAGNOSTIC_VERSION, profile: profile.to_owned(),
                    }}).await?;
                    let Reply::Analysis { response } = reply(&mut client, &mut frames, 4).await?
                        else { return Err("missing ordinary diagnostic response".into()); };
                    let BuildDiagnosticResponse::BuildDiagnostics { source_root, .. } = &response;
                    assert_eq!(*source_root, root);
                    command(&mut client, 5, Request::Disconnect).await?;
                    assert!(matches!(reply(&mut client, &mut frames, 5).await?, Reply::Closed));
                    (BuildSessionEnd::Disconnected, Some(response))
                }
                _ => return Err("unexpected recovery handshake reply".into()),
            };
            assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await?.is_none());
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((status, response))
        },
    );
    let (expected_end, response) = result?;
    assert_eq!(end?, expected_end);
    Ok(response)
}

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, profile: &str, advance: bool,
) -> Result<serde_json::Value> {
    let mut selected = client.acknowledged_build_source(target)?.ok_or("no acknowledged source for this target")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let version = BUILD_RECONCILIATION_VERSION;
    let started = Instant::now();
    exchange(&worker, selected.clone(), None, Expected::Refused(Refusal::UnsupportedVersion), version + 1, profile).await?;
    exchange(&worker, selected.clone(), None, Expected::ReplayRequired, version, profile).await?;
    let challenge_ms = milliseconds(started);
    assert!(store.snapshot(selected.root)?.is_none());
    let before = selected.root;
    let mut replay = client.replay_acknowledged_source(&selected)?.ok_or("fixture acknowledged source changed")?;

    if advance {
        // The previously captured replay really was acknowledged. Confirm
        // the newer source normally in a separate disposable catalog.
        let next = client.replay(target, ReplaySource::Pending)?.ok_or("fixture needs newer staged source")?;
        let attempt = next.attempt.ok_or("fixture pending replay has no attempt")?;
        let confirming = SourceStore::open(&path.with_extension("confirmation"), 8 << 20)?;
        confirming.publish(next.source.root, &next.source.delta).await?;
        assert!(client.acknowledge(target, attempt)?);
        confirming.close().await?;
        let old_selection = selected;
        selected = client.acknowledged_build_source(target)?.ok_or("fixture advance did not select")?;
        assert_ne!(old_selection.root, selected.root);
        assert!(client.replay_acknowledged_source(&old_selection)?.is_none());
        exchange(&worker, selected.clone(), Some(replay.source.delta),
            Expected::Refused(Refusal::SourceUnavailable), version, profile).await?;
        assert!(store.snapshot(old_selection.root)?.is_none() && store.snapshot(selected.root)?.is_none());
        replay = client.replay_acknowledged_source(&selected)?.ok_or("fixture needs newer acknowledged replay")?;
    }
    assert_eq!(replay.source.parsed, 0);

    // A prequeued valid start doesn't win against an already elapsed real
    // start deadline. Then reconnect on a distinct stream with the same ack.
    let (mut client_stream, peer) = io::duplex(256);
    command(&mut client_stream, 1, Request::StartRecoverable { version }).await?;
    let end = worker.serve_recoverable_build_session(
        peer, selected.clone(), None, admission_limits(false), Duration::ZERO,
    ).await?;
    assert_eq!(end, BuildSessionEnd::Expired);
    assert!(SessionFrameReader::default().read::<_, BuildSessionResponse<Diagnostic>>(&mut client_stream).await?.is_none());
    assert!(store.snapshot(selected.root)?.is_none());

    let started = Instant::now();
    let restored = exchange(&worker, selected.clone(), Some(replay.source.delta), Expected::Analysis, version, profile).await?;
    let framed_recovery_ms = milliseconds(started);
    let started = Instant::now();
    let warm = exchange(&worker, selected.clone(), None, Expected::Analysis, version, profile).await?;
    let framed_warm_ms = milliseconds(started);
    assert!(store.snapshot(selected.root)?.is_some());

    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns source store").close().await?;
    Ok(serde_json::json!({
        "challenge_pair_ms": challenge_ms, "framed_recovery_ms": framed_recovery_ms, "framed_warm_ms": framed_warm_ms,
        "initial": before.hex(), "current": selected.root.hex(), "stale_refused": advance, "deadline_closed": true,
        "restored": restored, "warm": warm,
    }))
}
