use std::{fs, num::NonZeroUsize, path::PathBuf, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::{PendingBasePublication, ReplaySource, SourceAttempt, SqliteSourceState}};
use wosy_daemon::{AuthoredComptimeCoverage, BaseCstAdmissionLimits, RootedLlvmProjection, SourceReconciliation, SourceStorageWorker, SourceStore};
use wosy_protocol::SourcePinReflection;

const CLIENT: ClientSourceLimits = ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 };
const LIMITS: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
    records: 25_000, record_bytes: 16 << 20, graph_edges: 100_000, expanded_record_bytes: 32 << 20,
    cst_occurrences: 100_000, cst_depth: 128, authored_bytes: 2 << 20,
};
const TTL: Duration = Duration::from_secs(30);

const MANIFEST: &str = r#"
[package]
name = "app"
default_target = "app"
[comments]
categories = ["WHY"]
[builders.nominal]
command = ["fixture-must-not-build"]
toolchain_identity = "source-only fixture"
[runners.nominal]
command = ["fixture-must-not-run"]
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "Main"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "nominal"
run_runner = "nominal"
[targets.app.profiles.dev]
main_artifact = "main"
"#;
const BEFORE: &str = "%%start\nlib = namespace app \"library.w\";\ni32 value = lib.answer;\n%%end\n";
const EDITED: &str = "%%start\nlib = namespace app \"library.w\";\ni32 value = 14;\n%%end\n";
// A comment mentioning the keyword is not an authored comptime node.
const ORDINARY_LIB: &str = "%%start\n# WHY: comptime is not used here\ni32 answer = 14;\n%%end\n";
const COMPTIME_LIB: &str =
    "%%start\ni32 answer = comptime fixture { i32 seed = 7; seed ^ 9 };\n%%end\n";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("wosy-authored-comptime-coverage-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

async fn coverage(
    client: &mut SqliteSourceState, worker: &SourceStorageWorker, store: &SourceStore,
    attempt: SourceAttempt,
) -> AuthoredComptimeCoverage {
    let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
    let owner = worker.prepare_base_cst_declarations(binding, proposal, LIMITS, 1 << 20, TTL).await.unwrap();
    assert_eq!(owner.modules().len(), 2);
    // An unchanged imported file must never trigger a full parser rerun.
    assert_eq!(owner.admission().validated_files, 1);
    assert_eq!(owner.admission().attempt, attempt.generation());
    assert_eq!(owner.admission().proposed, attempt.root());
    assert!(store.snapshot(attempt.root()).unwrap().is_none());
    owner.authored_comptime_coverage().unwrap()
}

#[test]
fn prepared_coverage_requires_authentic_reused_grammar_and_tracks_exact_edits() {
    let fixture = Scratch::new();
    let root = fixture.0.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("wosy.toml"), MANIFEST).unwrap();
    let entry = root.join("main.w");
    let library = root.join("library.w");
    fs::write(&entry, BEFORE).unwrap();
    fs::write(&library, ORDINARY_LIB).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    rt.block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&root, CLIENT).unwrap();
        let base = client.stage("app").unwrap();
        assert_eq!(base.parsed, 2);
        let initial = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(base.root, &initial.source.delta).await.unwrap();
        assert!(client.acknowledge("app", base.pending.unwrap()).unwrap());
        let prior = client.acknowledged_build_source("app").unwrap().unwrap();

        fs::write(&entry, EDITED).unwrap();
        let pending = client.stage("app").unwrap();
        assert_eq!((pending.parsed, pending.reused), (1, 1));
        let attempt = pending.pending.unwrap();
        // The committed predecessor marker alone is not grammar evidence.
        assert_eq!(coverage(&mut client, &worker, &store, attempt).await, AuthoredComptimeCoverage::Unverified);
        // Existing authentic authored LLVM projection checks the original
        // grammar with the ordinary parser. No linker or runner is invoked.
        let lease = worker.open_build_diagnostics(prior, TTL).await.unwrap().unwrap();
        let RootedLlvmProjection::Emitted(llvm) = lease.project_ordinary_llvm("dev".into(), LIMITS, 1 << 20).await.unwrap()
            else { panic!("initial source must emit ordinary LLVM text"); };
        assert_eq!((llvm.grammar_verified_files, llvm.grammar_reparsed_files), (2, 2));
        lease.release();
        assert_eq!(coverage(&mut client, &worker, &store, attempt).await, AuthoredComptimeCoverage::Absent);
        assert!(client.abandon("app", attempt).unwrap());

        fs::write(&entry, BEFORE).unwrap();
        fs::write(&library, COMPTIME_LIB).unwrap();
        let pending = client.stage("app").unwrap();
        assert_eq!((pending.parsed, pending.reused), (1, 1));
        let added = pending.pending.unwrap();
        assert_eq!(coverage(&mut client, &worker, &store, added).await, AuthoredComptimeCoverage::Present);
        // Legitimate source publication only, not a published result.
        let applied = client.publish_pending_base("app", added, |binding, proposal| {
            worker.publish_bound_base_source(binding, proposal, LIMITS, TTL)
        }).await.unwrap();
        let PendingBasePublication::Applied(receipt) = applied else { panic!("expected source-only commit") };
        assert_eq!(receipt.validated_files, 1);

        fs::write(&entry, EDITED).unwrap();
        let edited = client.stage("app").unwrap();
        assert_eq!((edited.parsed, edited.reused), (1, 1));
        let reused = edited.pending.unwrap();
        // The unchanged dependency now has a committed authentic Present
        // receipt, so editing the entry without comptime cannot make it empty.
        assert_eq!(coverage(&mut client, &worker, &store, reused).await, AuthoredComptimeCoverage::Present);
        assert!(client.abandon("app", reused).unwrap());

        fs::write(&entry, BEFORE).unwrap();
        // A fresh non-comptime content revision is distinct from all
        // previously published roots, so the preparation has no publish hit.
        fs::write(&library, ORDINARY_LIB.replace("= 14", "= 19")).unwrap();
        let removed = client.stage("app").unwrap().pending.unwrap();
        assert_eq!(coverage(&mut client, &worker, &store, removed).await, AuthoredComptimeCoverage::Absent);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[test]
