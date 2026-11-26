use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{num::NonZeroUsize, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_cli::source_state::{PendingBasePublication, ReplaySource, SqliteSourceState};
use wosy_daemon::{
    BaseCstAdmissionLimits, RootedLlvmIdentity, RootedLlvmOrigin, RootedLlvmPartition, RootedLlvmProjection,
    RootedLlvmRejection, RootedLlvmUnsupported,
    SourceStorageWorker, SourceStore,
};
use wosy_protocol::{BuildSourceSelection, NodeId};

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
    let identity = if source == "executable" { RootedLlvmIdentity::Executable } else { RootedLlvmIdentity::Authored };
    let projected = session.project_llvm(profile, limits, output, identity).await;
    let elapsed = milliseconds(start);
    session.release();
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    let mut grammar_ms = None;
    let answer = match projected? {
        RootedLlvmProjection::Emitted(ll) => {
            assert_eq!(ll.source_root, selected.root);
            assert_eq!(ll.identity, identity);
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

pub(super) fn ordinary_worker(store: &Arc<SourceStore>) -> Result<SourceStorageWorker> {
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

pub(super) async fn run_executable_receipts(
    local: &SqliteSourceState, path: &Path, target: &str, history: [NodeId; 3],
) -> Result<serde_json::Value> {
    let d = local.acknowledged_build_source(target)?.ok_or("fixture needs the acknowledged alternate export")?;
    let [a, b, c] = history.map(|root| BuildSourceSelection { root, ..d.clone() });
    assert_ne!(a.root, b.root);
    assert_ne!(b.root, c.root);
    assert_ne!(c.root, d.root);
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;

    let fresh = RootedLlvmOrigin::Derived;
    let cached = RootedLlvmOrigin::LiveExecutableReceipt;
    let should_refuse = |error: super::Result<_>, reason: &str| {
        assert!(error.unwrap_err().to_string().contains(reason));
    };
    should_refuse(executable_projection(&worker, &a, "dev", admission_limits(false), 1).await,
        "output byte budget");
    let (cold_a, cold_ms) = executable_projection(&worker, &a, "dev", admission_limits(false), 2 << 20).await?;
    assert_eq!(cold_a.origin, fresh);
    let total = cold_a.grammar_verified_files;
    assert_eq!(cold_a.grammar_reparsed_files, total, "over-budget egress cannot seed either receipt");
    let (same_a, same_ms) = executable_projection(&worker, &a, "dev", admission_limits(false), 2 << 20).await?;
    assert_eq!((same_a.origin, same_a.grammar_reparsed_files), (cached, 0));
    assert_eq!(cold_a.bytes, same_a.bytes);
    let (native_a, _) = executable_projection(&worker, &a, "native", admission_limits(false), 2 << 20).await?;
    assert_eq!(native_a.origin, fresh, "WASI cannot seed Native");
    assert_ne!(cold_a.digest, native_a.digest);

    should_refuse(executable_projection(&worker, &b, "dev", admission_limits(true), 2 << 20).await,
        "authored byte budget");
    let (authored_b, edited_ms) = executable_projection(&worker, &b, "dev", admission_limits(false), 2 << 20).await?;
    assert_eq!((authored_b.origin, authored_b.source_root), (cached, b.root));
    assert_eq!((authored_b.grammar_verified_files, authored_b.grammar_reparsed_files), (total, 1));
    assert_eq!((authored_b.digest, authored_b.bytes), (cold_a.digest, cold_a.bytes));
    should_refuse(executable_projection(&worker, &b, "dev", admission_limits(false), 1).await,
        "output byte budget");
    let (native_b, _) = executable_projection(&worker, &b, "native", admission_limits(false), 2 << 20).await?;
    assert_eq!((native_b.origin, native_b.grammar_reparsed_files), (cached, 0));
    assert_eq!(native_b.bytes, native_a.bytes);

    let (executable_c, changed_ms) = executable_projection(&worker, &c, "dev", admission_limits(false), 2 << 20).await?;
    assert_eq!((executable_c.origin, executable_c.grammar_reparsed_files), (fresh, 1));
    assert_ne!(executable_c.digest, cold_a.digest);
    let (export_d, _) = executable_projection(&worker, &d, "dev", admission_limits(false), 2 << 20).await?;
    assert_eq!((export_d.origin, export_d.grammar_reparsed_files), (fresh, 0));
    assert_ne!(export_d.digest, executable_c.digest);
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its source store").close().await?;

    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;
    let (reopened, reopen_ms) = executable_projection(&worker, &d, "dev", admission_limits(false), 2 << 20).await?;
    assert_eq!((reopened.origin, reopened.grammar_reparsed_files), (fresh, total));
    assert_eq!(reopened.bytes, export_d.bytes);
    assert_eq!(reopened.compiler_image, export_d.compiler_image);
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns reopened store").close().await?;
    Ok(serde_json::json!({ "files": total, "annotated_reparsed": authored_b.grammar_reparsed_files,
        "cold_derived_ms": cold_ms, "same_live_ms": same_ms, "annotation_live_ms": edited_ms,
        "changed_derived_ms": changed_ms, "reopened_derived_ms": reopen_ms,
        "profile_dependency_export_and_reopen_distinct": true, "hit_source_and_output_budgets": true }))
}

async fn executable_projection(
    worker: &SourceStorageWorker, selection: &BuildSourceSelection, profile: &str,
    limits: BaseCstAdmissionLimits, output: usize,
) -> Result<(RootedLlvmPartition, f64)> {
    let lease = worker.open_build_diagnostics(selection.clone(), Duration::from_secs(30)).await?
        .ok_or("fixture needs an acknowledged source lease")?;
    let start = Instant::now();
    let outcome = lease.project_llvm(profile.to_owned(), limits, output, RootedLlvmIdentity::Executable).await?;
    match outcome {
        RootedLlvmProjection::Emitted(partition) => Ok((partition, milliseconds(start))),
        _ => Err("receipt fixture expected an executable LLVM projection".into()),
    }
}
