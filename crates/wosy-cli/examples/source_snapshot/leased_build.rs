use super::{milliseconds, typed_preparation, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_compiler::Diagnostic;
use wosy_daemon::{SourceStorageWorker, SourceStore};
use wosy_protocol::{BuildDiagnosticRequest, BuildDiagnosticResponse, BuildSourceSelection, BUILD_DIAGNOSTIC_VERSION};

pub(super) async fn run(
    project: &Path, dir: &Path, selected: BuildSourceSelection, profile: &str, oracle: bool,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(dir, 8 << 20)?);
    let bound = |value| NonZeroUsize::new(value).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let root = selected.root;
    let start = Instant::now();
    let session = worker.open_build_diagnostics(selected, Duration::from_secs(30)).await?
        .ok_or("fixture selected source needs reconciliation")?;
    let open_ms = milliseconds(start);
    assert!(session.renew());
    let request_bytes = serde_json::to_vec(&BuildDiagnosticRequest::BuildDiagnostics {
        version: BUILD_DIAGNOSTIC_VERSION, profile: profile.into(),
    })?;
    let start = Instant::now();
    let response = session.dispatch(serde_json::from_slice(&request_bytes)?).await?;
    let dispatch_ms = milliseconds(start);
    let encoded = serde_json::to_vec(&response)?;
    let decoded: BuildDiagnosticResponse<Diagnostic> = serde_json::from_slice(&encoded)?;
    assert_eq!(response, decoded);
    let BuildDiagnosticResponse::BuildDiagnostics { source_root, phase, ref diagnostics, .. } = decoded;
    assert_eq!(source_root, root);
    if oracle { typed_preparation::check_ordinary_build(project, profile, phase, diagnostics)?; }
    session.release();
    drop(worker);
    Arc::try_unwrap(store).map_err(|_| "fixture leased dispatcher still has worker owners")?.close().await?;
    Ok(serde_json::json!({
        "open_ms": open_ms, "dispatch_ms": dispatch_ms,
        "request_bytes": request_bytes.len(), "response_bytes": encoded.len(), "response": response,
    }))
}