fn recovered_grammar_is_available_to_sparse_preparation_only_after_commit() {
    let fixture = Scratch::new();
    let root = fixture.0.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("wosy.toml"), MANIFEST).unwrap();
    fs::write(root.join("main.w"), BEFORE).unwrap();
    let library = root.join("library.w");
    fs::write(&library, ORDINARY_LIB).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    rt.block_on(async {
        let one = NonZeroUsize::new(1).unwrap();
        let worker_for = |store: &Arc<SourceStore>| {
            SourceStorageWorker::new(Arc::clone(store), tokio::runtime::Handle::current(), one, one, one).unwrap()
        };
        let prior_store = SourceStore::open(&fixture.0.join("prior"), 8 << 20).unwrap();
        let mut client = SqliteSourceState::open(&root, CLIENT).unwrap();
        let base = client.stage("app").unwrap();
        let pending = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        prior_store.publish(base.root, &pending.source.delta).await.unwrap();
        assert!(client.acknowledge("app", base.pending.unwrap()).unwrap());
        prior_store.close().await.unwrap();
        drop(client);

        // Reopen the actual acknowledged SQLite selection. The daemon only
        // receives its immutable delta, even though the live dependency changed.
        let mut client = SqliteSourceState::open(&root, CLIENT).unwrap();
        let selected = client.acknowledged_build_source("app").unwrap().unwrap();
        let replay = client.replay_acknowledged_source(&selected).unwrap().unwrap();
        assert_eq!(selected.root, base.root);
        assert_eq!((replay.source.parsed, replay.source.reused), (0, 0));
        let pin = "retained-before-edit";
        client.pin_selected_source("app", pin).unwrap();
        let intent = client.source_pin_intent(pin).unwrap().unwrap();
        let pinned_replay = client.replay_source_pin_intent(&intent).unwrap().unwrap();
        assert_eq!(pinned_replay.source.root, selected.root);
        fs::write(&library, ORDINARY_LIB.replace("= 14", "= 19")).unwrap();
        let edited = client.stage("app").unwrap();
        assert_eq!((edited.parsed, edited.reused), (1, 1));
        let attempt = edited.pending.unwrap();

        let lost = fixture.0.join("replacement");
        let store = Arc::new(SourceStore::open(&lost, 8 << 20).unwrap());
        let worker = worker_for(&store);
        assert!(matches!(worker.reconcile_acknowledged_source(selected.clone(), None, LIMITS).await.unwrap(),
            SourceReconciliation::ReplayRequired { root } if root == selected.root));
        let restored = worker.reconcile_acknowledged_source(selected.clone(), Some(replay.source.delta), LIMITS).await.unwrap();
        assert!(matches!(restored, SourceReconciliation::Restored { root, validated_files: 2, .. }
            if root == selected.root));
        assert_eq!(coverage(&mut client, &worker, &store, attempt).await, AuthoredComptimeCoverage::Absent);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();

        // Reopening an existing marker does not restore a process-local memo.
        let store = Arc::new(SourceStore::open(&lost, 8 << 20).unwrap());
        let worker = worker_for(&store);
        assert!(matches!(worker.reconcile_acknowledged_source(selected.clone(), None, LIMITS).await.unwrap(),
            SourceReconciliation::Known { root } if root == selected.root));
        assert_eq!(coverage(&mut client, &worker, &store, attempt).await, AuthoredComptimeCoverage::Unverified);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();

        // Explicit named-pin restore takes the same canonical parser path,
        // but installs its evidence only after the existing immediate pin transaction.
        let store = Arc::new(SourceStore::open(&fixture.0.join("pin-replacement"), 8 << 20).unwrap());
        let worker = worker_for(&store);
        assert_eq!(worker.reflect_source_pin(intent.clone(), None, LIMITS).await.unwrap(), SourcePinReflection::ReplayRequired);
        let reflected = worker.reflect_source_pin(intent.clone(), Some(pinned_replay.source.delta), LIMITS).await.unwrap();
        assert_eq!(reflected, SourcePinReflection::Durable(intent));
        assert!(client.confirms_source_pin_reflection(&reflected).unwrap());
        assert_eq!(coverage(&mut client, &worker, &store, attempt).await, AuthoredComptimeCoverage::Absent);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();

        let store = Arc::new(SourceStore::open(&fixture.0.join("rejected"), 8 << 20).unwrap());
        let worker = worker_for(&store);
        // Canonical graph traversal sees the exact entry first. This allows
        // its full parse, but not the second file. Discard the whole staged batch.
        let strict = BaseCstAdmissionLimits { authored_bytes: BEFORE.len(), ..LIMITS };
        let replay = client.replay_acknowledged_source(&selected).unwrap().unwrap();
        let denied = worker.reconcile_acknowledged_source(selected.clone(), Some(replay.source.delta), strict)
            .await.err().unwrap();
        assert!(denied.to_string().contains("authored byte budget"));
        assert!(store.snapshot(selected.root).unwrap().is_none());
        // Independent ordinary baseline publication is not grammar evidence.
        // The edited dependency is checked, but a leaked entry witness would
        // incorrectly turn this unsupported predecessor into proven absence.
        let replay = client.replay_acknowledged_source(&selected).unwrap().unwrap();
        store.publish(selected.root, &replay.source.delta).await.unwrap();
        assert_eq!(coverage(&mut client, &worker, &store, attempt).await, AuthoredComptimeCoverage::Unverified);
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap(), selected);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn changed_pending_entry_reuses_verified_parse_for_first_comptime_fragment() {
    use wosy_compiler::{ClosedComptimeRejection, ScalarTargetLayout, ScalarType};
    use wosy_daemon::{PendingFirstComptimeProjection, RootedClosedComptimeProjection,
        RootedClosedComptimeRequest, RootedClosedComptimeRefusal};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::FileKey;

    const SOURCE: &str = "%%start\n# WHY: é literal\ni32 answer = comptime fixture { i32 seed = 7; seed ^ 9 };\n%%end\n";
    let fixture = Scratch::new();
    let root = fixture.0.join("project");
    fs::create_dir(&root).unwrap();
    let manifest = root.join("wosy.toml");
    let entry = root.join("main.w");
    fs::write(&manifest, MANIFEST).unwrap();
    fs::write(&entry, "%%start\ni32 answer = 14;\n%%end\n").unwrap();
    let request = |layout| RootedClosedComptimeRequest {
        profile: "dev".into(),
        file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
        id: "fixture".into(), evaluator_layout: layout, source_limits: LIMITS, output_bytes: 1 << 20,
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    rt.block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&root, CLIENT).unwrap();
        let base = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(base.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", base.pending.unwrap()).unwrap());
        let prior = client.acknowledged_build_source("app").unwrap().unwrap();

        fs::write(&entry, SOURCE).unwrap();
        let stage = client.stage("app").unwrap();
        let pending = stage.pending.unwrap();
        let mut previews = Vec::new();
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let (binding, proposal) = client.pending_base_cst_proposal("app", pending).unwrap();
            let prepared = worker.prepare_pending_first_comptime(binding, proposal, request(layout), 1 << 20, TTL)
                .await.unwrap();
            let admitted = prepared.base().admission();
            assert_eq!((admitted.base, admitted.proposed, admitted.attempt, admitted.validated_files),
                (prior.root, pending.root(), pending.generation(), 1));
            assert_eq!(prepared.base().authored_comptime_coverage(), Some(AuthoredComptimeCoverage::Present));
            let Some(PendingFirstComptimeProjection::Candidate(RootedClosedComptimeProjection::Emitted(fragment))) =
                prepared.projection() else { panic!("expected first entered comptime fragment"); };
            let range = fragment.site.range;
            assert_eq!(&SOURCE[range.start as usize..range.end as usize],
                "comptime fixture { i32 seed = 7; seed ^ 9 }");
            assert_eq!(fragment.source_root, pending.root());
            assert_eq!(fragment.result_type, ScalarType::I32);
            assert_eq!(fragment.evaluator_layout, layout);
            previews.push((fragment.bytes.clone(), fragment.digest, fragment.site.clone(), fragment.authored_cst,
                fragment.compiler_image));
        }
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap(), prior);
        assert!(store.snapshot(pending.root()).unwrap().is_none());

        let (binding, proposal) = client.pending_base_cst_proposal("app", pending).unwrap();
        let mut too_small = request(ScalarTargetLayout::NATIVE64);
        too_small.output_bytes = 1;
        let error = worker.prepare_pending_first_comptime(binding, proposal, too_small, 1 << 20, TTL).await.err().unwrap();
        assert!(error.to_string().contains("output byte budget"));
        assert!(store.snapshot(pending.root()).unwrap().is_none());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap(), prior);

        let committed = client.publish_pending_base("app", pending, |binding, proposal| {
            worker.publish_bound_base_source(binding, proposal, LIMITS, TTL)
        }).await.unwrap();
        assert!(matches!(committed, PendingBasePublication::Applied(_)));
        let current = client.acknowledged_build_source("app").unwrap().unwrap();
        let published = worker.open_build_diagnostics(current.clone(), TTL).await.unwrap().unwrap();
        for (layout, preview) in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32].into_iter().zip(previews) {
            let RootedClosedComptimeProjection::Emitted(fragment) = published.project_first_entry_comptime(request(layout)).await.unwrap()
                else { panic!("published entry must yield the same ordinary fragment"); };
            assert_eq!(preview, (fragment.bytes, fragment.digest, fragment.site, fragment.authored_cst, fragment.compiler_image));
        }
        published.release();

        fs::write(&manifest, MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")).unwrap();
        let staged = client.stage("app").unwrap();
        assert_eq!((staged.parsed, staged.reused), (0, 1));
        let attempt = staged.pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let prepared = worker.prepare_pending_first_comptime(binding, proposal, request(ScalarTargetLayout::WASM32), 1 << 20, TTL)
            .await.unwrap();
        assert_eq!(prepared.base().admission().validated_files, 0);
        assert!(matches!(prepared.projection(), Some(PendingFirstComptimeProjection::EntryUnchanged)));
        drop(prepared);
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert!(client.abandon("app", attempt).unwrap());

        fs::write(&manifest, MANIFEST).unwrap();
        fs::write(&entry, SOURCE.replace("i32 answer", "i32 earlier = 41;\ni32 answer")).unwrap();
        let attempt = client.stage("app").unwrap().pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let prepared = worker.prepare_pending_first_comptime(binding, proposal, request(ScalarTargetLayout::NATIVE64), 1 << 20, TTL)
            .await.unwrap();
        assert!(matches!(prepared.projection(), Some(PendingFirstComptimeProjection::Candidate(
            RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::Compiler(
                ClosedComptimeRejection::PrecededByAuthoredItem { .. }))))));
        drop(prepared);
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert!(client.abandon("app", attempt).unwrap());

        fs::write(&entry, SOURCE.replace("i32 seed = 7; seed ^ 9", "core.comp_param(\"limit\", 7)")).unwrap();
        let attempt = client.stage("app").unwrap().pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let prepared = worker.prepare_pending_first_comptime(binding, proposal, request(ScalarTargetLayout::NATIVE64), 1 << 20, TTL)
            .await.unwrap();
        assert!(matches!(prepared.projection(), Some(PendingFirstComptimeProjection::Candidate(
            RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::ParameterAuthorityUnavailable { name, .. })))
            if name == "limit"));
        drop(prepared);
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap(), current);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn exact_pending_owner_can_commit_source_without_a_second_parser_run() {
    use wosy_compiler::ScalarTargetLayout;
    use wosy_daemon::{PendingFirstComptimeProjection, PreparedPendingFirstComptime,
        RootedClosedComptimeProjection, RootedClosedComptimeRequest};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::FileKey;

    async fn prepare(
        client: &mut SqliteSourceState, worker: &SourceStorageWorker, attempt: SourceAttempt, lifetime: Duration,
    ) -> PreparedPendingFirstComptime {
        let request = RootedClosedComptimeRequest {
            profile: "dev".into(),
            file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
            id: "fixture".into(), evaluator_layout: ScalarTargetLayout::NATIVE64, source_limits: LIMITS, output_bytes: 1 << 20,
        };
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let owner = worker.prepare_pending_first_comptime(binding, proposal, request, 1 << 20, lifetime).await.unwrap();
        assert_eq!(owner.base().admission().validated_files, 1);
        assert!(matches!(owner.projection(), Some(PendingFirstComptimeProjection::Candidate(
            RootedClosedComptimeProjection::Emitted(_)))));
        owner
    }

    let fixture = Scratch::new();
    let root = fixture.0.join("project");
    fs::create_dir(&root).unwrap();
    let entry = root.join("main.w");
    fs::write(root.join("wosy.toml"), MANIFEST).unwrap();
    fs::write(&entry, "%%start\ni32 answer = 14;\n%%end\n").unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    rt.block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&root, CLIENT).unwrap();
        let base = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(base.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", base.pending.unwrap()).unwrap());
        let prior = client.acknowledged_build_source("app").unwrap().unwrap();

        fs::write(&entry, COMPTIME_LIB).unwrap();
        let obsolete = client.stage("app").unwrap().pending.unwrap();
        let stale = prepare(&mut client, &worker, obsolete, TTL).await;
        assert!(client.abandon("app", obsolete).unwrap());
        let attempt = client.stage("app").unwrap().pending.unwrap();
        assert_eq!(attempt.root(), obsolete.root());
        assert_ne!(attempt.generation(), obsolete.generation());
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let refused = worker.publish_prepared_first_comptime_source(binding, proposal, stale, LIMITS).await.err().unwrap();
        assert!(refused.to_string().contains("does not match this pending attempt"));
        assert!(store.snapshot(attempt.root()).unwrap().is_none());

        let lifetime = Duration::from_secs(2);
        let expired = prepare(&mut client, &worker, attempt, lifetime).await;
        tokio::time::sleep(lifetime).await;
        assert!(expired.projection().is_none());
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let refused = worker.publish_prepared_first_comptime_source(binding, proposal, expired, LIMITS).await.err().unwrap();
        assert!(refused.to_string().contains("predecessor lease expired"));
        assert!(store.snapshot(attempt.root()).unwrap().is_none());

        let limited = prepare(&mut client, &worker, attempt, TTL).await;
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let strict = BaseCstAdmissionLimits { authored_bytes: 0, ..LIMITS };
        let refused = worker.publish_prepared_first_comptime_source(binding, proposal, limited, strict).await.err().unwrap();
        assert!(refused.to_string().contains("authored byte budget"));
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap(), prior);

        let exact = prepare(&mut client, &worker, attempt, TTL).await;
        let active_worker = &worker;
        let applied = client.publish_pending_base("app", attempt, |binding, proposal| async move {
            let committed = active_worker.publish_prepared_first_comptime_source(binding, proposal, exact, LIMITS).await?;
            assert_eq!(committed.authored_reparsed_files, 0);
            Ok(committed.source)
        }).await.unwrap();
        let PendingBasePublication::Applied(receipt) = applied else { panic!("expected exact source-only acknowledgement") };
        assert_eq!((receipt.binding.proposed, receipt.binding.attempt, receipt.validated_files),
            (attempt.root(), attempt.generation(), 1));
        assert!(store.snapshot(attempt.root()).unwrap().is_some());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, attempt.root());
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn client_shares_one_exact_proposal_and_respects_both_sqlite_freshness_cuts() {
    use std::{cell::{Cell, RefCell}, path::Path, sync::Weak};
    use wosy_compiler::ScalarTargetLayout;
    use wosy_daemon::{PendingFirstComptimeProjection, RootedClosedComptimeProjection, RootedClosedComptimeRequest};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::{BaseCstProposal, FileKey};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Advance { DuringPrepare, AfterCommit, Never }

    fn restage(project: &Path, obsolete: SourceAttempt) {
        let mut concurrent = SqliteSourceState::open(project, CLIENT).unwrap();
        assert!(concurrent.abandon("app", obsolete).unwrap());
        let replacement = concurrent.stage("app").unwrap().pending.unwrap();
        assert_eq!(replacement.root(), obsolete.root());
        assert_ne!(replacement.generation(), obsolete.generation());
    }

    async fn compose(
        local: &mut SqliteSourceState, worker: &SourceStorageWorker, project: &Path,
        attempt: SourceAttempt, advance: Advance,
    ) -> PendingBasePublication {
        let canonical = RefCell::<Option<Weak<BaseCstProposal>>>::new(None);
        let acknowledged = Cell::new(false);
        let observed_before = &canonical;
        let observed_after = &canonical;
        let committed = &acknowledged;
        let result = local.prepare_then_publish_pending_base("app", attempt,
            |binding, proposal| async move {
                assert!(Arc::strong_count(&proposal) >= 2);
                observed_before.replace(Some(Arc::downgrade(&proposal)));
                let request = RootedClosedComptimeRequest {
                    profile: "dev".into(),
                    file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
                    id: "fixture".into(), evaluator_layout: ScalarTargetLayout::NATIVE64,
                    source_limits: LIMITS, output_bytes: 1 << 20,
                };
                let prepared = worker.prepare_pending_first_comptime(binding, proposal, request, 1 << 20, TTL).await?;
                assert_eq!(prepared.base().admission().validated_files, 1);
                assert!(matches!(prepared.projection(), Some(PendingFirstComptimeProjection::Candidate(
                    RootedClosedComptimeProjection::Emitted(_)))));
                if advance == Advance::DuringPrepare { restage(project, attempt); }
                Ok(prepared)
            },
            |binding, proposal, prepared| async move {
                assert!(Arc::strong_count(&proposal) >= 2);
                assert!(Weak::ptr_eq(observed_after.borrow().as_ref().unwrap(), &Arc::downgrade(&proposal)));
                let receipt = worker.publish_prepared_first_comptime_source(binding, proposal, prepared, LIMITS).await?;
                assert_eq!(receipt.authored_reparsed_files, 0);
                committed.set(true);
                if advance == Advance::AfterCommit { restage(project, attempt); }
                Ok(receipt.source)
            },
        ).await.unwrap();
        assert_eq!(acknowledged.get(), advance != Advance::DuringPrepare);
        result
    }

    let fixture = Scratch::new();
    let project = fixture.0.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("wosy.toml"), MANIFEST).unwrap();
    let entry = project.join("main.w");
    fs::write(&entry, "%%start\ni32 answer = 14;\n%%end\n").unwrap();

    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut local = SqliteSourceState::open(&project, CLIENT).unwrap();
        let initial = local.stage("app").unwrap();
        let replay = local.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(initial.root, &replay.source.delta).await.unwrap();
        assert!(local.acknowledge("app", initial.pending.unwrap()).unwrap());

        fs::write(&entry, COMPTIME_LIB).unwrap();
        let obsolete = local.stage("app").unwrap().pending.unwrap();
        assert!(matches!(compose(&mut local, &worker, &project, obsolete, Advance::DuringPrepare).await,
            PendingBasePublication::StaleAttempt));
        assert!(store.snapshot(obsolete.root()).unwrap().is_none());
        assert_eq!(local.acknowledged_build_source("app").unwrap().unwrap().root, initial.root);

        let current = local.selection("app").unwrap().pending.unwrap();
        let matched = compose(&mut local, &worker, &project, current, Advance::Never).await;
        let PendingBasePublication::Applied(receipt) = matched else { panic!("expected exact acknowledgement"); };
        assert_eq!(receipt.binding.attempt, current.generation());
        assert_eq!(local.acknowledged_build_source("app").unwrap().unwrap().root, current.root());
        assert!(store.snapshot(current.root()).unwrap().is_some());

        fs::write(&entry, COMPTIME_LIB.replace("= 7", "= 8")).unwrap();
        let next = local.stage("app").unwrap().pending.unwrap();
        assert!(matches!(compose(&mut local, &worker, &project, next, Advance::AfterCommit).await,
            PendingBasePublication::StaleAttempt));
        assert!(store.snapshot(next.root()).unwrap().is_some());
        let still_pending = local.selection("app").unwrap();
        assert_eq!(still_pending.selected, Some(current.root()));
        assert_ne!(still_pending.pending.unwrap().generation(), next.generation());
        assert_eq!(still_pending.pending.unwrap().root(), next.root());
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}
