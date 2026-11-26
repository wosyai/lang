use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_cli::source_state::{PendingBasePublication, ReplaySource, SqliteSourceState};
use wosy_daemon::{
    BaseCstAdmissionLimits, RootedLlvmPartition, RootedLlvmProjection, RootedLlvmRejection, RootedLlvmUnsupported,
    SourceStorageWorker, SourceStore,
};
use wosy_protocol::BuildSourceSelection;

pub(super) async fn run(
    path: &Path, selected: BuildSourceSelection, profile: String, output: usize, source: &str,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;
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
                "grammar_verified_files": ll.grammar_verified_files, "grammar_reparsed_files": ll.grammar_reparsed_files })
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

fn ordinary_worker(store: &Arc<SourceStore>) -> Result<SourceStorageWorker> {
    let bound = |n| NonZeroUsize::new(n).unwrap();
    SourceStorageWorker::new(
        Arc::clone(store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )
}

async fn memo_projection(
    worker: &SourceStorageWorker, selected: &BuildSourceSelection, profile: &str,
    limits: BaseCstAdmissionLimits,
) -> Result<(RootedLlvmPartition, f64)> {
    let lease = worker.open_build_diagnostics(selected.clone(), Duration::from_secs(30)).await?
        .ok_or("memo fixture needs an authenticated source lease")?;
    let start = Instant::now();
    let projected = lease.project_ordinary_llvm(profile.to_owned(), limits, 2 << 20).await?;
    let elapsed = milliseconds(start);
    lease.release();
    match projected {
        RootedLlvmProjection::Emitted(partition) => Ok((partition, elapsed)),
        _ => Err("memo fixture expected an ordinary LLVM partition".into()),
    }
}

pub(super) async fn run_memo(
    local: &mut SqliteSourceState, path: &Path, target: &str,
) -> Result<serde_json::Value> {
    let a = local.acknowledged_build_source(target)?.ok_or("memo fixture needs A")?;
    let pending = local.selection(target)?.pending.ok_or("memo fixture needs pending B")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;
    let limits = admission_limits(false);
    let (cold_a, cold_a_ms) = memo_projection(&worker, &a, "dev", limits).await?;
    let total = cold_a.grammar_verified_files;
    assert_eq!(cold_a.grammar_reparsed_files, total, "legacy publication is not a grammar receipt");
    let (warm_a, warm_a_ms) = memo_projection(&worker, &a, "dev", limits).await?;
    assert_eq!(warm_a.grammar_reparsed_files, 0);
    assert_eq!(cold_a.bytes, warm_a.bytes);
    assert!(memo_projection(&worker, &a, "dev", admission_limits(true))
        .await.unwrap_err().to_string().contains("authored byte budget"));

    // Use the real acknowledged predecessor CAS and changed-file grammar
    // transaction, not a fixture's marker.
    let PendingBasePublication::Applied(applied) = local.publish_pending_base(target, pending, |binding, proposal| {
        worker.publish_bound_base_source(binding, proposal, limits, Duration::from_secs(30))
    }).await? else { return Err("memo fixture did not publish the hot dependency".into()); };
    assert_eq!(applied.validated_files, 1);
    let b = local.acknowledged_build_source(target)?.ok_or("memo fixture lost B")?;
    assert_eq!(b.root, pending.root());
    let (hot_b, hot_b_ms) = memo_projection(&worker, &b, "dev", limits).await?;
    assert_eq!((hot_b.grammar_verified_files, hot_b.grammar_reparsed_files), (total, 0));
    assert_ne!(hot_b.digest, cold_a.digest);
    let (native_b, _) = memo_projection(&worker, &b, "native", limits).await?;
    assert_eq!((native_b.grammar_verified_files, native_b.grammar_reparsed_files), (total, 0));
    drop(worker);
    Arc::try_unwrap(store).ok().expect("memo fixture owns its store").close().await?;

    // Same executable image; a genuinely reopened graph has no live receipt.
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;
    let (reopened, reopen_ms) = memo_projection(&worker, &b, "dev", limits).await?;
    assert_eq!((reopened.grammar_verified_files, reopened.grammar_reparsed_files), (total, total));
    assert_eq!(reopened.bytes, hot_b.bytes);
    assert_eq!(reopened.compiler_image, hot_b.compiler_image);
    drop(worker);
    Arc::try_unwrap(store).ok().expect("memo fixture owns reopened store").close().await?;

    Ok(serde_json::json!({
        "root": b.root.hex(), "files": total, "hot_admission_reparsed": applied.validated_files,
        "cold_a": [cold_a_ms, cold_a.grammar_verification.as_secs_f64() * 1000.0],
        "warm_a": [warm_a_ms, warm_a.grammar_verification.as_secs_f64() * 1000.0],
        "hot_b": [hot_b_ms, hot_b.grammar_verification.as_secs_f64() * 1000.0],
        "reopened_b": [reopen_ms, reopened.grammar_verification.as_secs_f64() * 1000.0],
        "wasi_digest": hot_b.digest.hex(), "native_digest": native_b.digest.hex(),
    }))
}

pub(super) async fn run_policy(
    local: &mut SqliteSourceState, project: &Path, path: &Path, target: &str,
) -> Result<serde_json::Value> {
    let b = local.acknowledged_build_source(target)?.ok_or("policy fixture needs B")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;
    let limits = admission_limits(false);
    let (prior, _) = memo_projection(&worker, &b, "dev", limits).await?;
    let total = prior.grammar_verified_files;
    let manifest = project.join("wosy.toml");
    let before = std::fs::read_to_string(&manifest)?;
    let after = before.replace("[\"WHY\"]", "[\"WHY\", \"OTHER\"]");
    assert_ne!(before, after);
    std::fs::write(&manifest, after)?;
    let staged = local.stage(target)?;
    assert_eq!(staged.parsed, total);
    let pending = staged.pending.ok_or("policy fixture did not produce C")?;
    let (binding, proposal) = local.pending_base_cst_proposal(target, pending)?;
    let checked = worker.validate_base_cst_proposal(binding, proposal, limits, Duration::from_secs(30)).await?;
    assert_eq!(checked.validated_files, total);
    assert!(store.snapshot(staged.root)?.is_none(), "rollback-only validation must not publish C");

    // A legacy submission does not retroactively accept grammar receipts
    // for this exact, truthfully parsed but uncommitted validation of C.
    let full = local.replay(target, ReplaySource::Pending)?.ok_or("policy fixture lost C")?;
    store.publish(full.source.root, &full.source.delta).await?;
    assert!(local.acknowledge(target, pending)?);
    let c = local.acknowledged_build_source(target)?.ok_or("policy fixture lost acknowledged C")?;
    let (curr, _) = memo_projection(&worker, &c, "dev", limits).await?;
    assert_eq!((curr.grammar_verified_files, curr.grammar_reparsed_files), (total, total));
    assert_eq!(prior.bytes, curr.bytes, "policy-only edit preserves ordinary LLVM for this fixture");
    drop(worker);
    Arc::try_unwrap(store).ok().expect("policy fixture owns its store").close().await?;
    Ok(serde_json::json!({ "new_policy_reparsed": total, "rollback_did_not_seed": true }))
}
