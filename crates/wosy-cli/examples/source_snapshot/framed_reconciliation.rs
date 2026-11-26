use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use tokio::io::{self, AsyncWriteExt, DuplexStream};
use wosy_cli::source_state::{
    CapturedBuildDiagnostics, DiagnosticExchange, ReplaySource, RequiredSourceReplay, SqliteSourceState,
};
use wosy_compiler::Diagnostic;
use wosy_daemon::{BuildSessionEnd, SourceStorageWorker, SourceStore};
use wosy_protocol::{
    encode_session_frame, BuildDiagnosticResponse, BuildSessionCommand, BuildSessionCommandId,
    BuildSessionRefusal as Refusal, BuildSessionReply as Reply, BuildSessionRequest as Request,
    BuildSessionResponse, BuildSourceSelection, Delta, SessionFrameReader, BUILD_RECONCILIATION_VERSION,
};

async fn start(stream: &mut DuplexStream, version: u16) -> Result<()> {
    stream.write_all(&encode_session_frame(&BuildSessionCommand {
        id: BuildSessionCommandId(1), request: Request::StartRecoverable { version },
    })?).await?;
    Ok(())
}

async fn unsupported_start(worker: &SourceStorageWorker, selected: BuildSourceSelection) -> Result<()> {
    let (mut client, peer) = io::duplex(256);
    let (end, result) = tokio::join!(
        worker.serve_recoverable_build_session(peer, selected, None, admission_limits(false), Duration::from_secs(30)),
        async {
            start(&mut client, BUILD_RECONCILIATION_VERSION + 1).await?;
            let mut frames = SessionFrameReader::default();
            let reply: BuildSessionResponse<Diagnostic> = frames.read(&mut client).await?.ok_or("missing version refusal")?;
            assert_eq!(reply.id, BuildSessionCommandId(1));
            assert!(matches!(reply.reply, Reply::Refused { reason: Refusal::UnsupportedVersion }));
            assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await?.is_none());
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        },
    );
    result?;
    assert_eq!(end?, BuildSessionEnd::Refused);
    Ok(())
}

async fn exchange(
    worker: &SourceStorageWorker, local: &mut SqliteSourceState,
    capture: &CapturedBuildDiagnostics, replay: Option<Delta>,
) -> Result<DiagnosticExchange> {
    let (client, peer) = io::duplex(256);
    let (end, result) = tokio::join!(
        worker.serve_recoverable_build_session(peer, capture.selection().clone(), replay,
            admission_limits(false), Duration::from_secs(30)),
        capture.exchange(local, client, Duration::from_secs(10)),
    );
    let result = result?;
    match &result {
        DiagnosticExchange::ReplayRequired(_) => assert_eq!(end?, BuildSessionEnd::ReplayRequired),
        DiagnosticExchange::Refused(_) => assert!(matches!(end?, BuildSessionEnd::Refused | BuildSessionEnd::Disconnected)),
        DiagnosticExchange::Diagnostics(_) => assert_eq!(end?, BuildSessionEnd::Disconnected),
        DiagnosticExchange::StaleSelection => assert_eq!(end?, BuildSessionEnd::PeerClosed),
    }
    Ok(result)
}

fn required(result: DiagnosticExchange) -> Result<RequiredSourceReplay> {
    match result {
        DiagnosticExchange::ReplayRequired(request) => Ok(request),
        _ => Err("expected a terminal replay request".into()),
    }
}

fn diagnostics(result: DiagnosticExchange) -> Result<BuildDiagnosticResponse<Diagnostic>> {
    match result {
        DiagnosticExchange::Diagnostics(response) => Ok(response),
        _ => Err("expected ordinary diagnostics".into()),
    }
}

