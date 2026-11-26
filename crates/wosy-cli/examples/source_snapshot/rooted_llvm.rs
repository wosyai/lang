use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_daemon::{RootedLlvmProjection, RootedLlvmRejection, RootedLlvmUnsupported, SourceStorageWorker, SourceStore};
use wosy_protocol::BuildSourceSelection;

pub(super) async fn run(
    path: &Path, selected: BuildSourceSelection, profile: String, output: usize, source: &str,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let lifetime = Duration::from_secs(30);
    if source == "wrong-project" {
        let other = path.parent().unwrap().to_str().unwrap().to_owned();
        assert_ne!(selected.project, other);
        let Err(error) = worker.open_build_diagnostics(BuildSourceSelection { project: other, ..selected }, lifetime).await else {
            return Err("fixture admitted a different workspace".into());
        };
        assert_eq!(error.to_string(), "Build diagnostic selection has a different workspace");
        drop(worker);
        Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
        return Ok(serde_json::json!({ "kind": "scope-refused" }));
    }
    let session = worker.open_build_diagnostics(selected.clone(), lifetime).await?
        .ok_or("fixture has no authenticated LLVM source lease")?;
    let mut limits = admission_limits(source == "authored");
    if source == "index" { limits.records = 1; }
    let start = Instant::now();
    let projected = session.project_ordinary_llvm(profile, limits, output).await;
    let elapsed = milliseconds(start);
    session.release();
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    let mut grammar_ms = None;
    let answer = match projected? {
        RootedLlvmProjection::Emitted(ll) => {
            assert_eq!(ll.source_root, selected.root);
            grammar_ms = Some(ll.grammar_verification.as_secs_f64() * 1000.0);
            serde_json::json!({ "kind": "llvm", "root": ll.source_root.hex(), "profile": ll.profile,
                "image": ll.compiler_image.hex(), "digest": ll.digest.hex(), "llvm": String::from_utf8(ll.bytes)?,
                "grammar_verified_files": ll.grammar_verified_files })
        }
        RootedLlvmProjection::Unsupported(unsupported) => match unsupported {
            RootedLlvmUnsupported::ProfileNotSelected => serde_json::json!({ "kind": "profile-not-selected" }),
            RootedLlvmUnsupported::ProfileNotSupported(reason) =>
                serde_json::json!({ "kind": "profile-not-supported", "reason": reason }),
            RootedLlvmUnsupported::LegacySourcePath => serde_json::json!({ "kind": "legacy-source-path" }),
            RootedLlvmUnsupported::CompilerImageUnavailable => serde_json::json!({ "kind": "compiler-image-unavailable" }),
            RootedLlvmUnsupported::AuthoredGrammarNotVerified => serde_json::json!({ "kind": "authored-grammar-not-verified" }),
        }
        RootedLlvmProjection::Rejected(rejected) => match rejected {
            RootedLlvmRejection::Declarations(diagnostics) =>
                serde_json::json!({ "kind": "declarations-rejected", "diagnostics": diagnostics }),
            RootedLlvmRejection::Preparation(diagnostics) =>
                serde_json::json!({ "kind": "preparation-rejected", "diagnostics": diagnostics }),
            RootedLlvmRejection::Analysis(diagnostics) =>
                serde_json::json!({ "kind": "analysis-rejected", "diagnostics": diagnostics }),
            RootedLlvmRejection::Emitter(reason) =>
                serde_json::json!({ "kind": "emitter-rejected", "reason": reason }),
        }
    };
    Ok(serde_json::json!({ "answer": answer, "projection_ms": elapsed, "grammar_ms": grammar_ms }))
}
