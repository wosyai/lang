//! Own each end of two anonymous fixture pipes in a distinct process. This
//! is not a deployment transport; the launcher binds the verified source.
use super::{milliseconds, source_reconciliation::admission_limits, Result};
use codespan_reporting::term::termcolor::Buffer;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io;
use wosy_cli::source_recovery::ClientSourceLimits;
use wosy_cli::source_state::{ProjectDiagnosticExchange, SqliteSourceState};
use wosy_daemon::{SourceStorageWorker, SourceStore};
use wosy_protocol::{BuildSourceSelection, CstExpansionBudget, NodeId};

pub(super) async fn run(mode: &str, args: &mut impl Iterator<Item = String>) -> Result<()> {
    let start = Instant::now();
    let dir = args.next().ok_or("expected fixture directory")?;
    let result = if mode == "--fixture-diagnostic-client" {
        let target = args.next().ok_or("expected fixture target")?;
        let profile = args.next().ok_or("expected fixture profile")?;
        let mut local = SqliteSourceState::open(Path::new(&dir), ClientSourceLimits {
            nodes: 25_000, encoded_bytes: 16 << 20,
        })?;
        let capture = local.capture_build_diagnostics(&target, &profile)?.ok_or("fixture needs acknowledged source")?;
        let stream = io::join(io::stdin(), io::stdout());
        let budget = CstExpansionBudget::new(16 << 20, 200_000, 1024, 8 << 20);
        match capture.exchange_project_diagnostics(&mut local, stream, Duration::from_secs(10), budget).await? {
            ProjectDiagnosticExchange::Report(report) => {
                let mut rendered = Buffer::no_color();
                report.emit_to(&mut rendered)?;
                serde_json::json!({ "kind": "report", "response": report.response(),
                    "rendered": std::str::from_utf8(rendered.as_slice())? })
            }
            ProjectDiagnosticExchange::ReplayRequired(_) => serde_json::json!({ "kind": "replay-required" }),
            _ => return Err("unexpected client outcome in process fixture".into()),
        }
    } else {
        let project = args.next().ok_or("expected host-selected fixture project")?;
        let target = args.next().ok_or("expected host-selected fixture target")?;
        let root = NodeId::parse_hex(&args.next().ok_or("expected host-selected fixture root")?)?;
        let selected = BuildSourceSelection { project, target, root };
        let store = Arc::new(SourceStore::open(Path::new(&dir), 8 << 20)?);
        let bound = |value| NonZeroUsize::new(value).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(),
            bound(1), bound(2), bound(20_000))?;
        let end = worker.serve_recoverable_build_session(io::join(io::stdin(), io::stdout()), selected,
            None, admission_limits(false), Duration::from_secs(8)).await?;
        drop(worker);
        Arc::try_unwrap(store).ok().ok_or("fixture peer still holds the store")?.close().await?;
        serde_json::json!({ "end": format!("{end:?}") })
    };
    // Stdout carries only the actual session bytes. The fixture launcher
    // reads the result side channel after both processes have terminated.
    eprintln!("{}", serde_json::json!({ "pid": std::process::id(), "elapsed_ms": milliseconds(start), "result": result }));
    Ok(())
}
