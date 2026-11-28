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

#[cfg(target_os = "linux")]
#[test]
fn pending_pair_call_graph_matches_ordinary_ordered_specializations_and_bypasses() {
    use wosy_cli::source_preparation::{prepare_pair_call_graph_comptime_input_facts, StartupComptimeInputSelection};
    use wosy_project::packages::ComptimeWatchLimits;
    use wosy_compiler::{DirectComptimeArgument, PairStartupOccurrence, ScalarTargetLayout};
    use wosy_daemon::{PendingPairCallGraphComptimeProjection, RootedClosedComptimeRequest,
        RootedPairCallGraphComptimeProjection};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::{FileKey, NodeId};

    const PREFIX: &str = r#"%%start
# WHY: startup com café ☕
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32(u32, u32) swap_data = fn(swap_left, swap_right) { solve(swap_right, swap_left) };
u32(bool, u32, u32) choose = fn(select_path, data_left, data_right) {
    if (select_path) { swap_data(data_left, data_right) } else { data_right }
};
u32 seed_value = choose(false, 1, 37);
u32 carried = choose(false, 1, seed_value);
"#;
    const CALLS: &str = "u32 reached = choose(true, 7, carried);\nu32 same = solve(carried, 7);\nu32 changed = solve(13, 7);\n";
    let source = format!("{PREFIX}{CALLS}%%end\n");
    let manifest = MANIFEST.replace("[targets.app.profiles.dev]", "[targets.app.profiles.dev]\ncomptime_builder = \"nominal\"\ncomptime_runner = \"nominal\"\nbuild_permissions = []\nbuild_mounts = []");
    let fixture = Scratch::new();
    let project = fixture.0.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("wosy.toml"), manifest).unwrap();
    let entry = project.join("main.w");
    fs::write(&entry, "%%start\nu32 answer = 14;\n%%end\n").unwrap();

    let request = |layout, output_bytes| RootedClosedComptimeRequest {
        profile: "dev".into(),
        file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
        id: "prepare".into(), evaluator_layout: layout, source_limits: LIMITS, output_bytes,
    };
    let ordinary = |layout| prepare_pair_call_graph_comptime_input_facts(&project, StartupComptimeInputSelection {
        target: "app", profile: "dev", id: "prepare", evaluator_layout: layout,
        watch_limits: ComptimeWatchLimits::default(), max_ir_bytes: 1 << 20,
    }).unwrap();

    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&project, CLIENT).unwrap();
        let initial = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(initial.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", initial.pending.unwrap()).unwrap());

        fs::write(&entry, &source).unwrap();
        let attempt = client.stage("app").unwrap().pending.unwrap();
        let layouts = [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32];
        let observed = layouts.map(ordinary);
        // Valid live content changes after preparation of the preceding
        // immutable pending generation. The daemon must not reread it.
        fs::write(&entry, source.replace("1, 37", "1, 41")).unwrap();

        let mut native_bytes = 0;
        for (layout, (ordinary_source, expected)) in layouts.into_iter().zip(observed) {
            let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
            let prepared = worker.prepare_pending_pair_call_graph_comptime(binding, proposal,
                request(layout, 1 << 20), 1 << 20, TTL).await.unwrap();
            let admission = prepared.base().admission();
            assert_eq!((admission.base, admission.proposed, admission.attempt, admission.validated_files),
                (initial.root, attempt.root(), attempt.generation(), 1));
            let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan))) =
                prepared.projection() else { panic!("expected an exact pending typed startup graph"); };
            assert_eq!(ordinary_source.root, attempt.root());
            assert_eq!(plan.source_root(), attempt.root());
            assert_eq!(plan.authored_cst(), expected.authored_cst());
            assert_eq!((plan.target(), plan.profile(), plan.id()), ("app", "dev", "prepare"));
            assert_eq!(plan.evidence().site(), expected.evidence().site());
            assert_eq!(plan.evidence().layout(), layout);
            let actual = plan.evidence().occurrences().collect::<Vec<_>>();
            let oracle = expected.evidence().occurrences().collect::<Vec<_>>();
            assert_eq!((actual.len(), oracle.len()), (5, 5));
            for (actual, oracle) in actual.iter().zip(&oracle) {
                match (actual, oracle) {
                    (PairStartupOccurrence::Bypass(actual), PairStartupOccurrence::Bypass(oracle)) => {
                        assert_eq!(actual.call(), oracle.call());
                        assert_eq!(actual.returned_argument(), oracle.returned_argument());
                        assert_eq!(actual.returned_value(), DirectComptimeArgument::U32(37));
                        assert_eq!(actual.returned_origin().prior_bypass_slot(), oracle.returned_origin().prior_bypass_slot());
                    }
                    (PairStartupOccurrence::Specialization(actual, path), PairStartupOccurrence::Specialization(oracle, oracle_path)) => {
                        assert_eq!(actual.facts().destination(), oracle.facts().destination());
                        assert_eq!(actual.facts().inputs().each_ref().map(|input| input.value()),
                            oracle.facts().inputs().each_ref().map(|input| input.value()));
                        assert_eq!(actual.evaluator_index(), oracle.evaluator_index());
                        assert_eq!(path.hop_indices(), oracle_path.hop_indices());
                    }
                    _ => panic!("pending and ordinary startup disagree on reachability"),
                }
            }
            assert_eq!(plan.evidence().calls().map(|(use_, _)| use_.evaluator_index()).collect::<Vec<_>>(), [0, 0, 1]);
            assert_eq!((plan.evaluators().len(), expected.evaluator_irs().len()), (2, 2));
            for (actual, oracle) in plan.evaluators().iter().zip(expected.evaluator_irs()) {
                assert_eq!((actual.bytes().len(), actual.digest()), (oracle.bytes, NodeId(*oracle.blake3.as_bytes())));
            }
            if layout == ScalarTargetLayout::NATIVE64 {
                native_bytes = plan.evaluators().iter().map(|ir| ir.bytes().len()).sum();
            }
            assert!(store.snapshot(attempt.root()).unwrap().is_none());
        }

        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let denied = worker.prepare_pending_pair_call_graph_comptime(binding, proposal,
            request(ScalarTargetLayout::NATIVE64, native_bytes - 1), 1 << 20, TTL).await.err().unwrap();
        assert!(denied.to_string().contains("output byte budget"));
        assert!(client.abandon("app", attempt).unwrap());
        assert!(store.snapshot(attempt.root()).unwrap().is_none());

        fs::write(&entry, format!("{PREFIX}%%end\n")).unwrap();
        let bypass = client.stage("app").unwrap().pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", bypass).unwrap();
        let prepared = worker.prepare_pending_pair_call_graph_comptime(binding, proposal,
            request(ScalarTargetLayout::WASM32, 0), 1 << 20, TTL).await.unwrap();
        let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan))) =
            prepared.projection() else { panic!("known bypasses require no evaluator budget"); };
        assert_eq!(prepared.base().authored_comptime_coverage(), Some(AuthoredComptimeCoverage::Present));
        assert_eq!((plan.evidence().occurrences().len(), plan.evidence().calls().len(), plan.evaluators().len()), (2, 0, 0));
        assert!(plan.evidence().occurrences().all(|slot| matches!(slot, PairStartupOccurrence::Bypass(_))));
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, initial.root);
        assert!(store.snapshot(bypass.root()).unwrap().is_none());
        drop(prepared);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn trusted_pending_graph_observes_only_exact_consumer_config_and_live_watch_inputs() {
    use std::path::Path;
    use wosy_cli::source_state::TrustedPreparedPairComptime;
    use wosy_compiler::ScalarTargetLayout;
    use wosy_daemon::{PendingPairCallGraphComptimeProjection, PreparedPendingPairCallGraphComptime,
        RootedClosedComptimeRequest, RootedPairCallGraphComptimeProjection};
    use wosy_project::{packages::{ComptimeWatchLimits, WatchRoot}, DeclaredComptimeParameter, PackageRelativePath};
    use wosy_protocol::FileKey;

    fn trusted(owner: &PreparedPendingPairCallGraphComptime) -> Option<TrustedPreparedPairComptime<'_>> {
        let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan))) =
            owner.projection() else { return None; };
        Some(TrustedPreparedPairComptime {
            binding: owner.binding(), authored_cst: plan.authored_cst(), profile: plan.profile(), graph: plan.evidence(),
        })
    }

    const SOURCE: &str = "%%start\n# WHY: perfil válido\nu32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };\nu32 sample = solve(4, 9);\n%%end\n";
    let manifest = MANIFEST.replace("[targets.app.profiles.dev]", "[targets.app.profiles.dev]\ncomptime_builder = \"nominal\"\ncomptime_runner = \"nominal\"\nbuild_permissions = []\nbuild_mounts = []")
        + "\n[targets.app.profiles.dev.params]\nlimit = 77\n[comptime.prepare]\nwatch = [\"watched/value.txt\"]\n";
    let fixture = Scratch::new();
    let project = fixture.0.join("project");
    let wm = project.join("wosy.toml");
    let entry = project.join("main.w");
    let watched = project.join("watched/value.txt");
    fs::create_dir_all(project.join("watched")).unwrap();
    fs::write(&wm, &manifest).unwrap();
    fs::write(&entry, "%%start\nu32 answer = 14;\n%%end\n").unwrap();
    fs::write(&watched, "first-watch").unwrap();

    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&project, CLIENT).unwrap();
        let initial = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(initial.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", initial.pending.unwrap()).unwrap());

        fs::write(&entry, SOURCE).unwrap();
        let attempt = client.stage("app").unwrap().pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let owner = worker.prepare_pending_pair_call_graph_comptime(binding.clone(), proposal, RootedClosedComptimeRequest {
            profile: "dev".into(), file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
            id: "prepare".into(), evaluator_layout: ScalarTargetLayout::WASM32, source_limits: LIMITS, output_bytes: 1 << 20,
        }, 1 << 20, TTL).await.unwrap();
        let plan = trusted(&owner).expect("real compiler-derived pending plan");
        assert_eq!(plan.graph.site_id(), "prepare");
        // Valid live source no longer contains the site. It must not
        // supply or disturb this immutable pending graph's input evidence.
        fs::write(&entry, "%%start\nu32 answer = 41;\n%%end\n").unwrap();
        let limits = ComptimeWatchLimits::default();
        let first = client.observe_pending_pair_comptime_inputs(&binding, "dev", limits, || trusted(&owner)).unwrap().unwrap();
        assert_eq!(first.binding(), &binding);
        assert_eq!(first.authored_cst(), plan.authored_cst);
        assert_eq!(first.watches().site, *plan.graph.site());
        assert_eq!(first.watches().id, "prepare");
        let tools = first.declarations().tools();
        assert_eq!(tools.builder().0, "nominal");
        assert_eq!(tools.runner().0, "nominal");
        assert_eq!(first.declarations().parameters().get("limit"), Some(&DeclaredComptimeParameter::Integer(77)));
        let [observed] = first.watches().files.as_slice() else { panic!("only the configured owning watch"); };
        assert_eq!(observed.root, WatchRoot::OwningPackage);
        assert_eq!(observed.path.as_path(), Path::new("watched/value.txt"));
        assert_eq!(observed.revision.0, blake3::hash(b"first-watch").to_hex().as_str());

        fs::write(project.join("watched/unselected.txt"), "not observed").unwrap();
        let unchanged = client.observe_pending_pair_comptime_inputs(&binding, "dev", limits, || trusted(&owner)).unwrap().unwrap();
        assert_eq!(unchanged.watches().files, first.watches().files);
        fs::write(&watched, "updated-watch").unwrap();
        let changed = client.observe_pending_pair_comptime_inputs(&binding, "dev", limits, || trusted(&owner)).unwrap().unwrap();
        assert_eq!(changed.configuration_inputs(), first.configuration_inputs());
        assert_eq!(changed.authored_cst(), first.authored_cst());
        assert_eq!(changed.watches().files[0].revision.0, blake3::hash(b"updated-watch").to_hex().as_str());

        fs::write(&wm, manifest.replace("watched/value.txt", "watched/not-created.txt")).unwrap();
        let changed_manifest = client.observe_pending_pair_comptime_inputs(&binding, "dev", limits, || trusted(&owner)).err().unwrap();
        assert_eq!(changed_manifest.to_string(), "local comptime configuration changed after source staging");
        fs::write(&wm, &manifest).unwrap();
        fs::write(&entry, SOURCE).unwrap();
        let stale = client.observe_pending_pair_comptime_inputs(&binding, "dev", limits, || {
            let mut current = SqliteSourceState::open(&project, CLIENT).unwrap();
            assert!(current.abandon("app", attempt).unwrap());
            let replacement = current.stage("app").unwrap().pending.unwrap();
            assert_eq!(replacement.root(), attempt.root());
            assert_ne!(replacement.generation(), attempt.generation());
            trusted(&owner)
        }).unwrap();
        assert!(stale.is_none());
        assert!(client.observe_pending_pair_comptime_inputs(&binding, "dev", limits,
            || panic!("do not call the local owner for an already stale attempt")).unwrap().is_none());
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, initial.root);
        drop(owner);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn pending_pair_graph_emits_incumbent_wasip1_stdout_for_distinct_scalar_tuples_only() {
    use wosy_compiler::{ClosedComptimeRejection, ScalarTargetLayout};
    use wosy_daemon::{PendingPairCallGraphComptimeProjection, RootedClosedComptimeRefusal, RootedClosedComptimeRequest,
        RootedPairCallGraphComptimeProjection};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::FileKey;

    fn body(ir: &str, symbol: &str) -> String {
        let prefix = format!("@{symbol}(");
        let mut lines = ir.lines();
        assert!(lines.any(|line| line.starts_with("define ") && line.contains(&prefix)), "{symbol} definition");
        lines.take_while(|line| *line != "}").collect::<Vec<_>>().join("\n")
    }

    const SOURCE: &str = r#"%%start
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32(bool, u32, u32) choose = fn(select_path, left, right) {
    if (select_path) { solve(left, right) } else { right }
};
u32 skipped = choose(false, 7, 37);
u32 reached = choose(true, 7, skipped);
u32 same = solve(7, skipped);
u32 changed = solve(7, 13);
%%end
"#;
    let manifest = MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"") + r#"
