use std::{fs, num::NonZeroUsize, path::{Path, PathBuf}, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::{ReplaySource, SqliteSourceState}};
use wosy_daemon::{BaseCstAdmissionLimits, SourcePinPage, SourceStorageWorker, SourceStore};
use wosy_protocol::{BuildSourceSelection, SourcePinReflection, SourcePinRetention};

const CLIENT: ClientSourceLimits = ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 };
const ADMISSION: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
    records: 25_000, record_bytes: 16 << 20, graph_edges: 100_000, expanded_record_bytes: 32 << 20,
    cst_occurrences: 100_000, cst_depth: 128, authored_bytes: 2 << 20,
};
const MANIFEST: &str = r#"
[package]
name = "app"
default_target = "app"
[comments]
categories = []
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

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("wosy-daemon-pin-page-{}-{now}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    async fn project(&self, store: &SourceStore, name: &str) -> (SqliteSourceState, BuildSourceSelection) {
        let root = self.0.join(name);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("wosy.toml"), MANIFEST).unwrap();
        fs::write(root.join("main.w"), "%%start\nu32 value = 42;\n%%end\n").unwrap();
        acknowledged_project(store, &root).await
    }
}

impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

async fn acknowledged_project(store: &SourceStore, root: &Path) -> (SqliteSourceState, BuildSourceSelection) {
    let mut local = SqliteSourceState::open(root, CLIENT).unwrap();
    let stage = local.stage("app").unwrap();
    assert_eq!((stage.parsed, stage.reused), (1, 0));
    let replay = local.replay("app", ReplaySource::Pending).unwrap().unwrap();
    store.publish(replay.source.root, &replay.source.delta).await.unwrap();
    assert!(local.acknowledge("app", stage.pending.unwrap()).unwrap());
    let selected = local.acknowledged_build_source("app").unwrap().unwrap();
    assert_eq!(selected.root, replay.source.root);
    (local, selected)
}

fn rows(rows: usize) -> NonZeroUsize { NonZeroUsize::new(rows).unwrap() }

fn names(page: &SourcePinPage) -> Vec<&str> { page.pins.iter().map(|pin| pin.name.as_str()).collect() }

fn local_worker(store: &Arc<SourceStore>) -> SourceStorageWorker {
    SourceStorageWorker::new(Arc::clone(store), tokio::runtime::Handle::current(), rows(1), rows(2), rows(20_000)).unwrap()
}