pub(super) async fn run(
    client: &mut SqliteSourceState, path: &Path, target: &str, profile: &str, advance: bool,
) -> Result<serde_json::Value> {
    let mut capture = client.capture_build_diagnostics(target, profile)?.ok_or("no acknowledged source for this target")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let started = Instant::now();
    unsupported_start(&worker, capture.selection().clone()).await?;
    let request = required(exchange(&worker, client, &capture, None).await?)?;
    let challenge_ms = milliseconds(started);
    let before = capture.selection().root;
    assert!(store.snapshot(before)?.is_none());
    let mut replay = request.prepare(client)?.ok_or("fixture acknowledged source changed")?;

    if advance {
        let next = client.replay(target, ReplaySource::Pending)?.ok_or("fixture needs newer staged source")?;
        let attempt = next.attempt.ok_or("fixture pending replay has no attempt")?;
        let confirming = SourceStore::open(&path.with_extension("confirmation"), 8 << 20)?;
        confirming.publish(next.source.root, &next.source.delta).await?;
        assert!(client.acknowledge(target, attempt)?);
        confirming.close().await?;
        assert!(matches!(exchange(&worker, client, &capture, None).await?, DiagnosticExchange::StaleSelection));
        capture = client.capture_build_diagnostics(target, profile)?.ok_or("fixture advance did not select")?;
        assert_ne!(before, capture.selection().root);
        assert!(matches!(exchange(&worker, client, &capture, Some(replay.source.delta)).await?,
            DiagnosticExchange::Refused(Refusal::SourceUnavailable)));
        assert!(store.snapshot(before)?.is_none() && store.snapshot(capture.selection().root)?.is_none());
        replay = required(exchange(&worker, client, &capture, None).await?)?.prepare(client)?
            .ok_or("fixture needs newer acknowledged replay")?;
    }
    assert_eq!(replay.source.parsed, 0);

    let (mut client_stream, peer) = io::duplex(256);
    start(&mut client_stream, BUILD_RECONCILIATION_VERSION).await?;
    let end = worker.serve_recoverable_build_session(
        peer, capture.selection().clone(), None, admission_limits(false), Duration::ZERO,
    ).await?;
    assert_eq!(end, BuildSessionEnd::Expired);
    assert!(SessionFrameReader::default().read::<_, BuildSessionResponse<Diagnostic>>(&mut client_stream).await?.is_none());
    assert!(store.snapshot(capture.selection().root)?.is_none());

    let started = Instant::now();
    let restored = diagnostics(exchange(&worker, client, &capture, Some(replay.source.delta)).await?)?;
    let framed_recovery_ms = milliseconds(started);
    let started = Instant::now();
    let warm = diagnostics(exchange(&worker, client, &capture, None).await?)?;
    let framed_warm_ms = milliseconds(started);
    assert!(store.snapshot(capture.selection().root)?.is_some());

    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns source store").close().await?;
    Ok(serde_json::json!({
        "challenge_pair_ms": challenge_ms, "framed_recovery_ms": framed_recovery_ms, "framed_warm_ms": framed_warm_ms,
        "initial": before.hex(), "current": capture.selection().root.hex(), "stale_refused": advance, "deadline_closed": true,
        "restored": restored, "warm": warm,
    }))
}

