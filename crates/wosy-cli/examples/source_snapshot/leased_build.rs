use super::{milliseconds, typed_preparation, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_compiler::Diagnostic;
use tokio::io::{self, AsyncWriteExt};
use wosy_daemon::{encode_session_frame, BuildSessionEnd, SessionFrameReader, SourceStorageWorker, SourceStore};
use wosy_protocol::{
    BuildDiagnosticRequest, BuildDiagnosticResponse, BuildSessionCommand, BuildSessionCommandId,
    BuildSessionReply, BuildSessionRequest, BuildSessionResponse,
    BuildSourceSelection, BUILD_DIAGNOSTIC_VERSION,
};

pub(super) async fn run(
    project: &Path, dir: &Path, selected: BuildSourceSelection, profile: &str, oracle: bool, framed: bool,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(dir, 8 << 20)?);
    let bound = |value| NonZeroUsize::new(value).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let root = selected.root;
    let (response, open_ms, dispatch_ms, request_bytes, response_bytes) = if framed {
        through_codec(&worker, selected, profile).await?
    } else {
        let start = Instant::now();
        let session = worker.open_build_diagnostics(selected, Duration::from_secs(30)).await?
            .ok_or("fixture selected source needs reconciliation")?;
        let open_ms = milliseconds(start);
        assert!(session.renew());
        let request_bytes = serde_json::to_vec(&analyze(profile))?;
        let start = Instant::now();
        let response = session.dispatch(serde_json::from_slice(&request_bytes)?).await?;
        let dispatch_ms = milliseconds(start);
        let encoded = serde_json::to_vec(&response)?;
        let decoded: BuildDiagnosticResponse<Diagnostic> = serde_json::from_slice(&encoded)?;
        assert_eq!(response, decoded);
        session.release();
        (decoded, open_ms, dispatch_ms, request_bytes.len(), encoded.len())
    };
    let BuildDiagnosticResponse::BuildDiagnostics { source_root, phase, ref diagnostics, .. } = response;
    assert_eq!(source_root, root);
    if oracle { typed_preparation::check_ordinary_build(project, profile, phase, diagnostics)?; }
    drop(worker);
    Arc::try_unwrap(store).map_err(|_| "fixture leased dispatcher still has worker owners")?.close().await?;
    Ok(serde_json::json!({
        "open_ms": open_ms, "dispatch_ms": dispatch_ms,
        "request_bytes": request_bytes, "response_bytes": response_bytes, "response": response, "framed": framed,
    }))
}

fn analyze(profile: &str) -> BuildDiagnosticRequest {
    BuildDiagnosticRequest::BuildDiagnostics { version: BUILD_DIAGNOSTIC_VERSION, profile: profile.into() }
}

async fn through_codec(
    worker: &SourceStorageWorker, selected: BuildSourceSelection, profile: &str,
) -> Result<(BuildDiagnosticResponse<Diagnostic>, f64, f64, usize, usize)> {
    let (mut client, stream) = io::duplex(256);
    let root = selected.root;
    let session = worker.serve_build_session(stream, selected, Duration::from_secs(30));
    let request = async {
        let mut frames = SessionFrameReader::default();
        let start = Instant::now();
        let open = BuildSessionCommand {
            id: BuildSessionCommandId(1), request: BuildSessionRequest::Start { version: BUILD_DIAGNOSTIC_VERSION },
        };
        client.write_all(&encode_session_frame(&open)?).await?;
        match frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await? {
            Some(BuildSessionResponse { id: BuildSessionCommandId(1), reply:
                BuildSessionReply::Started { version: BUILD_DIAGNOSTIC_VERSION, source_root }
            }) => assert_eq!(source_root, root),
            _ => return Err("fixture did not start the framed session".into()),
        }
        let open_ms = milliseconds(start);
        let command = encode_session_frame(&BuildSessionCommand {
            id: BuildSessionCommandId(2), request: BuildSessionRequest::Analyze { request: analyze(profile) },
        })?;
        let start = Instant::now();
        client.write_all(&command).await?;
        let Some(message @ BuildSessionResponse { id: BuildSessionCommandId(2), reply: BuildSessionReply::Analysis { .. } }) =
            frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await?
            else { return Err("fixture did not receive framed diagnostics".into()); };
        let dispatch_ms = milliseconds(start);
        let bytes = encode_session_frame(&message)?.len();
        let BuildSessionReply::Analysis { response } = message.reply else { unreachable!() };
        let close = BuildSessionCommand { id: BuildSessionCommandId(3), request: BuildSessionRequest::Disconnect };
        client.write_all(&encode_session_frame(&close)?).await?;
        assert!(matches!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await?,
            Some(BuildSessionResponse { id: BuildSessionCommandId(3), reply: BuildSessionReply::Closed })));
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((response, open_ms, dispatch_ms, command.len(), bytes))
    };
    let (end, response) = tokio::join!(session, request);
    assert_eq!(end?, BuildSessionEnd::Disconnected);
    response
}