#[test]
fn source_pin_page_advances_over_real_reflected_releases_and_keeps_prefix_names() {
    let owned = Scratch::new();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    runtime.block_on(async {
        let store = Arc::new(SourceStore::open(&owned.0.join("graph"), 8 << 20).unwrap());
        let (mut local, selected) = owned.project(&store, "project").await;
        let worker = local_worker(&store);
        for name in ["a", "a-prefix", "a-tail"] {
            let pinned = local.pin_selected_source("app", name).unwrap();
            let retained = local.source_pin_intent(name).unwrap().unwrap();
            assert_eq!(worker.reflect_source_pin(retained.clone(), None, ADMISSION).await.unwrap(),
                SourcePinReflection::Durable(retained));
            assert!(local.unpin_local_source(&pinned).unwrap());
            let released = local.source_pin_intent(name).unwrap().unwrap();
            assert_eq!(released.retention, SourcePinRetention::Released);
            assert_eq!(worker.reflect_source_pin(released.clone(), None, ADMISSION).await.unwrap(),
                SourcePinReflection::Durable(released));
        }
        store.pin_source(selected.root, "b").await.unwrap();
        // One real reflected live row and one direct pin cover both formats.
        local.pin_selected_source("app", "b-prefix").unwrap();
        let reflected = local.source_pin_intent("b-prefix").unwrap().unwrap();
        assert_eq!(worker.reflect_source_pin(reflected.clone(), None, ADMISSION).await.unwrap(),
            SourcePinReflection::Durable(reflected));
        store.pin_source(selected.root, "é").await.unwrap();
        let before = local.selection("app").unwrap();
        let old_names = store.source_pins(selected.root).unwrap().into_iter().map(|pin| pin.name).collect::<Vec<_>>();

        let session = worker.open_build_diagnostics(selected.clone(), Duration::from_secs(30)).await.unwrap().unwrap();
        let strict = BaseCstAdmissionLimits { records: 1, ..ADMISSION };
        let error = session.source_pin_page(None, rows(2), strict).await.err().unwrap();
        assert_eq!(error.to_string(), "source index exceeds the record budget");
        let first = session.source_pin_page(None, rows(2), ADMISSION).await.unwrap();
        assert!(first.pins.is_empty());
        assert_eq!(first.visible_rows_visited, 3);
        let next = first.continuation.unwrap();
        // The continuation uses the admitted immutable owner, not another
        // unbounded source/config read. The live lease is checked again.
        let continuation_budget = BaseCstAdmissionLimits { records: 0, record_bytes: 0, ..ADMISSION };
        let second = session.source_pin_page(Some(next), rows(2), continuation_budget).await.unwrap();
        assert_eq!(names(&second), ["b"]);
        assert_eq!(second.visible_rows_visited, 3);
        let third = session.source_pin_page(second.continuation, rows(2), continuation_budget).await.unwrap();
        assert_eq!(names(&third), ["b-prefix", "é"]);
        assert_eq!(third.visible_rows_visited, 2);
        assert!(third.continuation.is_none());
        assert!(second.pins.iter().chain(third.pins.iter()).all(|pin| pin.root == selected.root));
        assert_eq!(local.selection("app").unwrap(), before);
        assert_eq!(store.source_pins(selected.root).unwrap().into_iter().map(|pin| pin.name).collect::<Vec<_>>(), old_names);
        session.release();
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[test]
fn source_pin_page_rejects_another_session_and_observes_fresh_local_cuts() {
    let owned = Scratch::new();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    runtime.block_on(async {
        let store = Arc::new(SourceStore::open(&owned.0.join("graph"), 8 << 20).unwrap());
        let (_local, selected) = owned.project(&store, "one").await;
        let (_other, elsewhere) = owned.project(&store, "two").await;
        let worker = local_worker(&store);
        for name in ["a", "z"] { store.pin_source(selected.root, name).await.unwrap(); }
        store.pin_source(elsewhere.root, "another-workspace").await.unwrap();

        let open = |chosen| worker.open_build_diagnostics(chosen, Duration::from_secs(30));
        let first = open(selected.clone()).await.unwrap().unwrap();
        let same_source = open(selected.clone()).await.unwrap().unwrap();
        let foreign = open(elsewhere).await.unwrap().unwrap();
        let first_page = first.source_pin_page(None, rows(1), ADMISSION).await.unwrap();
        assert_eq!(names(&first_page), ["a"]);
        assert_eq!(first_page.visible_rows_visited, 2);
        let cursor = first_page.continuation.unwrap();
        for session in [&same_source, &foreign] {
            let error = session.source_pin_page(Some(cursor.clone()), rows(1), ADMISSION).await.err().unwrap();
            assert_eq!(error.to_string(), "source pin cursor belongs to another opened session");
        }
        assert_eq!(names(&foreign.source_pin_page(None, rows(1), ADMISSION).await.unwrap()), ["another-workspace"]);
        store.pin_source(selected.root, "b").await.unwrap();
        let resumed = first.source_pin_page(Some(cursor.clone()), rows(2), ADMISSION).await.unwrap();
        assert_eq!(names(&resumed), ["b", "z"]); // new read transaction, no retained MVCC view.
        assert!(resumed.continuation.is_none());
        first.release();
        let error = same_source.source_pin_page(Some(cursor), rows(1), ADMISSION).await.err().unwrap();
        assert_eq!(error.to_string(), "source pin cursor belongs to another opened session");

        let expires = worker.open_build_diagnostics(selected, Duration::from_millis(20)).await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(!expires.renew());
        let error = expires.source_pin_page(None, rows(1), ADMISSION).await.err().unwrap();
        assert_eq!(error.to_string(), "Build diagnostic session lease expired");
        expires.release(); same_source.release(); foreign.release();
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

#[test]
fn source_pin_intent_observation_matches_only_the_actual_mirror_without_mutation() {
    use wosy_cli::source_state::PinIntentPageRead;
    let owned = Scratch::new();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    runtime.block_on(async {
        let store = Arc::new(SourceStore::open(&owned.0.join("graph"), 8 << 20).unwrap());
        let (mut local, selected) = owned.project(&store, "project").await;
        let (mut other, _) = owned.project(&store, "other").await;
        let worker = local_worker(&store);
        for name in ["a-exact", "b-missing", "c-direct", "d-older", "e-release", "f-reflected-release"] {
            let pin = local.pin_selected_source("app", name).unwrap();
            if matches!(name, "a-exact" | "d-older" | "e-release" | "f-reflected-release") {
                let initial = local.source_pin_intent(name).unwrap().unwrap();
                assert_eq!(worker.reflect_source_pin(initial.clone(), None, ADMISSION).await.unwrap(),
                    SourcePinReflection::Durable(initial));
            }
            if name == "d-older" {
                // Same immutable source root, but a different real client generation.
                let next = local.pin_selected_source("app", name).unwrap();
                assert_eq!(next.root, pin.root);
                assert!(next.generation > pin.generation);
            }
            if matches!(name, "e-release" | "f-reflected-release") {
                assert!(local.unpin_local_source(&pin).unwrap());
            }
            if name == "f-reflected-release" {
                let released = local.source_pin_intent(name).unwrap().unwrap();
                assert_eq!(released.retention, SourcePinRetention::Released);
                assert_eq!(worker.reflect_source_pin(released.clone(), None, ADMISSION).await.unwrap(),
                    SourcePinReflection::Durable(released));
            }
        }
        store.pin_source(selected.root, "c-direct").await.unwrap();
        other.pin_selected_source("app", "a-exact").unwrap();
        let foreign = other.source_pin_intent("a-exact").unwrap().unwrap();
        let root = selected.root;
        let direct_rows = || store.source_pins(root).unwrap().into_iter()
            .map(|pin| (pin.name, pin.root)).collect::<Vec<_>>();
        let before = direct_rows();
        let selection_before = local.selection("app").unwrap();
        let mut cursor = local.begin_source_pin_scan().unwrap();
        let first_cursor = cursor.clone();
        let session = worker.open_build_diagnostics(selected, Duration::from_secs(30)).await.unwrap().unwrap();
        let mut seen = Vec::new();
        loop {
            let PinIntentPageRead::Observed(page) = local.read_source_pin_intent_page(&cursor, rows(2)).unwrap()
                else { panic!("read-only fixture changed its local revision"); };
            assert!(page.intents.len() <= 2);
            for intent in &page.intents {
                assert_eq!(local.source_pin_intent(&intent.name).unwrap().as_ref(), Some(intent));
            }
            let names: Vec<_> = page.intents.iter().map(|intent| intent.name.clone()).collect();
            let matches = session.observe_source_pin_intents(page.intents, ADMISSION).await.unwrap();
            assert_eq!(matches.len(), names.len());
            assert!(local.source_pin_scan_is_current(&cursor).unwrap());
            seen.extend(names.into_iter().zip(matches));
            let Some(next) = page.continuation else { break };
            cursor = next;
        }
        assert_eq!(seen, [
            ("a-exact".into(), true), ("b-missing".into(), false), ("c-direct".into(), false),
            ("d-older".into(), false), ("e-release".into(), false), ("f-reflected-release".into(), true),
        ]);
        let error = session.observe_source_pin_intents(vec![foreign], ADMISSION).await.err().unwrap();
        assert_eq!(error.to_string(), "source pin intent belongs to another workspace");
        let exact = local.source_pin_intent("a-exact").unwrap().unwrap();
        let error = session.observe_source_pin_intents(vec![exact; 65], ADMISSION).await.err().unwrap();
        assert_eq!(error.to_string(), "source pin observation exceeds the page window");
        assert_eq!(direct_rows(), before);
        assert_eq!(local.selection("app").unwrap(), selection_before);
        assert!(local.source_pin_scan_is_current(&first_cursor).unwrap());
        local.pin_selected_source("app", "b-missing").unwrap();
        assert!(!local.source_pin_scan_is_current(&first_cursor).unwrap());
        assert!(matches!(local.read_source_pin_intent_page(&first_cursor, rows(2)).unwrap(), PinIntentPageRead::RestartRequired));
        assert_eq!(direct_rows(), before);
        session.release();
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}
