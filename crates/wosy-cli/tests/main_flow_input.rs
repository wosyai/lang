use std::{fs, path::{Path, PathBuf}, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::time::Instant;
use wosy_cli::{
    source_recovery::ClientSourceLimits,
    source_state::{ReplaySource, SelectedMainFlowInput, SelectedMainFlowPreparation, SqliteSourceState},
};
use wosy_daemon::{MainFlowInputPreparation, MainFlowInputRefusal, SourceStore};

const INPUT: &str = "%%start\n# WHY: café ☕\ncore.require(true, { .stdout = \"\"; .stderr = \"\"; .code = 0; .files = []; });\ncore.require(false, { .stdout = \"first\"; .stderr = \"\"; .code = 19; .files = []; });\n%%end\n";
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
[targets.app.artifacts.main.profiles.wasm]
backend = "llvm"
output = "wasm-wasip1"
builder = "nominal"
run_runner = "nominal"
[targets.app.profiles.dev]
main_artifact = "main"
[targets.app.profiles.wasm]
main_artifact = "main"
"#;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn project() -> Scratch {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("wosy-main-flow-input-{}-{stamp}", std::process::id()));
    fs::create_dir(&path).unwrap();
    fs::write(path.join("main.w"), INPUT).unwrap();
    fs::write(path.join("wosy.toml"), MANIFEST).unwrap();
    Scratch(path)
}
fn client(path: &Path) -> SqliteSourceState {
    SqliteSourceState::open(path, ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 }).unwrap()
}
async fn select(client: &mut SqliteSourceState, store: &SourceStore) {
    let stage = client.stage("app").unwrap();
    let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
    store.publish(stage.root, &replay.source.delta).await.unwrap();
    assert!(client.acknowledge("app", stage.pending.unwrap()).unwrap());
}
fn deadline() -> Instant { Instant::now() + Duration::from_secs(5) }
fn prepare(client: &mut SqliteSourceState, store: &SourceStore, profile: &str, prior: Option<&SelectedMainFlowInput>)
    -> SelectedMainFlowPreparation
{
    client.prepare_selected_main_flow_source(store, "app", profile, deadline(), 1 << 20, prior).unwrap()
}
fn ready(result: SelectedMainFlowPreparation) -> SelectedMainFlowInput {
    let SelectedMainFlowPreparation::Ready(input) = result else { panic!("expected an exact authored input"); };
    input
}

#[test]
fn source_handoff_binds_exact_authored_project_profile_and_delivery_generation() {
    let scratch = project();
    let root = &scratch.0;
    let mut client = client(root);
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    rt.block_on(async {
        let store = SourceStore::open(&root.join("store"), 8 << 20).unwrap();
        assert!(matches!(prepare(&mut client, &store, "dev", None), SelectedMainFlowPreparation::NotAcknowledged));
        select(&mut client, &store).await;

        let native = ready(prepare(&mut client, &store, "dev", None));
        let warm = ready(prepare(&mut client, &store, "dev", Some(&native)));
        let wasm = ready(prepare(&mut client, &store, "wasm", Some(&native)));
        let wasm_warm = ready(prepare(&mut client, &store, "wasm", Some(&wasm)));
        assert!(!native.reused_input && warm.reused_input && !wasm.reused_input && wasm_warm.reused_input);
        assert_eq!(native.input().text(), INPUT);
        assert_eq!(native.input().categories(), &["WHY"]);
        assert_eq!(native.input().layout().pointer_size, 8);
        assert_eq!(wasm.input().layout().pointer_size, 4);
        assert_eq!(native.input().source(), wasm.input().source());
        assert_eq!(native.input().shape(), wasm.input().shape());
        assert_eq!(native.input().identity(), warm.input().identity());
        assert_ne!(native.input().identity(), wasm.input().identity());
        assert!(matches!(prepare(&mut client, &store, "missing", Some(&native)), SelectedMainFlowPreparation::ProfileNotSelected));

        let mut wrong = native.source().selection().clone();
        wrong.project.push_str("-other");
        assert!(matches!(store.prepare_common_main_flow_source(&wrong, "dev", None).unwrap(),
            MainFlowInputPreparation::Refused(MainFlowInputRefusal::SourceUnavailable)));
        let empty_store = SourceStore::open(&root.join("empty-store"), 8 << 20).unwrap();
        assert!(matches!(prepare(&mut client, &empty_store, "dev", Some(&native)),
            SelectedMainFlowPreparation::Refused(MainFlowInputRefusal::SourceUnavailable)));
        empty_store.close().await.unwrap();

        // Exact preparation, not proof timings. Warm compares only against an
        // identical retained input and still undergoes live source checks.
        let mut native_samples = vec![];
        let mut wasm_samples = vec![];
        for (profile, prior, samples) in [("dev", &native, &mut native_samples), ("wasm", &wasm, &mut wasm_samples)] {
            for _ in 0..7 {
                let cold = ready(prepare(&mut client, &store, profile, None));
                let warm = ready(prepare(&mut client, &store, profile, Some(prior)));
                assert!(warm.reused_input);
                samples.push((cold.daemon_input_elapsed.as_micros(), warm.daemon_input_elapsed.as_micros()));
            }
        }
        eprintln!("daemon source-input cold/warm µs: Native64={native_samples:?} Wasm32={wasm_samples:?}");

        let file = root.join("main.w");
        // Same grammar, edited byte at the exact require condition.
        let edited = INPUT.replace("require(false", "require(true ");
        fs::write(&file, &edited).unwrap();
        assert!(!native.is_current_live(&mut client, deadline(), 1 << 20).unwrap());
        assert!(matches!(prepare(&mut client, &store, "dev", Some(&native)), SelectedMainFlowPreparation::WorkspaceChanged));
        select(&mut client, &store).await;
        let different = ready(prepare(&mut client, &store, "dev", Some(&native)));
        assert!(!different.reused_input);
        assert_ne!(different.input().identity(), native.input().identity());
        assert_eq!(different.input().text(), edited);

        fs::write(&file, INPUT).unwrap();
        select(&mut client, &store).await;
        let restored = ready(prepare(&mut client, &store, "dev", Some(&native)));
        assert!(restored.reused_input);
        assert_eq!(restored.input().identity(), native.input().identity());
        assert_eq!(restored.source().selection().root, native.source().selection().root);
        assert_ne!(restored.source().generation(), native.source().generation());
        assert!(!native.is_current_live(&mut client, deadline(), 1 << 20).unwrap());
        assert!(restored.is_current_live(&mut client, deadline(), 1 << 20).unwrap());

        // Both are ordinary source inputs; neither can become a proof-free
        // root-only handoff through silently dropping a grammar role.
        fs::write(&file, "%%start\ni32 value = comptime fixture { i32 seed = 7; seed ^ 9 };\n%%end\n").unwrap();
        select(&mut client, &store).await;
        assert!(matches!(prepare(&mut client, &store, "dev", Some(&native)),
            SelectedMainFlowPreparation::Refused(MainFlowInputRefusal::OutsideCommonScope)));
        fs::write(root.join("library.w"), "%%start\ni32 value = 7;\n%%end\n").unwrap();
        fs::write(&file, "%%start\nlib = namespace app \"library.w\";\n%%end\n").unwrap();
        select(&mut client, &store).await;
        assert!(matches!(prepare(&mut client, &store, "dev", None),
            SelectedMainFlowPreparation::Refused(MainFlowInputRefusal::OutsideCommonScope)));
        assert!(!root.join(".wosy/artifacts").exists());
        store.close().await.unwrap();
    });
}