/// Safe test fixture: `path` is the disposable catalog for this project's
/// already accepted initial source. Both files changed by this driver live
/// under the fixture supplied by tests/acknowledged_source_reconciliation.py.
pub(super) async fn run_generation(
    client: &mut SqliteSourceState, project: &Path, path: &Path, target: &str, profile: &str,
    changed: &str,
) -> Result<serde_json::Value> {
    use std::{future::{poll_fn, Future}, task::Poll};
    use wosy_cli::source_recovery::ClientSourceLimits;

    let initial = client.capture_build_diagnostics(target, profile)?.ok_or("fixture needs an acknowledged source")?;
    let root = initial.selection().root;
    let runtime = tokio::runtime::Handle::current();
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let missing = Arc::new(SourceStore::open(&path.with_extension("generation-miss"), 8 << 20)?);
    let probe = SourceStorageWorker::new(Arc::clone(&missing), runtime.clone(), bound(1), bound(2), bound(20_000))?;
    let stale_replay = required(exchange(&probe, client, &initial, None).await?)?;
    drop(probe);
    Arc::try_unwrap(missing).ok().expect("owned missing store").close().await?;

    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = SourceStorageWorker::new(Arc::clone(&store), runtime, bound(1), bound(2), bound(20_000))?;
    let entry = wosy_project::ProjectContext::load_parse_target(project, Some(target))?.source_path();
    let original = std::fs::read(&entry)?;
    let limits = ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 };
    let mut writer = SqliteSourceState::open(project, limits)?;
    let pending = match writer.selection(target)?.pending {
        Some(attempt) => {
            // An upgrade fixture can already have legitimately staged B
            // using the previous client. Its pending root must survive.
            assert_eq!(std::fs::read(&entry)?, changed.as_bytes());
            // The initial authored file will be supplied by the fixture in
            // this case via its separately accepted CST, not guessed here.
            attempt
        }
        None => {
            std::fs::write(&entry, changed)?;
            writer.stage(target)?.pending.ok_or("fixture failed to stage a second source")?
        }
    };
    assert_ne!(pending.root(), root);
    assert_eq!(writer.capture_build_diagnostics(target, profile)?.unwrap().generation(), initial.generation());
    let before = diagnostics(exchange(&worker, client, &initial, None).await?)?;

    let (stream, peer) = io::duplex(256);
    let mut delayed = Box::pin(initial.exchange(client, stream, Duration::from_secs(10)));
    // Enter the real client operation before the daemon side is polled.
    // Its initial selection check passes and it waits for the real reply.
    assert!(poll_fn(|cx| Poll::Ready(delayed.as_mut().poll(cx).is_pending())).await);
    confirm(&store, &writer, target, pending).await?;
    assert_ne!(writer.capture_build_diagnostics(target, profile)?.unwrap().selection().root, root);

    let key = wosy_protocol::FileKey {
        package: wosy_project::ProjectContext::load_parse_target(project, Some(target))?.package,
        path: wosy_project::PackageRelativePath::new(entry.strip_prefix(project)?.into())?,
    };
    let restore = store.snapshot(root)?.ok_or("fixture previous source missing")?
        .authored(&key)?.cst.root.text().to_string().into_bytes();
    // Our regular fixture starts on A. An upgrade fixture can instead open
    // while B is pending, in which case exact A comes from its retained CST.
    if original != changed.as_bytes() { assert_eq!(restore, original); }
    std::fs::write(&entry, restore)?;
    let same_root = writer.stage(target)?.pending.ok_or("fixture failed to restage the original source")?;
    assert_eq!(same_root.root(), root);
    assert!(same_root.generation() > initial.generation());
    confirm(&store, &writer, target, same_root).await?;

    let (end, result) = tokio::join!(
        worker.serve_recoverable_build_session(peer, initial.selection().clone(), None,
            admission_limits(false), Duration::from_secs(30)),
        delayed,
    );
    assert_eq!(end?, BuildSessionEnd::Disconnected);
    assert!(matches!(result?, DiagnosticExchange::StaleSelection));
    assert!(stale_replay.prepare(client)?.is_none());
    assert!(matches!(exchange(&worker, client, &initial, None).await?, DiagnosticExchange::StaleSelection));
    let mut reopened = SqliteSourceState::open(project, limits)?;
    let latest = reopened.capture_build_diagnostics(target, profile)?.ok_or("fixture lost acknowledgement at reopen")?;
    assert_eq!((latest.selection().root, latest.generation()), (root, same_root.generation()));
    let fresh = diagnostics(exchange(&worker, &mut reopened, &latest, None).await?)?;
    let view = |response| match response {
        BuildDiagnosticResponse::BuildDiagnostics { version, source_root, profile, preparation, receipt, phase, diagnostics, .. }
            => (version, source_root, profile, preparation, receipt, phase, diagnostics),
    };
    assert_eq!(view(before), view(fresh));
    drop(worker);
    Arc::try_unwrap(store).ok().expect("owned fixture store").close().await?;
    Ok(serde_json::json!({
        "root": root.hex(), "initial_generation": initial.generation(), "final_generation": latest.generation(),
        "pending_retained_selection": true, "delayed_reply_refused": true, "old_terminal_replay_refused": true,
    }))
}