[targets.app.artifacts.main.profiles.native]
backend = "llvm"
output = "native"
builder = "nominal"
run_runner = "nominal"
[targets.app.profiles.native]
main_artifact = "main"
"#;
    let fixture = Scratch::new();
    let project = fixture.0.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("wosy.toml"), manifest).unwrap();
    let entry = project.join("main.w");
    fs::write(&entry, "%%start\nu32 answer = 14;\n%%end\n").unwrap();
    let request = |profile: &str, layout| RootedClosedComptimeRequest {
        profile: profile.into(),
        file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
        id: "prepare".into(), evaluator_layout: layout, source_limits: LIMITS, output_bytes: 1 << 20,
    };
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&project, CLIENT).unwrap();
        let initial = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(initial.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", initial.pending.unwrap()).unwrap());

        fs::write(&entry, SOURCE).unwrap();
        let attempt = client.stage("app").unwrap().pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let detached = worker.prepare_pending_pair_call_graph_comptime(binding, proposal,
            request("dev", ScalarTargetLayout::WASM32), 1 << 20, TTL).await.unwrap();
        let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan))) =
            detached.projection() else { panic!("existing detached route must remain supported"); };
        assert_eq!(plan.max_wasip1_stdout(), None);
        let detached_body = plan.evaluators().iter().map(|evaluator| {
            let ir = std::str::from_utf8(evaluator.bytes()).unwrap();
            assert!(!ir.contains("wasi_snapshot_preview1"));
            body(ir, "evaluate")
        }).collect::<Vec<_>>();
        drop(detached);

        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let stdout = worker.prepare_pending_pair_call_graph_wasip1_stdout(binding, proposal,
            request("dev", ScalarTargetLayout::WASM32), 1 << 20, TTL, 11).await.unwrap();
        let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan))) =
            stdout.projection() else { panic!("typed P1 stdout evaluator was not emitted"); };
        assert_eq!(stdout.base().admission().validated_files, 1);
        assert_eq!(plan.max_wasip1_stdout(), Some(11));
        assert_eq!(plan.source_root(), attempt.root());
        assert_eq!((plan.evidence().occurrences().len(), plan.evaluators().len()), (4, 2));
        assert_eq!(plan.evidence().calls().map(|(call, _)| call.evaluator_index()).collect::<Vec<_>>(), [0, 0, 1]);
        assert_eq!(detached_body.len(), 2);
        for (index, evaluator) in plan.evaluators().iter().enumerate() {
            let ir = std::str::from_utf8(evaluator.bytes()).unwrap();
            assert_eq!(body(ir, "evaluate"), detached_body[index]);
            assert!(ir.contains("\"wasm-import-module\"=\"wasi_snapshot_preview1\""));
            assert!(ir.contains("\"wasm-import-name\"=\"fd_write\""));
            let main = body(ir, "main");
            assert!(main.contains("call i32 @evaluate()"));
            assert!(main.contains("@fd_write(i32 1,"));
            assert!(main.contains("store i8 10"));
        }
        drop(stdout);

        for (profile, layout, limit, expected) in [
            ("native", ScalarTargetLayout::WASM32, 11, 0),
            ("dev", ScalarTargetLayout::NATIVE64, 11, 1),
            ("dev", ScalarTargetLayout::WASM32, 10, 2),
        ] {
            let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
            let denied = worker.prepare_pending_pair_call_graph_wasip1_stdout(binding, proposal,
                request(profile, layout), 1 << 20, TTL, limit).await.unwrap();
            let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Refused(reason))) =
                denied.projection() else { panic!("unsupported output scope must not emit"); };
            match (expected, reason) {
                (0, RootedClosedComptimeRefusal::ProfileNotWasip1)
                | (1, RootedClosedComptimeRefusal::Compiler(ClosedComptimeRejection::UnsupportedLayout)) => {}
                (2, RootedClosedComptimeRefusal::Compiler(ClosedComptimeRejection::Emitter(error))) =>
                    assert!(error.contains("stdout shape exceeds the selected byte budget")),
                _ => panic!("unexpected stdout refusal: {reason:?}"),
            }
        }
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, initial.root);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn stage_exact_pending_wasip1_evaluators_once_per_tuple_without_launch() {
    exercise_pending_pair(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires ordinary clang, wasm-ld, python3 and wasmtime on PATH"]
fn executes_selected_ordinary_comptime_tools_for_each_distinct_evaluator() {
    exercise_pending_pair(true);
}

#[cfg(target_os = "linux")]
fn exercise_pending_pair(run_tools: bool) {
    use std::{os::unix::{fs::DirBuilderExt, process::ExitStatusExt}, path::Path, process::ExitStatus};
    use wosy_cli::{comptime_fragment::{ComptimeReadbackError, PendingPairWasip1Occurrence, PendingPairWasip1ReadbackError},
        source_state::TrustedPreparedPairWasip1};
    use wosy_compiler::{ComptimeStdoutError, DirectComptimeArgument, PairStartupOccurrence, ScalarExpression,
        ScalarTargetLayout, ScalarType};
    use wosy_syntax::{byte_span, SyntaxKind};
    use wosy_daemon::{PendingPairCallGraphComptimeProjection, PreparedPendingPairCallGraphComptime,
        RootedClosedComptimeRequest, RootedPairCallGraphComptimeProjection};
    use wosy_project::{packages::ComptimeWatchLimits, PackageRelativePath};
    use wosy_protocol::FileKey;

    fn trusted(owner: &PreparedPendingPairCallGraphComptime) -> Option<TrustedPreparedPairWasip1<'_>> {
        let Some(PendingPairCallGraphComptimeProjection::Candidate(RootedPairCallGraphComptimeProjection::Emitted(plan))) =
            owner.projection() else { return None; };
        Some(TrustedPreparedPairWasip1 {
            binding: owner.binding(), authored_cst: plan.authored_cst(), profile: plan.profile(), output: plan.wasip1_output()?,
            execution_seal: owner.execution_seal(),
        })
    }

    const SOURCE: &str = r#"%%start
# WHY: órbita com café ☕
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32(bool, u32, u32) choose = fn(select_path, left, right) {
    if (select_path) { solve(left, right) } else { right }
};
u32 bypass = choose(false, 7, 37);
u32 first = choose(true, 7, bypass);
u32 repeated = solve(7, bypass);
u32 different = solve(7, 13);
%%end
"#;
    let source = SOURCE.replace("%%end", &format!("# WHY: {}\n%%end", "a".repeat(1 << 16)));
    let mut manifest = MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")
        .replace("[targets.app.profiles.dev]", "[targets.app.profiles.dev]\ncomptime_builder = \"nominal\"\ncomptime_runner = \"nominal\"\nbuild_permissions = []\nbuild_mounts = []")
        + "\n[comptime.prepare]\nwatch = [\"watch.txt\"]\n";
    let fixture = Scratch::new();
    let project = fixture.0.join("project");
    let private = fixture.0.join("private");
    fs::create_dir(&project).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    if run_tools {
        manifest = manifest.replace(r#""fixture-must-not-build""#, r#""python3", "ordinary_wasip1_pair.py", "build""#)
            .replace(r#""fixture-must-not-run""#, r#""python3", "ordinary_wasip1_pair.py", "run""#);
        fs::write(project.join("ordinary_wasip1_pair.py"), include_str!("fixtures/ordinary_wasip1_pair.py")).unwrap();
    }
    fs::write(project.join("wosy.toml"), manifest).unwrap();
    fs::write(project.join("watch.txt"), b"watched input").unwrap();
    let entry = project.join("main.w");
    fs::write(&entry, "%%start\nu32 answer = 14;\n%%end\n").unwrap();

    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&project, CLIENT).unwrap();
        let initial = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(initial.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", initial.pending.unwrap()).unwrap());

        fs::write(&entry, &source).unwrap();
        let attempt = client.stage("app").unwrap().pending.unwrap();
        let (binding, proposal) = client.pending_base_cst_proposal("app", attempt).unwrap();
        let request = |authored_bytes| RootedClosedComptimeRequest {
            profile: "dev".into(), file: FileKey { package: "app".into(), path: PackageRelativePath::new("main.w".into()).unwrap() },
            id: "prepare".into(), evaluator_layout: ScalarTargetLayout::WASM32,
            source_limits: BaseCstAdmissionLimits { authored_bytes, ..LIMITS }, output_bytes: 1 << 20,
        };
        let proposal = Arc::new(proposal);
        let denied = worker.prepare_pending_pair_call_graph_wasip1_stdout(binding.clone(), Arc::clone(&proposal),
            request(source.len() - 1), 1 << 20, TTL, 11).await.err().expect("one byte over explicit admission budget");
        assert!(denied.to_string().contains("authored byte budget"));
        let owner = worker.prepare_pending_pair_call_graph_wasip1_stdout(binding.clone(), Arc::clone(&proposal),
            request(source.len()), 1 << 20, TTL, 11).await.unwrap();
        assert_eq!(owner.base().admission().validated_files, 1);
        let held = trusted(&owner).expect("same private compiler-issued output");
        let [first, distinct] = held.output.evaluators() else { panic!("one module per typed tuple"); };
        assert_eq!(held.output.evidence().calls().map(|(call, _)| call.evaluator_index()).collect::<Vec<_>>(), [0, 0, 1]);
        assert_ne!(first.bytes(), distinct.bytes());
        let shape = owner.result_shape().expect("complete owned distinct evaluator set");
        assert_eq!(shape.binding(), &binding);
        assert_eq!(shape.prepared(), owner.base().identity());
        assert_eq!(shape.authored_cst(), held.authored_cst);
        assert_eq!(shape.profile(), "dev");
        assert_eq!(shape.evaluators().len(), 2);
        for (planned, emitted) in shape.evaluators().iter().zip(held.output.evaluators()) {
            assert_eq!(planned.fragment_ir.0, *blake3::hash(emitted.bytes()).as_bytes());
        }
        let total = first.bytes().len() + distinct.bytes().len();

        // The staging cut uses the captured CST, not re-authored input.
        fs::write(&entry, "%%start\nu32 answer = 41;\n%%end\n").unwrap();
        let limits = ComptimeWatchLimits::default();
        let staging = client.stage_pending_pair_wasip1(&binding, "dev", limits, &private, total,
            || trusted(&owner)).unwrap().unwrap();
        assert!(std::ptr::eq(staging.plan(), held.output));
        assert_eq!(staging.plan().max_stdout(), 11);
        assert_eq!(staging.observation().binding(), &binding);
        assert_eq!(staging.observation().watches().files[0].revision.0, blake3::hash(b"watched input").to_hex().as_str());

        // The green tree came through the daemon's blocking worker from
        // the once-validated proposal. Open only a new view, after the
        // live file lost this graph entirely. UTF-8 ranges remain authored.
        let ordinary = staging.plan().ordinary_continuation();
        let exact_site = staging.plan().evidence().site();
        assert_eq!(ordinary.canonical_cst().source, exact_site.source);
        assert_eq!(ordinary.canonical_cst().root.text().to_string(), source);
        assert_eq!(staging.plan().authored_source_bytes(), source.len());
        let site_node = ordinary.comptime_site();
        let tail = ordinary.callable_tail();
        assert_eq!(byte_span(&site_node), exact_site.range);
        assert_eq!(site_node.text().to_string(), "comptime prepare { right ^ left }");
        assert_eq!(tail.kind(), SyntaxKind::Expression);
        assert!(site_node.ancestors().any(|node| node == tail));
        let mut selections = Vec::new();
        let initializers = [
            "choose(false, 7, 37)", "choose(true, 7, bypass)", "solve(7, bypass)", "solve(7, 13)",
        ];
        assert_eq!(ordinary.initializers().len(), initializers.len());
        for ((expression, case), spelling) in ordinary.initializers().zip(initializers) {
            assert_eq!(expression.kind(), SyntaxKind::Expression);
            assert_eq!(expression.text().to_string(), spelling);
            let (call, evaluator_index) = match case {
                PairStartupOccurrence::Bypass(bypass) => (bypass.call(), None),
                PairStartupOccurrence::Specialization(call, _) => (call.facts().call(), Some(call.evaluator_index())),
            };
            let span = byte_span(&expression);
            assert!(span.start <= call.range.start && call.range.end <= span.end);
            assert_eq!(&source[span.start as usize..span.end as usize], spelling);
            selections.push(evaluator_index);
        }
        assert_eq!(selections, [None, Some(0), Some(0), Some(1)]);
        drop(ordinary);
        assert_eq!(fs::read_dir(&private).unwrap().count(), 2);
        for (stage, exact) in staging.evaluators().zip([first, distinct]) {
            assert_eq!(stage.result_type(), &ScalarType::U32);
            let build = stage.builder();
            let run = stage.runner();
            assert_eq!(build.working_directory, project);
            assert_eq!(run.working_directory, project);
            let (builder_paths, runner_paths) = if run_tools {
                assert_eq!(build.executable, "python3");
                assert_eq!(run.executable, "python3");
                assert_eq!(build.arguments[0], "ordinary_wasip1_pair.py");
                assert_eq!(build.arguments[1], "build");
                assert_eq!(run.arguments[0], "ordinary_wasip1_pair.py");
                assert_eq!(run.arguments[1], "run");
                (&build.arguments[2..], &run.arguments[2..])
            } else {
                assert_eq!(build.executable, "fixture-must-not-build");
                assert_eq!(run.executable, "fixture-must-not-run");
                (build.arguments.as_slice(), run.arguments.as_slice())
            };
            let [ir, output, manifest] = builder_paths else { panic!("three builder paths"); };
            assert_eq!(runner_paths, &[*manifest]);
            assert_eq!(Path::new(output), stage.expected_module_path().parent().unwrap());
            let actual = fs::read(ir).unwrap();
            assert_eq!(actual, exact.bytes());
            assert_eq!(stage.evaluator_ir().blake3, blake3::hash(&actual));
            let document: serde_json::Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
            assert_eq!(document, serde_json::json!({
                "executable": stage.expected_module_path(), "parameters": {}, "permissions": [], "mounts": []
            }));
            assert!(!stage.expected_module_path().exists());
        }
        // Known scalar bytes and statuses are host fixtures, not a claim
        // these evaluators ran. One parsed value per distinct tuple.
        let ok = ExitStatus::from_raw(0);
        let read = staging.read_stdout(&[(ok, b"34\n"), (ok, b"10\n")]).unwrap();
        let ordered = read.occurrences().collect::<Vec<_>>();
        let [
            PendingPairWasip1Occurrence::Bypass(bypass),
            PendingPairWasip1Occurrence::Specialization { call: first_call, value: first_value, .. },
            PendingPairWasip1Occurrence::Specialization { call: repeated_call, value: repeated_value, .. },
            PendingPairWasip1Occurrence::Specialization { call: other_call, value: other_value, .. },
        ] = ordered.as_slice() else { panic!("ordinary bypass and all three authored reached slots"); };
        assert_eq!(bypass.returned_value(), DirectComptimeArgument::U32(37));
        assert_eq!([first_call.evaluator_index(), repeated_call.evaluator_index(), other_call.evaluator_index()], [0, 0, 1]);
        assert!(std::ptr::eq(*first_value, *repeated_value), "a repeat borrows the same parsed value");
        assert!(!std::ptr::eq(*first_value, *other_value));
        let site = held.output.evidence().site();
        assert!(matches!(first_value, ScalarExpression::Integer { value, span }
            if u32::try_from(value) == Ok(34) && *span == site.range));
        assert!(matches!(other_value, ScalarExpression::Integer { value, span }
            if u32::try_from(value) == Ok(10) && *span == site.range));

        let terminals: [(ExitStatus, &[u8]); 2] = [(ok, b"34\n"), (ok, b"10\n")];
        let accepted = client.observe_staged_pending_pair_wasip1(&staging, limits, &terminals).unwrap().unwrap();
        assert_eq!(accepted.observation().binding(), &binding);
        use wosy_protocol::ComptimeScalarValue::Unsigned;
        assert_eq!(accepted.scalar_results(), Some(vec![
            Unsigned { width: 32, value: 34 }, Unsigned { width: 32, value: 10 },
        ]));
        let exact = held.output.ordinary_continuation();
        let projected = accepted.initializers().collect::<Vec<_>>();
        assert_eq!(projected.len(), initializers.len());
        for (((node, observed), (original, case)), spelling) in projected.iter().zip(exact.initializers()).zip(initializers) {
            assert_eq!(byte_span(node), byte_span(&original));
            assert_eq!(node.text().to_string(), spelling);
            assert_eq!(node.ancestors().last().unwrap().text().to_string(), source);
            match (observed, case) {
                (PendingPairWasip1Occurrence::Bypass(seen), PairStartupOccurrence::Bypass(authored)) => {
                    assert!(std::ptr::eq(*seen, authored));
                }
                (PendingPairWasip1Occurrence::Specialization { call, path, .. }, PairStartupOccurrence::Specialization(authored, route)) => {
                    assert!(std::ptr::eq(*call, authored));
                    assert!(std::ptr::eq(*path, route));
                }
                _ => panic!("initializer and result must keep the original case"),
            }
        }
        let [(_, PendingPairWasip1Occurrence::Bypass(_)),
            (_, PendingPairWasip1Occurrence::Specialization { value: one, .. }),
            (_, PendingPairWasip1Occurrence::Specialization { value: again, .. }),
            (_, PendingPairWasip1Occurrence::Specialization { value: other, .. })] = projected.as_slice()
            else { panic!("same exact three reached slots") };
        assert!(std::ptr::eq(*one, *again));
        assert!(!std::ptr::eq(*one, *other));

        if run_tools {
            fs::write(&entry, &source).unwrap();
            let executed = client.execute_staged_pending_pair_wasip1(
                &staging, limits, 1 << 20, Duration::from_secs(10), || false,
            ).unwrap().expect("same exact pending attempt");
            assert_eq!(executed.readback().observation().binding(), &binding);
            assert_eq!(executed.readback().scalar_results(), accepted.scalar_results());
            assert_eq!(executed.modules().len(), 2);
            assert_ne!(executed.modules()[0], executed.modules()[1]);
            let admitted = owner.prepare_owned_result_intake(executed.execution())
                .unwrap().expect("the exact live source owns both observed results");
            assert_eq!(admitted.prepared(), owner.base().identity());
            assert_eq!(admitted.binding(), &binding);
            assert_eq!(admitted.modules().iter().map(|id| id.0).collect::<Vec<_>>(),
                executed.modules().iter().map(|digest| *digest.as_bytes()).collect::<Vec<_>>());
            assert_eq!(admitted.values().len(), 2);
            assert!(matches!(&admitted.values()[0], ScalarExpression::Integer { value, .. } if u32::try_from(value) == Ok(34)));
            assert!(matches!(&admitted.values()[1], ScalarExpression::Integer { value, .. } if u32::try_from(value) == Ok(10)));
            let same = owner.prepare_owned_result_intake(executed.execution()).unwrap().unwrap();
            assert_eq!(admitted.root(), same.root());
            let unrelated_owner = worker.prepare_pending_pair_call_graph_wasip1_stdout(
                binding.clone(), Arc::clone(&proposal), request(source.len()), 1 << 20, TTL, 11,
            ).await.unwrap();
            assert_eq!(unrelated_owner.base().identity(), owner.base().identity());
            assert!(unrelated_owner.prepare_owned_result_intake(executed.execution())
                .err().unwrap().to_string().contains("different prepared attempt"));

            assert!(store.snapshot(attempt.root()).unwrap().is_none(), "intake does not commit base source");
            assert!(unrelated_owner.into_semantic_snapshot(same)
                .err().unwrap().to_string().contains("does not belong"));
            let result_root = admitted.root();
            drop(executed);
            drop(staging);
            let snapshot = owner.into_semantic_snapshot(admitted).unwrap().unwrap();
            assert_eq!(snapshot.source(), &binding);
            assert_eq!(snapshot.result_root(), result_root);
            let (ordinary, applied_inputs) = snapshot.ordinary_inputs().expect("live attached compiler owner");
            assert_eq!(ordinary.ordinary_continuation().initializers().count(), 4);
            assert_eq!(applied_inputs.len(), 2);
            assert!(snapshot.lease_is_live());
            let deadline = Duration::from_secs(10);
            let watch = project.join("watch.txt");
            fs::write(&watch, b"stale watched input").unwrap();
            assert!(client.emit_live_pair_ordinary_startup(&snapshot, limits, deadline).unwrap().is_none());
            fs::write(&watch, b"watched input").unwrap();
            fs::write(&entry, "%%start\nu32 changed = 0;\n%%end\n").unwrap();
            assert!(client.emit_live_pair_ordinary_startup(&snapshot, limits, deadline).unwrap().is_none());
            fs::write(&entry, &source).unwrap();
            let partition = client.emit_live_pair_ordinary_startup(&snapshot, limits, deadline)
                .unwrap().expect("ordinary typed result continuation for current source");
            let mut emitted = Vec::new();
            partition.write_text(&mut emitted).unwrap();
            let emitted = String::from_utf8(emitted).unwrap();
            assert!(emitted.contains("i32 37"), "{emitted}");
            assert_eq!(emitted.matches("i32 34").count(), 2, "{emitted}");
            assert!(emitted.contains("i32 10"), "{emitted}");
            assert!(!emitted.contains("call i32 @solve") && !emitted.contains("call i32 @choose"), "{emitted}");
            // The ordinary artifact itself, independent of the evaluator
            // runs, must initialize the real globals in Wasmtime.
            let ir = fixture.0.join("ordinary-artifact.ll");
            let runtime = fixture.0.join("ordinary-runtime.ll");
            let wasm = fixture.0.join("ordinary-artifact.wasm");
            fs::write(&ir, &emitted).unwrap();
            fs::write(&runtime, wosy_compiler::core_runtime("llvm", "wasm-wasip1").unwrap()).unwrap();
            for (expected, exit) in [("10", 0), ("11", 73)] {
                let status = std::process::Command::new("python3").current_dir(&project)
                    .args(["ordinary_wasip1_pair.py", "artifact-check"])
                    .args([&ir, &runtime, &wasm]).arg(expected).status().unwrap();
                assert_eq!(status.code(), Some(exit));
            }


            drop(snapshot);
            drop(worker);
            Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
            return;


        }

        let watch = project.join("watch.txt");
        fs::write(&watch, b"changed input").unwrap();
        assert!(client.observe_staged_pending_pair_wasip1(&staging, limits, &terminals)
            .err().unwrap().to_string().contains("inputs changed"));
        fs::write(&watch, b"watched input").unwrap();
        let manifest_file = project.join("wosy.toml");
        let original_manifest = fs::read(&manifest_file).unwrap();
        let changed_manifest = String::from_utf8(original_manifest.clone()).unwrap()
            .replace(if run_tools { "ordinary_wasip1_pair.py" } else { "fixture-must-not-run" }, "other-configured-runner");
        fs::write(&manifest_file, changed_manifest).unwrap();
        assert!(client.observe_staged_pending_pair_wasip1(&staging, limits, &terminals)
            .err().unwrap().to_string().contains("configuration changed"));
        fs::write(&manifest_file, original_manifest).unwrap();
        assert!(client.observe_staged_pending_pair_wasip1(&staging, limits, &terminals[..1]).is_err());
        assert!(client.observe_staged_pending_pair_wasip1(&staging, limits,
            &[(ok, b"34\n"), (ExitStatus::from_raw(7 << 8), b"not parsed")]).is_err());

        type ReadError = PendingPairWasip1ReadbackError;
        assert!(matches!(staging.read_stdout(&[(ok, b"34\n")]), Err(ReadError::TerminalCount { expected: 2, supplied: 1 })));
        assert!(matches!(staging.read_stdout(&[(ok, b"34\n"), (ok, b"10\n"), (ok, b"99\n")]),
            Err(ReadError::TerminalCount { expected: 2, supplied: 3 })));
        assert!(matches!(staging.read_stdout(&[(ok, b"34\n"), (ExitStatus::from_raw(7 << 8), b"not parsed")]),
            Err(ReadError::Evaluator { index: 1, error: ComptimeReadbackError::Exit(Some(7)) })));
        for (bytes, expected) in [
            (b"true\n".as_slice(), ComptimeStdoutError::NonCanonical),
            (b"4294967296\n".as_slice(), ComptimeStdoutError::OutOfRange),
            (b"00000000000\n".as_slice(), ComptimeStdoutError::ByteLimit),
        ] {
            assert!(matches!(staging.read_stdout(&[(ok, b"34\n"), (ok, bytes)]),
                Err(ReadError::Evaluator { index: 1, error: ComptimeReadbackError::Output(actual) }) if actual == expected));
        }
        assert_eq!(fs::read_dir(&private).unwrap().count(), 2);

        // A budget that admits the first slot but not both writes neither.
        let denied = client.stage_pending_pair_wasip1(&binding, "dev", limits, &private, first.bytes().len(),
            || trusted(&owner)).err().unwrap();
        assert!(denied.to_string().contains("exceeds byte budget"));
        assert_eq!(fs::read_dir(&private).unwrap().count(), 2);

        fs::write(&entry, &source).unwrap();
        let stale = client.stage_pending_pair_wasip1(&binding, "dev", limits, &private, total, || {
            let mut writer = SqliteSourceState::open(&project, CLIENT).unwrap();
            assert!(writer.abandon("app", attempt).unwrap());
            let replacement = writer.stage("app").unwrap().pending.unwrap();
            assert_eq!(replacement.root(), attempt.root());
            assert_ne!(replacement.generation(), attempt.generation());
            trusted(&owner)
        }).unwrap();
        assert!(stale.is_none());
        assert!(client.observe_staged_pending_pair_wasip1(&staging, limits, &terminals).unwrap().is_none());
        drop(staging);
        assert_eq!(fs::read_dir(&private).unwrap().count(), 0);
        assert!(client.stage_pending_pair_wasip1(&binding, "dev", limits, &private, total,
            || panic!("do not call the owner for an already superseded attempt")).unwrap().is_none());
        assert!(store.snapshot(attempt.root()).unwrap().is_none());
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, initial.root);
        drop(owner);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn ordinary_v2_plan_reobserves_imported_source_after_real_framed_reply() {
    use std::{cell::Cell, fs::File};
    use tokio::io::duplex;
    use wosy_cli::source_state::{AcknowledgedOrdinaryLlvm, AuthorizedOrdinaryBuilderPeer};
    use wosy_daemon::BuildSessionEnd;
    use wosy_protocol::{
        encode_session_frame, write_encoded_session_frame, BuildSessionCommand, BuildSessionReply,
        BuildSessionResponse, SessionFrameReader,
    };

    let fixture = Scratch::new();
    let root = fixture.0.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("wosy.toml"), MANIFEST).unwrap();
    fs::write(root.join("main.w"), BEFORE).unwrap();
    let library = root.join("library.w");
    fs::write(&library, ORDINARY_LIB).unwrap();
    let output = fixture.0.join("must-not-write");
    let bytes = BEFORE.len() + ORDINARY_LIB.len();
    let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(File::open("/proc/self/exe").unwrap()).unwrap();
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("cache"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&root, CLIENT).unwrap();
        let base = client.stage("app").unwrap();
        let initial = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(base.root, &initial.source.delta).await.unwrap();
        assert!(client.acknowledge("app", base.pending.unwrap()).unwrap());
        let selected = client.acknowledged_build_source("app").unwrap().unwrap();

        for replace_after_reply in [false, true] {
            let (caller, mut downstream) = duplex(512);
            let mut caller = Some(caller);
            let (mut upstream, daemon) = duplex(512);
            let plan = client.live_ordinary_builder_inputs("app", "dev", TTL, bytes, 1 << 20, &expected,
                output.clone(), |capture| {
                    assert_eq!(capture, selected);
                    std::future::ready(Ok(caller.take().expect("known v2 path opens once")))
                });
            let service = worker.serve_known_ordinary_builder_input_session(daemon, selected.clone(), LIMITS, 1 << 20, TTL);
            let library = &library;
            let selected_root = base.root;
            let relay = async move {
                let (mut requests, mut replies) = (SessionFrameReader::default(), SessionFrameReader::default());
                for round in 1..=3 {
                    let request: BuildSessionCommand = requests.read(&mut downstream).await.unwrap().unwrap();
                    let request = encode_session_frame(&request).unwrap();
                    assert!(write_encoded_session_frame(&mut upstream, &request, || true).await.unwrap());
                    let reply: BuildSessionResponse<wosy_compiler::Diagnostic> = replies.read(&mut upstream).await.unwrap().unwrap();
                    if round == 2 {
                        assert!(matches!(&reply.reply, BuildSessionReply::OrdinaryLlvmProjection { projection }
                            if projection.source_root == selected_root && projection.builder.is_some()));
                        if replace_after_reply {
                            let changed = ORDINARY_LIB.replace("14", "15");
                            assert_eq!(changed.len(), ORDINARY_LIB.len());
                            fs::write(library, changed).unwrap();
                        }
                    }
                    let reply = encode_session_frame(&reply).unwrap();
                    assert!(write_encoded_session_frame(&mut downstream, &reply, || true).await.unwrap());
                }
            };
            let (plan, end, ()) = tokio::join!(plan, service, relay);
            assert_eq!(end.unwrap(), BuildSessionEnd::Disconnected);
            let plan = plan.unwrap();
            if replace_after_reply {
                assert!(matches!(plan, AcknowledgedOrdinaryLlvm::WorkspaceChanged));
            } else {
                let AcknowledgedOrdinaryLlvm::Partition(plan) = plan else { panic!("unchanged graph at exact budget"); };
                assert_eq!(plan.source().selection(), &selected);
                assert!(!plan.partition().is_empty());
            }
            assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap(), selected);
            assert!(!output.exists());
        }
        fs::write(&library, ORDINARY_LIB).unwrap();
        let opens = Cell::new(0);
        let below = client.live_ordinary_builder_inputs("app", "dev", TTL, bytes - 1, 1 << 20, &expected,
            output.clone(), |_| {
                opens.set(opens.get() + 1);
                std::future::ready(Ok(duplex(512).0))
            }).await;
        assert!(below.err().unwrap().to_string().contains("live authored source exceeds the byte budget"));
        assert_eq!(opens.get(), 0);
        assert!(!output.exists());
        drop(worker);
        Arc::into_inner(store).unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn prepares_owned_pair_from_the_first_committed_selected_source() {
    exercise_first_selected_pair(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires ordinary clang, wasm-ld, python3 and wasmtime on PATH"]
fn prepares_ordinary_builder_inputs_by_executing_first_selected_comptime() {
    exercise_first_selected_pair(true);
}

#[cfg(target_os = "linux")]
fn exercise_first_selected_pair(run_tools: bool) {
    use wosy_compiler::ScalarTargetLayout;
    use wosy_daemon::{RootedClosedComptimeRefusal, RootedClosedComptimeRequest, SelectedPairComptimePreparation};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::FileKey;

    let fixture = Scratch::new();
    let project = fixture.0.join("selected-first");
    fs::create_dir(&project).unwrap();
    let mut manifest = MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"");
    if run_tools {
        manifest = manifest.replace(r#""fixture-must-not-build""#, r#""python3", "ordinary_wasip1_pair.py", "build""#)
            .replace(r#""fixture-must-not-run""#, r#""python3", "ordinary_wasip1_pair.py", "run""#)
            .replace("[targets.app.profiles.dev]", "[targets.app.profiles.dev]\ncomptime_builder = \"nominal\"\ncomptime_runner = \"nominal\"\nbuild_permissions = []\nbuild_mounts = []");
        fs::write(project.join("ordinary_wasip1_pair.py"), include_str!("fixtures/ordinary_wasip1_pair.py")).unwrap();
    }
    fs::write(project.join("wosy.toml"), manifest).unwrap();
    fs::write(project.join("main.w"), r#"%%start
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32 first = solve(7, 37);
u32 repeated = solve(7, 37);
u32 different = solve(7, 13);
%%end
"#).unwrap();

    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&fixture.0.join("selected-cas"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&project, CLIENT).unwrap();
        let first = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(first.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", first.pending.unwrap()).unwrap());
        assert!(client.selection("app").unwrap().pending.is_none());

        let selected = client.acknowledged_build_source("app").unwrap().unwrap();
        let session = worker.open_build_diagnostics(selected, TTL).await.unwrap().unwrap();
        let request = |profile: &str, path: &str| RootedClosedComptimeRequest {
            profile: profile.into(),
            file: FileKey { package: "app".into(), path: PackageRelativePath::new(path.into()).unwrap() },
            id: "prepare".into(), evaluator_layout: ScalarTargetLayout::WASM32,
            source_limits: LIMITS, output_bytes: 1 << 20,
        };
        assert!(matches!(session.prepare_selected_pair_wasip1(request("missing", "main.w"), 11).await.unwrap(),
            SelectedPairComptimePreparation::Refused(RootedClosedComptimeRefusal::ProfileNotSelected)));
        assert!(matches!(session.prepare_selected_pair_wasip1(request("dev", "missing.w"), 11).await.unwrap(),
            SelectedPairComptimePreparation::Refused(RootedClosedComptimeRefusal::FileNotRooted)));
        let SelectedPairComptimePreparation::Ready(owner) =
            session.prepare_unique_selected_pair_wasip1("dev".into(), LIMITS, 1 << 20, 11).await.unwrap()
            else { panic!("first committed exact source should admit its unique bounded pair plan"); };
        let plan = owner.plan().expect("same live selected lease");
        assert_eq!(plan.source_root(), first.root);
        assert_eq!(plan.target(), "app");
        assert_eq!(plan.profile(), "dev");
        assert_eq!(plan.evaluators().len(), 2);
        let ordinals = plan.evidence().calls().map(|(call, _)| call.evaluator_index()).collect::<Vec<_>>();
        assert_eq!(ordinals, [0, 0, 1]);
        assert!(client.selection("app").unwrap().pending.is_none());
        if run_tools {
            use std::os::unix::fs::DirBuilderExt;
            let stage = project.join("compilation");
            fs::DirBuilder::new().mode(0o700).create(&stage).unwrap();
            let selected = client.prepare_selected_pair_ordinary_inputs(
                &owner, &stage, &project.join("artifact"), wosy_cli::source_state::SelectedPairToolBudget {
                    watch_limits: wosy_project::packages::ComptimeWatchLimits::default(),
                    evaluator_ir_bytes: 1 << 20, evaluator_module_bytes: 1 << 20, timeout: Duration::from_secs(10),
                }, || false,
            ).unwrap().expect("real first selected compilation");
            assert_eq!(selected.source().selection().root, first.root);
            assert_eq!(selected.inputs().manifest.identity.output, "wasm-wasip1");
            let ir = std::str::from_utf8(selected.partition()).unwrap();
            assert_eq!(ir.matches("i32 34").count(), 2);
            assert!(ir.contains("i32 10"));
            assert_eq!(selected.runtime(), wosy_compiler::core_runtime("llvm", "wasm-wasip1").unwrap());
            assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
            assert!(!project.join("artifact").exists());
        }
        drop(owner);
        drop(session);
        fs::write(project.join("main.w"), r#"%%start
u32(u32, u32) solve = fn(left, right) { comptime first { right ^ left } };
u32(u32, u32) duplicate = fn(left, right) { comptime second { right ^ left } };
u32 result = solve(7, 37);
%%end
"#).unwrap();
        let changed = client.stage("app").unwrap();
        let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(changed.root, &replay.source.delta).await.unwrap();
        assert!(client.acknowledge("app", changed.pending.unwrap()).unwrap());
        let session = worker.open_build_diagnostics(client.acknowledged_build_source("app").unwrap().unwrap(), TTL)
            .await.unwrap().unwrap();
        assert!(matches!(session.prepare_unique_selected_pair_wasip1("dev".into(), LIMITS, 1 << 20, 11).await.unwrap(),
            SelectedPairComptimePreparation::Refused(RootedClosedComptimeRefusal::MultipleComptimeSites)));
        drop(session);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires ordinary clang, wasm-ld, python3 and wasmtime on PATH"]
fn ordinary_cli_publishes_selected_comptime_and_checks_watched_inputs() {
    use std::process::Command;
    use wosy_project::SelectedArtifactGeneration;

    let fixture = Scratch::new();
    let root = &fixture.0;
    let manifest = MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")
        .replace(r#""fixture-must-not-build""#, r#""python3", "ordinary_wasip1_pair.py", "artifact-build""#)
        .replace(r#""fixture-must-not-run""#, r#""python3", "ordinary_wasip1_pair.py", "artifact-run""#)
        .replace("[targets.app.profiles.dev]", "[targets.app.profiles.dev]\ncomptime_builder = \"pair\"\ncomptime_runner = \"pair\"\nbuild_permissions = []\nbuild_mounts = []")
        + r#"
[builders.pair]
command = ["python3", "ordinary_wasip1_pair.py", "build"]
toolchain_identity = "ordinary WASI fixture"
[runners.pair]
command = ["python3", "ordinary_wasip1_pair.py", "run"]
[comptime.prepare]
watch = ["watch.txt"]
"#;
    fs::write(root.join("wosy.toml"), &manifest).unwrap();
    fs::write(root.join("ordinary_wasip1_pair.py"), include_str!("fixtures/ordinary_wasip1_pair.py")).unwrap();
    let watch = root.join("watch.txt");
    fs::write(&watch, b"first watched contents").unwrap();
    let source = r#"%%start
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32(bool, u32, u32) choose = fn(select_path, left, right) {
    if (select_path) { solve(left, right) } else { right }
};
u32 bypass = choose(false, 7, 37);
u32 first = choose(true, 7, bypass);
u32 repeated = solve(7, bypass);
u32 different = solve(7, 13);
%%end
"#;
    fs::write(root.join("main.w"), source).unwrap();

    let command = |verb: &str| Command::new(env!("CARGO_BIN_EXE_wosy"))
        .current_dir(root).env("WOSY_INCREMENTAL", "1").args([verb, "app"]).output().unwrap();
    let check = |verb: &str| {
        let output = command(verb);
        assert!(output.status.success(), "{verb}: {}", String::from_utf8_lossy(&output.stderr));
    };
    let generation_store = root.join(".wosy/artifacts/app/dev/main/incremental");
    let chosen = || SelectedArtifactGeneration::open(&generation_store).unwrap().unwrap();

    check("build");
    let first = chosen();
    let original = first.manifest().comptime.as_ref().expect("selected comptime observation");
    assert_eq!(first.manifest().identity.compiler_identity, wosy_cli::artifact_inputs::compiler_identity());
    assert_eq!(original.site, "prepare");
    assert_eq!(original.watches.len(), 1);
    let original_wasm = fs::read(&first.manifest().executable).unwrap();
    check("run");

    fs::write(&watch, b"changed watched contents").unwrap();
    let stale = command("run");
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("artifact comptime inputs are stale"));
    let refuse = root.join("refuse-final-builder");
    fs::write(&refuse, b"exercise the previous current generation").unwrap();
    let failed = command("build");
    assert!(!failed.status.success());
    let builder_error = String::from_utf8_lossy(&failed.stderr);
    assert!(builder_error.contains("python3 exited with exit status: 72"), "{builder_error}");
    eprintln!("{builder_error}");
    assert_eq!(chosen().manifest_path(), first.manifest_path());
    fs::remove_file(refuse).unwrap();

    check("build");
    let current = chosen();
    assert_ne!(current.manifest_path(), first.manifest_path());
    assert_ne!(current.manifest().comptime.as_ref().unwrap().watches, original.watches);
    assert_eq!(fs::read(&first.manifest().executable).unwrap(), original_wasm);
    check("run");

    let minimum = source.replace("u32", "i32")
        .replace("solve(7, 13)", "solve(1, -2147483647)");
    let parsed = wosy_syntax::parse_with_snapshot(
        wosy_syntax::SourceIdentity::new("signed".into(), "entry".into(), "main.w".into(), "signed".into()),
        minimum.clone(), &["WHY".into()],
    );
    let plan = wosy_compiler::derive_pair_startup_call_graph_wasip1_stdout(&parsed, "prepare", b"-2147483648\n".len())
        .expect("typed signed boundary must enter the exact compiler corridor");
    assert!(plan.into_bounded_wasip1_output(usize::MAX).is_ok(), "evaluator must materialize within the exact wire width");
    fs::write(root.join("main.w"), minimum).unwrap();
    fs::write(root.join("wosy.toml"), manifest.replace("\"artifact-build\"", "\"artifact-build-min\"")).unwrap();
    check("build");
    check("run");
    let signed = chosen();
    let warm = command("build");
    assert!(warm.status.success());
    assert!(String::from_utf8_lossy(&warm.stderr).contains("reused compiler LLVM after executing current comptime"));
    check("run");
    let refreshed = chosen();
    assert_ne!(signed.manifest_path(), refreshed.manifest_path());
    assert_eq!(signed.manifest().identity.llvm_input_identity, refreshed.manifest().identity.llvm_input_identity);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires ordinary clang, wasm-ld, python3 and wasmtime on PATH"]
fn ordinary_cli_compiles_all_closed_sites_in_source_order_and_publishes_once() {
    use std::process::Command;
    use wosy_project::SelectedArtifactGeneration;

    let fixture = Scratch::new();
    let root = &fixture.0;
    let manifest = MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")
        .replace(r#""fixture-must-not-build""#, r#""python3", "ordinary_wasip1_pair.py", "artifact-build-ordered""#)
        .replace(r#""fixture-must-not-run""#, r#""python3", "ordinary_wasip1_pair.py", "artifact-run""#)
        .replace("[targets.app.profiles.dev]", "[targets.app.profiles.dev]\ncomptime_builder = \"closed\"\ncomptime_runner = \"closed\"\nbuild_permissions = []\nbuild_mounts = []")
        + r#"
[builders.closed]
command = ["python3", "ordinary_wasip1_pair.py", "build"]
toolchain_identity = "ordinary ordered WASI fixture"
[runners.closed]
command = ["python3", "ordinary_wasip1_pair.py", "ordered-run"]
[comptime.zeta]
watch = ["first.txt"]
[comptime.alpha]
watch = ["second.txt"]
"#;
    fs::write(root.join("wosy.toml"), &manifest).unwrap();
    fs::write(root.join("ordinary_wasip1_pair.py"), include_str!("fixtures/ordinary_wasip1_pair.py")).unwrap();
    fs::write(root.join("first.txt"), b"first watched contents").unwrap();
    let watch = root.join("second.txt");
    fs::write(&watch, b"second watched contents").unwrap();
    let source = r#"%%start
u32 first = comptime zeta { u32 seed = 7; seed ^ 9 };
u32 second = comptime alpha { u32 base = 13; base ^ 7 };
u32 tied = first ^ second;
%%end
"#;
    let source_path = root.join("main.w");
    fs::write(&source_path, source).unwrap();
    let order = root.join("evaluator-order");
    // Same authored bytes and config do not permit crossing project owners.
    // The source root itself already contains project identity; verify both
    // clients have a committed selected root before invoking the wrong owner.
    let twin = Scratch::new();
    fs::write(twin.0.join("wosy.toml"), &manifest).unwrap();
    fs::write(twin.0.join("main.w"), source).unwrap();
    let receipts = Scratch::new();
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        use wosy_cli::local_build_daemon::establish_local_build_source;
        use wosy_cli::source_state::SelectedPairToolBudget;
        use wosy_daemon::SelectedComptimePreparation;
        let store = Arc::new(SourceStore::open(&receipts.0.join("cas"), 8 << 20).unwrap());
        let mut actual = SqliteSourceState::open(root, CLIENT).unwrap();
        let mut other = SqliteSourceState::open(&twin.0, CLIENT).unwrap();
        assert!(establish_local_build_source(&mut actual, &store, "app").await.unwrap());
        assert!(establish_local_build_source(&mut other, &store, "app").await.unwrap());
        let actual_selection = actual.acknowledged_build_source("app").unwrap().unwrap();
        let other_selection = other.acknowledged_build_source("app").unwrap().unwrap();
        assert_ne!(actual_selection.root, other_selection.root);
        assert_ne!(actual_selection.project, other_selection.project);
        let one = NonZeroUsize::MIN;
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let session = worker.open_build_diagnostics(other_selection, TTL).await.unwrap().unwrap();
        let SelectedComptimePreparation::Ordered(owner) = session.prepare_selected_wasip1_comptime(
            "dev".into(), LIMITS, usize::MAX,
        ).await.unwrap() else { panic!("other valid selected owner"); };
        assert!(actual.prepare_selected_ordered_ordinary_inputs(
            &owner, &receipts.0, &root.join("must-not-publish"), SelectedPairToolBudget {
                watch_limits: wosy_project::packages::ComptimeWatchLimits::default(),
                evaluator_ir_bytes: usize::MAX, evaluator_module_bytes: u64::MAX, timeout: TTL,
            }, || false,
        ).unwrap().is_none());
        assert!(!order.exists());
        assert!(!root.join("must-not-publish").exists());
        drop(owner);
        drop(session);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });

    let command = |verb: &str| Command::new(env!("CARGO_BIN_EXE_wosy"))
        .current_dir(root).env("WOSY_INCREMENTAL", "1").args([verb, "app"]).output().unwrap();
    let checked = |verb: &str| {
        let output = command(verb);
        assert!(output.status.success(), "{verb}: {}", String::from_utf8_lossy(&output.stderr));
    };
    let chosen = || SelectedArtifactGeneration::open(&root.join(".wosy/artifacts/app/dev/main/incremental"))
        .unwrap().unwrap();

    checked("build");
    checked("run");
    assert_eq!(fs::read(&order).unwrap(), b"14\n10\n");
    let first = chosen();
    let original = first.manifest().comptime.as_ref().unwrap();
    assert_ne!(first.manifest().identity.compiler_identity, wosy_cli::artifact_inputs::compiler_identity());
    assert!(first.manifest().identity.compiler_identity.ends_with(":ordered-comptime-v1"));
    assert_eq!(original.site, "zeta");
    assert_eq!(original.additional_sites.iter().map(|site| site.site.as_str()).collect::<Vec<_>>(), ["alpha"]);
    let retained = fs::read(&first.manifest().executable).unwrap();

    fs::write(&watch, b"changed second site only").unwrap();
    let stale = command("run");
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("artifact comptime inputs are stale"));

    let refuse = root.join("refuse-second-evaluator");
    fs::write(&refuse, b"the first evaluator succeeds; publication must not happen").unwrap();
    let failed = command("build");
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("selected comptime runner failed"));
    assert_eq!(fs::read(&order).unwrap(), b"14\n10\n14\n10\n");
    assert_eq!(chosen().manifest_path(), first.manifest_path());
    fs::remove_file(&refuse).unwrap();

    checked("build");
    checked("run");
    assert_eq!(fs::read(&order).unwrap(), b"14\n10\n14\n10\n14\n10\n");
    let current = chosen();
    let observation = current.manifest().comptime.as_ref().unwrap();
    assert_ne!(current.manifest_path(), first.manifest_path());
    assert_eq!(observation.watches, original.watches);
    assert_ne!(observation.additional_sites[0].watches, original.additional_sites[0].watches);
    assert_ne!(observation.result_root, original.result_root);
    assert_eq!(fs::read(&first.manifest().executable).unwrap(), retained);

    let warm = command("build");
    assert!(warm.status.success());
    assert!(String::from_utf8_lossy(&warm.stderr).contains("reused compiler LLVM after executing current comptime"));
    checked("run");
    let refreshed = chosen();
    assert_ne!(current.manifest_path(), refreshed.manifest_path());
    assert_eq!(current.manifest().identity.llvm_input_identity, refreshed.manifest().identity.llvm_input_identity);
    assert_eq!(fs::read(&order).unwrap(), b"14\n10\n14\n10\n14\n10\n14\n10\n");

    // A second body capturing the first is outside the closed plan; do
    // not run any of the batch or admit a result for the supported prefix.
    fs::write(&source_path, source.replace("base ^ 7", "base ^ first")).unwrap();
    assert!(!command("build").status.success());
    assert_eq!(fs::read(&order).unwrap(), b"14\n10\n14\n10\n14\n10\n14\n10\n");
    assert_eq!(chosen().manifest_path(), refreshed.manifest_path());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires ordinary clang, wasm-ld, python3 and wasmtime on PATH"]
fn ordinary_cli_specializes_only_the_concrete_wasm_export_roots() {
    use std::process::Command;
    use wosy_project::SelectedArtifactGeneration;

    let fixture = Scratch::new();
    let project = &fixture.0;
    let manifest = MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")
        .replace(r#""fixture-must-not-build""#, r#""python3", "ordinary_wasip1_pair.py", "artifact-build-exported""#)
        .replace(r#""fixture-must-not-run""#, r#""python3", "ordinary_wasip1_pair.py", "artifact-run""#)
        + r#"
[targets.app.artifacts.main.profiles.dev.exports]
west = "first"
south = "again"
north = "other"
mirror = "first"
[targets.app.artifacts.main.profiles.subset]
backend = "llvm"
output = "wasm-wasip1"
builder = "nominal"
run_runner = "nominal"
[targets.app.artifacts.main.profiles.subset.exports]
north = "other"
[targets.app.profiles.subset]
main_artifact = "main"
comptime_builder = "pair"
comptime_runner = "pair"
build_permissions = []
build_mounts = []
[builders.pair]
command = ["python3", "ordinary_wasip1_pair.py", "build"]
toolchain_identity = "ordinary exported WASI fixture"
[runners.pair]
command = ["python3", "ordinary_wasip1_pair.py", "ordered-run"]
[comptime.calculate]
watch = ["watch.txt"]
[targets.utility]
root = "utility.w"
[targets.utility.artifacts.main]
artifact = "Utility"
[targets.utility.artifacts.main.profiles.dev]
backend = "llvm"
output = "wasm-wasip1"
builder = "nominal"
run_runner = "nominal"
[targets.utility.artifacts.main.profiles.dev.exports]
util = "other"
[targets.utility.profiles.dev]
main_artifact = "main"
comptime_builder = "pair"
comptime_runner = "pair"
build_permissions = []
build_mounts = []
[comptime.calculate_utility]
watch = ["utility-watch.txt"]
"#;
    let manifest = manifest.replace("[targets.app.profiles.dev]", r#"
[targets.app.profiles.dev]
comptime_builder = "pair"
comptime_runner = "pair"
build_permissions = []
build_mounts = []
"#);
    let manifest_path = project.join("wosy.toml");
    fs::write(&manifest_path, &manifest).unwrap();
    fs::write(project.join("ordinary_wasip1_pair.py"), include_str!("fixtures/ordinary_wasip1_pair.py")).unwrap();
    let watch = project.join("watch.txt");
    fs::write(&watch, b"first watched contents").unwrap();
    let source = r#"%%start
u32(u32, u32) solve = fn(left, right) { comptime calculate { right ^ left } };
u32() first = fn { solve(7, 37) };
u32() again = fn { solve(7, 37) };
u32() other = fn { solve(7, 13) };
u32() hidden = fn { solve(21, 34) };
u32() nonliteral = fn { 5 };
u32() unknown = fn { solve(nonliteral(), 1) };
%%end
"#;
    fs::write(project.join("main.w"), source).unwrap();
    fs::write(project.join("utility.w"), source.replace("comptime calculate ", "comptime calculate_utility ")).unwrap();
    let utility_watch = project.join("utility-watch.txt");
    fs::write(&utility_watch, b"utility input").unwrap();
    let configuration = wosy_project::ProjectConfiguration::load(project).unwrap();
    let (_, _, artifact_profile, _) = configuration.artifact_profile("app", "dev").unwrap();
    let parsed = wosy_syntax::parse_with_snapshot(
        wosy_syntax::SourceIdentity::new(project.to_string_lossy().into_owned(), "app".into(), "main.w".into(), "authored".into()),
        source.to_owned(), &["WHY".into()],
    );
    let typed_plan = wosy_compiler::prepare_exported_pair_wasip1(
        &parsed, artifact_profile.exports.as_ref().unwrap(), usize::MAX,
    ).expect("expected the exact selected ordinary compatibility boundary to admit both tuples");
    assert_eq!(typed_plan.tuples().len(), 2);

    let order = project.join("evaluator-order");
    let partition_command = |verb: &str, target: &str, profile: &str| Command::new(env!("CARGO_BIN_EXE_wosy"))
        .current_dir(project).env("WOSY_INCREMENTAL", "1").args([verb, target, "--profile", profile])
        .output().unwrap();
    let command = |verb: &str, profile: &str| partition_command(verb, "app", profile);
    let checked = |verb: &str, profile: &str| {
        let output = command(verb, profile);
        assert!(output.status.success(), "{verb}/{profile}: {}", String::from_utf8_lossy(&output.stderr));
    };
    let chosen_partition = |target: &str, profile: &str| SelectedArtifactGeneration::open(
        &project.join(format!(".wosy/artifacts/{target}/{profile}/main/incremental")),
    ).unwrap().unwrap();
    let chosen = |profile: &str| chosen_partition("app", profile);
    let read_ir = |choice: &SelectedArtifactGeneration| fs::read_to_string(
        choice.manifest_path().parent().unwrap().join("partition.ll"),
    ).unwrap();

    let invoke = |generation: &SelectedArtifactGeneration, export: &str| Command::new("wasmtime")
        .args(["run", "-C", "cache=n", "--invoke", export])
        .arg(&generation.manifest().executable).output().unwrap();
    let check_export = |generation: &SelectedArtifactGeneration, export: &str, expected: &str| {
        let output = invoke(generation, export);
        assert!(output.status.success(), "{export}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
    };
    checked("build", "dev");
    checked("run", "dev");
    let first = chosen("dev");
    let original_wasm = fs::read(&first.manifest().executable).unwrap();
    let observation = first.manifest().comptime.as_ref().unwrap();
    assert_eq!(observation.site, "calculate");
    for (export, expected) in [("west", "34"), ("south", "34"), ("north", "10"), ("mirror", "34")] {
        check_export(&first, export, expected);
    }
    assert!(!invoke(&first, "hidden").status.success());
    assert_eq!(fs::read(&order).unwrap(), b"34\n10\n", "three wrappers and one alias, only two evaluated tuples");
    let ir = read_ir(&first);
    for selected in ["west", "south", "north", "mirror"] {
        assert!(ir.contains(&format!("@{selected}(")), "{ir}");
    }
    for removed in ["solve", "hidden", "unknown", "nonliteral"] {
        assert!(!ir.contains(&format!("@{removed}(")), "{ir}");
    }

    checked("build", "subset");
    checked("run", "subset");
    assert_eq!(fs::read(&order).unwrap(), b"34\n10\n10\n");
    let subset = chosen("subset");
    check_export(&subset, "north", "10");
    assert!(!invoke(&subset, "west").status.success());
    assert!(read_ir(&subset).contains("@north("));
    assert!(!read_ir(&subset).contains("@west("));
    assert_eq!(chosen("dev").manifest_path(), first.manifest_path());

    // Watch changes invalidate both selected partitions before a runner.
    fs::write(&watch, b"new watched contents").unwrap();
    for profile in ["dev", "subset"] {
        let stale = command("run", profile);
        assert!(!stale.status.success());
        assert!(String::from_utf8_lossy(&stale.stderr).contains("artifact comptime inputs are stale"));
    }
    let refuse = project.join("refuse-second-evaluator");
    fs::write(&refuse, b"later evaluator fails").unwrap();
    let failed = command("build", "dev");
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("selected comptime runner failed"));
    assert_eq!(chosen("dev").manifest_path(), first.manifest_path());
    fs::remove_file(refuse).unwrap();

    checked("build", "dev");
    checked("run", "dev");
    let current = chosen("dev");
    assert_ne!(current.manifest_path(), first.manifest_path());
    assert_ne!(current.manifest().comptime.as_ref().unwrap().result_root, observation.result_root);
    assert_eq!(fs::read(&first.manifest().executable).unwrap(), original_wasm);

    // A compiler LLVM hit is allowed only after both evaluated results
    // are fresh, never instead of evaluator or artifact processes.
    let hit = "reused compiler LLVM after executing current comptime";
    let build = |warm: bool| {
        let start = std::time::Instant::now();
        let output = command("build", "dev");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(stderr.contains(hit), warm, "{stderr}");
        eprintln!("dev LLVM receipt {}: {} ms including ordinary tool processes",
            if warm { "hit" } else { "miss" }, start.elapsed().as_millis());
        eprintln!("{}", stderr.lines().find(|line| line.contains("LLVM") && line.contains("evaluator tools")).unwrap());
    };
    fs::write(project.join("audit-compiler-reuse"), b"count ordinary process invocations").unwrap();
    let count = |file: &str| fs::read_to_string(project.join(file)).unwrap().lines().count();
    let start_runs = count("evaluator-order");
    build(true);
    checked("run", "dev");
    let warm = chosen("dev");
    assert_ne!(warm.manifest_path(), current.manifest_path());
    assert_eq!(read_ir(&warm), read_ir(&current));
    assert_eq!(count("evaluator-builds"), 2);
    assert_eq!(count("evaluator-order"), start_runs + 2);
    assert_eq!(count("final-builds"), 1);

    // Corrupt only a disposable generation, then re-derive LLVM from
    // the actual same typed terminal results rather than trusting bytes.
    fs::write(warm.manifest_path().parent().unwrap().join("partition.ll"), b"invalid LLVM").unwrap();
    build(false);
    checked("run", "dev");
    let repaired = chosen("dev");
    assert_eq!(read_ir(&repaired), read_ir(&current));

    // Changed terminal value with the same source, manifest and watches
    // still invalidates the compiler receipt.
    fs::write(project.join("different-result"), b"runner returns eleven").unwrap();
    build(false);
    checked("run", "dev");
    let changed = chosen("dev");
    let before = repaired.manifest().comptime.as_ref().unwrap();
    let after = changed.manifest().comptime.as_ref().unwrap();
    assert_eq!(before.source_root, after.source_root);
    assert_eq!(before.configuration_inputs, after.configuration_inputs);
    assert_eq!(before.watches, after.watches);
    assert_ne!(before.result_root, after.result_root);
    assert_ne!(read_ir(&repaired), read_ir(&changed));
    assert_ne!(repaired.manifest().identity.llvm_input_identity, changed.manifest().identity.llvm_input_identity);
    check_export(&changed, "north", "11");
    build(true);
    checked("run", "dev");
    let current = chosen("dev");
    assert_eq!(read_ir(&current), read_ir(&changed));
    assert_eq!(count("evaluator-builds"), 8);
    assert_eq!(count("evaluator-order"), start_runs + 8);
    assert_eq!(count("final-builds"), 4);

    fs::remove_file(project.join("different-result")).unwrap();
    let check_partition_build = |target: &str, profile: &str, warm: bool, evaluator_count: usize| {
        let before = (count("evaluator-builds"), count("evaluator-order"), count("final-builds"));
        let started = std::time::Instant::now();
        let output = partition_command("build", target, profile);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{target}/{profile}: {stderr}");
        assert_eq!(stderr.contains(hit), warm, "{target}/{profile}: {stderr}");
        assert_eq!((count("evaluator-builds"), count("evaluator-order"), count("final-builds")),
            (before.0 + evaluator_count, before.1 + evaluator_count, before.2 + 1));
        let run = partition_command("run", target, profile);
        assert!(run.status.success(), "{target}/{profile}: {}", String::from_utf8_lossy(&run.stderr));
        eprintln!("{target}/{profile} build+run {} ms: {}", started.elapsed().as_millis(),
            stderr.lines().find(|line| line.contains("LLVM") && line.contains("evaluator tools")).unwrap());
    };
    // All three partitions have their own histories and tool counts.
    check_partition_build("app", "dev", false, 2);
    check_partition_build("app", "subset", false, 1);
    check_partition_build("utility", "dev", false, 1);
    for (target, profile, expected) in [("app", "dev", 2), ("app", "subset", 1), ("utility", "dev", 1)] {
        check_partition_build(target, profile, true, expected);
    }
    let before_dev = chosen("dev");
    let before_subset = chosen("subset");
    let before_utility = chosen_partition("utility", "dev");

    // Rename only one actual export of dev. The whole TOML/root changes,
    // but neither independent partition's LLVM inputs do.
    let revised_manifest = manifest.replace("mirror = \"first\"", "replica = \"first\"");
    fs::write(&manifest_path, &revised_manifest).unwrap();
    check_partition_build("app", "dev", false, 2);
    check_partition_build("app", "subset", true, 1);
    check_partition_build("utility", "dev", true, 1);
    let revised_dev = chosen("dev");
    let revised_subset = chosen("subset");
    let revised_utility = chosen_partition("utility", "dev");
    assert_ne!(read_ir(&revised_dev), read_ir(&before_dev));
    check_export(&revised_dev, "replica", "34");
    assert!(!invoke(&revised_dev, "mirror").status.success());
    for (prior, revised) in [(&before_subset, &revised_subset), (&before_utility, &revised_utility)] {
        assert_ne!(revised.manifest_path(), prior.manifest_path());
        assert_ne!(revised.manifest().identity.configuration_identity, prior.manifest().identity.configuration_identity);
        let old = prior.manifest().comptime.as_ref().unwrap();
        let new = revised.manifest().comptime.as_ref().unwrap();
        assert_ne!(old.source_root, new.source_root);
        assert_ne!(old.result_root, new.result_root);
        assert_eq!(old.watches, new.watches);
        assert_eq!(read_ir(prior), read_ir(revised));
    }

    // Only utility uses utility-watch.txt; changing it cannot invalidate
    // app's selected partitions or their warm compiler receipts.
    fs::write(&utility_watch, b"utility input changed").unwrap();
    let utility_stale = partition_command("run", "utility", "dev");
    assert!(String::from_utf8_lossy(&utility_stale.stderr).contains("artifact comptime inputs are stale"));
    check_partition_build("utility", "dev", false, 1);
    check_partition_build("app", "dev", true, 2);
    check_partition_build("app", "subset", true, 1);
    let current = chosen("dev");

    // Making the unknown edge reachable rejects the complete plan before
    // any selected evaluator or the final artifact builder is called.
    let prior = fs::read(&order).unwrap();
    fs::write(&manifest_path, revised_manifest.replace("replica = \"first\"", "replica = \"unknown\"")).unwrap();
    assert!(!command("build", "dev").status.success());
    assert_eq!(fs::read(order).unwrap(), prior);
    assert_eq!(chosen("dev").manifest_path(), current.manifest_path());
}
