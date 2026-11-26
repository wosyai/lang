//! Local test adapter for named historical diagnostics, not a public command.
use super::{milliseconds, Result};
use codespan_reporting::term::termcolor::Buffer;
use std::{collections::HashMap, io::Write, num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_daemon::{SavedBuildDiagnostic, SourceStorageWorker, SourceStore};
use wosy_project::PackageRelativePath;
use wosy_protocol::{BuildSourceSelection, FileKey};

fn rendered(store: &SourceStore, report: &SavedBuildDiagnostic) -> Result<String> {
    let source = store.snapshot(report.source_root)?.ok_or("saved fixture source root is missing")?;
    let mut files = HashMap::new();
    for label in report.diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels) {
        let requested = &label.span.source;
        if files.contains_key(requested) { continue; }
        let key = FileKey { package: requested.package.clone(), path: PackageRelativePath::new(requested.path.clone().into())? };
        let file = source.authored(&key)?;
        if file.cst.source != *requested { return Err("saved fixture diagnostic revision mismatch".into()); }
        files.insert(requested.clone(), file.cst.root.text().to_string());
    }
    let mut buffer = Buffer::no_color();
    for diagnostic in &report.diagnostics {
        wosy_cli::diagnostic_output::emit_project_diagnostic(&mut buffer, diagnostic, |source| files.get(source).map(String::as_str))?;
    }
    Ok(std::str::from_utf8(buffer.as_slice())?.to_owned())
}

pub(super) async fn run(
    dir: &Path, selected: BuildSourceSelection, name: &str, profile: Option<&str>, max_bytes: usize, exit: bool,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(dir, 8 << 20)?);
    let bound = NonZeroUsize::new(1).unwrap();
    let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), bound, bound, bound)?;
    let session = worker.open_build_diagnostics(selected, Duration::from_secs(30)).await?
        .ok_or("fixture requires the exact published source")?;
    let limits = super::source_reconciliation::admission_limits(false);
    let start = Instant::now();
    let result = match profile {
        Some(profile) => Some(session.save_build_diagnostic(profile, name, limits, max_bytes).await?),
        None => session.saved_build_diagnostic(name, limits, max_bytes).await?,
    };
    let operation_ms = milliseconds(start);
    let rendered = result.as_ref().map(|report| rendered(&store, report)).transpose()?;
    let value = serde_json::json!({ "report": result, "rendered": rendered, "operation_ms": operation_ms });
    if exit {
        println!("{value}");
        std::io::stdout().flush()?;
        // The acknowledgment was returned; intentionally do not close the store.
        std::process::exit(0);
    }
    session.release();
    drop(worker);
    Arc::into_inner(store).ok_or("fixture has a retained storage worker")?.close().await?;
    Ok(value)
}