pub(super) async fn confirm(store: &SourceStore, writer: &SqliteSourceState, target: &str,
    attempt: wosy_cli::source_state::SourceAttempt,
) -> Result<()> {
    let pending = writer.replay(target, ReplaySource::Pending)?.ok_or("fixture pending source is missing")?;
    assert_eq!(pending.attempt, Some(attempt));
    store.publish(pending.source.root, &pending.source.delta).await?;
    assert!(writer.acknowledge(target, attempt)?);
    Ok(())
}

pub(super) async fn run_terminal_report(
    client: &mut SqliteSourceState, project: &Path, path: &Path, target: &str, profile: &str, mode: &str,
    live_bytes: usize,
) -> Result<serde_json::Value> {
    use codespan_reporting::term::termcolor::Buffer;
    use std::cell::Cell;
    use wosy_cli::{
        source_recovery::ClientSourceLimits,
        source_state::{AcknowledgedBuildDiagnostics as Outcome, SourceReplay},
    };
    use wosy_protocol::CstExpansionBudget;

    let (live, mode) = mode.strip_prefix("live-").map_or((false, mode), |mode| (true, mode));
    let initial = client.capture_build_diagnostics(target, profile)?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let (open, mut peers) = tokio::sync::mpsc::channel::<(BuildSourceSelection, Option<SourceReplay>, DuplexStream)>(1);
    let budget = CstExpansionBudget::new(16 << 20, 200_000, 1024, 8 << 20);
    let timeout = match mode {
        "expired" => Duration::ZERO,
        "late-replay" => Duration::from_millis(600),
        _ => Duration::from_secs(10),
    };
    let start = Instant::now();
    let calls = Cell::new(0);
    let open_calls = &calls;
    let adapter = move |selected, replay: Option<SourceReplay>| {
        let ordinal = open_calls.get() + 1;
        open_calls.set(ordinal);
        let open = open.clone();
        async move {
            if mode == "late-replay" {
                let ready = if ordinal == 1 {
                    assert!(replay.is_none());
                    start + Duration::from_millis(160)
                } else {
                    assert_eq!(ordinal, 2);
                    assert!(replay.is_some());
                    let late = start + timeout + Duration::from_millis(40);
                    // A reset timeout would admit this retry.
                    assert!(late < Instant::now() + timeout);
                    late
                };
                tokio::time::sleep_until(tokio::time::Instant::from_std(ready)).await;
            }
            let (stream, peer) = io::duplex(256);
            open.send((selected, replay, peer)).await
                .map_err(|_| "fixture adapter closed before opening the stream")?;
            Ok(stream)
        }
    };
    let (answer, opened) = tokio::join!(
        async {
            if live {
                client.live_build_diagnostics(target, profile, timeout, live_bytes, budget, adapter).await
            } else {
                client.acknowledged_build_diagnostics(target, profile, timeout, budget, adapter).await
            }
        },
        async {
            let mut ended = Vec::new();
            while let Some((selected, replay, mut peer)) = peers.recv().await {
                assert!(ended.len() < 2, "one operation opened a third stream");
                let captured = initial.as_ref().ok_or("fixture opened without an acknowledged source")?;
                assert_eq!(&selected, captured.selection());
                let offered = replay.is_some();
                let replay = replay.map(|replay| {
                    assert_eq!((replay.source.root, replay.source.parsed, replay.attempt), (selected.root, 0, None));
                    replay.source.delta
                });
                let end = worker.serve_recoverable_build_session(
                    &mut peer, selected, if mode == "ignore-replay" { None } else { replay },
                    admission_limits(false), Duration::from_secs(30),
                ).await?;
                if mode == "stale" {
                    assert_eq!(end, BuildSessionEnd::ReplayRequired);
                    assert!(!offered);
                    let confirmation = SourceStore::open(&path.with_extension("confirmation"), 8 << 20)?;
                    let mut writer = SqliteSourceState::open(project, ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 })?;
                    let entry = wosy_project::ProjectContext::load_parse_target(project, Some(target))?.source_path();
                    let a = std::fs::read(&entry)?;
                    std::fs::write(&entry, "%%start\nu32 updated = 9;\n%%end\n")?;
                    let b = writer.stage(target)?.pending.ok_or("fixture did not issue B")?;
                    assert_ne!(b.root(), captured.selection().root);
                    confirm(&confirmation, &writer, target, b).await?;
                    std::fs::write(&entry, a)?;
                    let fresh = writer.stage(target)?.pending.ok_or("fixture did not reissue A")?;
                    assert_eq!(fresh.root(), captured.selection().root);
                    assert!(fresh.generation() > captured.generation());
                    confirm(&confirmation, &writer, target, fresh).await?;
                    confirmation.close().await?;
                }
                // The real client validates terminal EOF before using the
                // response. In the stale case a second writer has just
                // re-acknowledged A at a different generation.
                drop(peer);
                ended.push((offered, end));
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(ended)
        },
    );
    let elapsed = milliseconds(start);
    let ended = opened?;
    let offered = ended.iter().filter(|(offered, _)| *offered).count();
    let result = match (mode, answer) {
        ("report", Ok(Outcome::Report(report))) => {
            assert!(matches!(ended.as_slice(), [(false, BuildSessionEnd::Disconnected)]
                | [(false, BuildSessionEnd::ReplayRequired), (true, BuildSessionEnd::Disconnected)]));
            let mut buffer = Buffer::no_color();
            report.emit_to(&mut buffer)?;
            serde_json::json!({ "kind": "report", "response": report.response(),
                "rendered": std::str::from_utf8(buffer.as_slice())? })
        }
        ("stale", Ok(Outcome::StaleSelection)) => {
            assert_eq!(ended, [(false, BuildSessionEnd::ReplayRequired)]);
            assert!(store.snapshot(initial.unwrap().selection().root)?.is_none());
            serde_json::json!({ "kind": "stale" })
        }
        ("ignore-replay", Ok(Outcome::ReplayNotAccepted)) => {
            assert_eq!(ended, [(false, BuildSessionEnd::ReplayRequired), (true, BuildSessionEnd::ReplayRequired)]);
            assert!(store.snapshot(initial.unwrap().selection().root)?.is_none());
            serde_json::json!({ "kind": "replay-not-accepted" })
        }
        ("none", Ok(Outcome::NotAcknowledged)) => {
            assert!(ended.is_empty());
            serde_json::json!({ "kind": "none" })
        }
        ("changed", Ok(Outcome::WorkspaceChanged)) | ("missing-profile", Ok(Outcome::ProfileNotSelected)) => {
            assert!(live && ended.is_empty());
            assert_eq!(calls.get(), 0);
            serde_json::json!({ "kind": mode })
        }
        ("record-budget" | "byte-budget" | "authored-budget", Err(error)) => {
            assert!(live && ended.is_empty());
            assert_eq!(calls.get(), 0);
            let reason = match mode {
                "record-budget" => "live source index exceeds the record budget",
                "byte-budget" => "live source index exceeds the byte budget",
                _ => "live authored source exceeds the byte budget",
            };
            assert!(error.to_string().contains(reason), "{error}");
            serde_json::json!({ "kind": mode })
        }
        ("expired", Err(error)) => {
            assert!(ended.is_empty());
            assert_eq!(calls.get(), 0);
            assert_eq!(error.downcast_ref::<std::io::Error>().map(std::io::Error::kind), Some(std::io::ErrorKind::TimedOut));
            serde_json::json!({ "kind": "expired" })
        }
        ("late-replay", Err(error)) => {
            assert_eq!(ended, [(false, BuildSessionEnd::ReplayRequired)]);
            assert_eq!(calls.get(), 2);
            assert_eq!(error.downcast_ref::<std::io::Error>().map(std::io::Error::kind), Some(std::io::ErrorKind::TimedOut));
            assert!(store.snapshot(initial.unwrap().selection().root)?.is_none());
            serde_json::json!({ "kind": "replay-timed-out" })
        }
        (_, Err(error)) => return Err(error),
        _ => return Err("fixture terminal report outcome did not match scope".into()),
    };
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its graph").close().await?;
    Ok(serde_json::json!({ "answer": result, "streams_opened": ended.len(),
        "adapter_calls": calls.get(), "recovery_payloads": offered, "elapsed_ms": elapsed }))
}
