#![cfg(all(target_os = "linux", feature = "native-diagnostics"))]

use std::{
    fs::{self, DirBuilder, File},
    io::{BufRead, BufReader, Write},
    num::NonZeroUsize,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::{
    AcknowledgedOrdinaryLlvm, ReplaySource, SqliteSourceState,
}};
use wosy_daemon::{BaseCstAdmissionLimits, BuildSessionEnd, SourceStorageWorker, SourceStore};
use wosy_protocol::{BuildSessionRefusal, BuildSourceSelection};

const TIME: Duration = Duration::from_secs(10);
const BYTES: usize = 1 << 20;
const LIMITS: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
    records: 4_000, record_bytes: BYTES, graph_edges: 4_000, expanded_record_bytes: BYTES,
    cst_occurrences: 4_000, cst_depth: 128, authored_bytes: BYTES,
};
const CLIENT: ClientSourceLimits = ClientSourceLimits { nodes: 4_000, encoded_bytes: BYTES };
const MANIFEST: &str = r#"
[package]
name = "app"
default_target = "app"
[comments]
categories = ["WHY"]
[builders.never]
command = ["fixture-must-not-build"]
toolchain_identity = "comparison-only"
[runners.never]
command = ["fixture-must-not-run"]
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "Main"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "never"
run_runner = "never"
[targets.app.artifacts.main.profiles.wasm]
backend = "llvm"
output = "wasm-wasip1"
builder = "never"
run_runner = "never"
[targets.app.artifacts.main.profiles.wasm.exports]
fixture_answer = "answer"
[targets.app.profiles.dev]
main_artifact = "main"
[targets.app.profiles.wasm]
main_artifact = "main"
"#;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap()
}
fn worker(store: Arc<SourceStore>) -> SourceStorageWorker {
    let one = NonZeroUsize::new(1).unwrap();
    SourceStorageWorker::new(store, tokio::runtime::Handle::current(), one, one, one).unwrap()
}

#[test]
#[ignore = "explicit fixture child entry"]
fn native_v3_child() {
    let args = |name| std::env::var(name).unwrap();
    let selection: BuildSourceSelection = serde_json::from_str(&args("WOSY_V3_SELECTION")).unwrap();
    let path = args("WOSY_V3_ENDPOINT");
    let profile = args("WOSY_V3_PROFILE");
    let activator = args("WOSY_V3_PARENT_PID").parse().unwrap();
    runtime().block_on(async {
        let store = Arc::new(SourceStore::open(Path::new(&args("WOSY_V3_STORE")), 8 << 20).unwrap());
        let worker = worker(Arc::clone(&store));
        let outlet = worker.listen_native_known_executable_inputs(
            Path::new(&path), selection, profile, LIMITS, BYTES, activator,
        ).unwrap();
        println!("V3_CHILD_READY");
        std::io::stdout().flush().unwrap();
        assert_eq!(outlet.serve(TIME).await.unwrap(), BuildSessionEnd::Disconnected);
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
impl OwnedChild {
    fn expect_exit(&mut self) {
        let deadline = Instant::now() + TIME;
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "native V3 child did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn launch(exe: &Path, store: &Path, endpoint: &Path, selected: &BuildSourceSelection, profile: &str) -> OwnedChild {
    let mut child = OwnedChild(Command::new(exe).args(["--exact", "native_v3_child", "--ignored", "--nocapture"])
        .env("WOSY_V3_STORE", store)
        .env("WOSY_V3_ENDPOINT", endpoint)
        .env("WOSY_V3_SELECTION", serde_json::to_string(selected).unwrap())
        .env("WOSY_V3_PROFILE", profile)
        .env("WOSY_V3_PARENT_PID", std::process::id().to_string())
        .stdout(Stdio::piped()).spawn().unwrap());
    let mut pipe = BufReader::new(child.0.stdout.take().unwrap());
    let (ready, wait_ready) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        while pipe.read_line(&mut line).ok().is_some_and(|length| length != 0) {
            if line.trim() == "V3_CHILD_READY" {
                let _ = ready.send(());
                break;
            }
            line.clear();
        }
        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
    });
    wait_ready.recv_timeout(TIME).expect("missing native V3 child readiness");
    child
}

#[test]
#[ignore = "explicit protected Linux AF_UNIX project fixture"]
fn native_v3_projects_only_the_activated_verified_profile() {
    let parent = PathBuf::from(std::env::var_os("WOSY_NATIVE_GATE_PARENT").unwrap());
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = parent.join(format!("w6v3-{}-{nonce:x}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&path).unwrap();
    let scratch = Scratch(path);
    let project = scratch.0.join("src");
    let store_path = scratch.0.join("store");
    let output = project.join(".wosy/artifacts");
    let endpoint = scratch.0.join("s");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("wosy.toml"), MANIFEST).unwrap();
    fs::write(project.join("main.w"), "%%start\n# WHY: native test\ni32() answer = fn { 42 };\n%%end\n").unwrap();
    let executable = std::env::current_exe().unwrap();
    let image = File::open(&executable).unwrap();
    let mut client = SqliteSourceState::open(&project, CLIENT).unwrap();

    runtime().block_on(async {
        let store = SourceStore::open(&store_path, 8 << 20).unwrap();
        let staged = client.stage("app").unwrap();
        let pending = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(staged.root, &pending.source.delta).await.unwrap();
        assert!(client.acknowledge("app", staged.pending.unwrap()).unwrap());
        store.close().await.unwrap();
        let selected = client.acknowledged_build_source("app").unwrap().unwrap();

        for (pinned, requested) in [("dev", "dev"), ("wasm", "wasm"), ("dev", "wasm")] {
            let mut child = launch(&executable, &store_path, &endpoint, &selected, pinned);
            let mut wrong_source = selected.clone();
            wrong_source.root = wosy_protocol::NodeId([0; 32]);
            let refusal = client.native_owned_executable_builder_inputs(&endpoint, &mut child.0, &image,
                &wrong_source, requested, TIME, BYTES, BYTES, output.clone()).await.err().unwrap();
            assert_eq!(refusal.to_string(), "native V3 source differs from the activated selection");
            assert!(!output.exists());
            // The mismatched activation did not consume the child's one connection.
            let plan = client.native_owned_executable_builder_inputs(&endpoint, &mut child.0, &image,
                &selected, requested, TIME, BYTES, BYTES, output.clone()).await.unwrap();
            if pinned == requested {
                let AcknowledgedOrdinaryLlvm::Partition(plan) = plan else { panic!("expected native V3 plan"); };
                assert_eq!(plan.source().selection(), &selected);
                assert_eq!(plan.inputs().manifest.identity.artifact_identity, format!("app:{requested}:Main"));
                let digest = plan.executable_partition_digest().unwrap().hex();
                assert_eq!(blake3::hash(plan.partition().as_bytes()).to_hex().as_str(), digest);
                assert_eq!(plan.inputs().manifest.identity.llvm_input_identity, digest);
                assert_eq!(plan.partition().contains("@fixture_answer"), requested == "wasm");
            } else {
                assert!(matches!(plan, AcknowledgedOrdinaryLlvm::Refused(BuildSessionRefusal::OrdinaryLlvmUnavailable)));
            }
            child.expect_exit();
            assert!(!endpoint.exists() && !output.exists());
        }
    });
}
