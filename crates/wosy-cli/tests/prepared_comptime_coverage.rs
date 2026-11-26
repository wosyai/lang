use std::{fs, num::NonZeroUsize, path::PathBuf, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::{PendingBasePublication, ReplaySource, SourceAttempt, SqliteSourceState}};
use wosy_daemon::{AuthoredComptimeCoverage, BaseCstAdmissionLimits, RootedLlvmProjection, SourceStorageWorker, SourceStore};

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
