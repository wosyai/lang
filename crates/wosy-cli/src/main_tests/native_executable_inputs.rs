use super::{
    BuilderInputPaths, PendingBuilderInputs, ProjectConfiguration, finalize_ordinary_build,
    load_artifact_manifest, observe_source_revision, pending_builder_inputs, project_configuration_identity,
};

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
    AcknowledgedOrdinaryBuilderInputs, AcknowledgedOrdinaryLlvm, ReplaySource, SqliteSourceState,
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
command = ["/bin/sh", "fixture-builder.sh"]
args = ["native-v3-fixture"]
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
    let selection = BuildSourceSelection {
        root: wosy_protocol::NodeId::parse_hex(&args("WOSY_V3_ROOT")).unwrap(),
        project: args("WOSY_V3_PROJECT"),
        target: args("WOSY_V3_TARGET"),
    };
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
    let mut child = OwnedChild(Command::new(exe).args(["--exact", "native_executable_inputs::native_v3_child", "--ignored", "--nocapture"])
        .env("WOSY_V3_STORE", store)
        .env("WOSY_V3_ENDPOINT", endpoint)
        .env("WOSY_V3_ROOT", selected.root.hex())
        .env("WOSY_V3_PROJECT", &selected.project)
        .env("WOSY_V3_TARGET", &selected.target)
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

// A private disposable owner, not a new production activation or builder
// policy. Only this fixture owns the approved builder and output directory.
fn finalize_fixture_native_plan(
    project: &Path, plan: &AcknowledgedOrdinaryBuilderInputs, selected: &BuildSourceSelection,
    output: &Path,
) {
    assert_eq!(plan.source().selection(), selected);
    assert_eq!(plan.project(), project);
    assert_eq!(plan.paths().output, output);
    let (configuration, document) = ProjectConfiguration::load_with_manifest(project).unwrap();
    let (_, artifact, profile, target_profile) = configuration.artifact_profile("app", "dev").unwrap();
    assert_eq!(output, project.join(".wosy/artifacts/app/dev").join(&target_profile.main_artifact));
    let observed = &plan.inputs().manifest.source_observations;
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].source.path.as_path(), Path::new("main.w"));
    assert_eq!(observe_source_revision(&project.join("main.w")).unwrap(), observed[0].content_revision);
    let runtime = wosy_compiler::core_runtime(&profile.backend, &profile.output).unwrap();
    assert_eq!(plan.runtime(), runtime);
    let digest = blake3::hash(plan.partition().as_bytes()).to_hex().to_string();
    assert_eq!(plan.executable_partition_digest().unwrap().hex(), digest);
    let expected = pending_builder_inputs(&configuration,
        project_configuration_identity(document.raw().as_bytes(), &configuration).unwrap(),
        "app", "dev", artifact, profile, observed.clone(), digest, output).unwrap();
    assert_eq!(expected.manifest, plan.inputs().manifest);
    assert_eq!(expected.builder, plan.inputs().builder);
    assert_eq!(expected.builder.command, ["/bin/sh", "fixture-builder.sh"]);
    assert_eq!(expected.builder.args, ["native-v3-fixture"]);

    let paths = BuilderInputPaths::new(output.to_owned());
    assert_eq!(plan.paths().arguments(), paths.arguments());
    assert!(!output.exists() && !project.join("builder.invocations").exists());
    fs::create_dir_all(output).unwrap();
    fs::write(&paths.partition, plan.partition()).unwrap();
    fs::write(&paths.runtime, runtime).unwrap();
    finalize_ordinary_build(project, paths, PendingBuilderInputs {
        manifest: expected.manifest.clone(), builder: expected.builder,
    }).unwrap();
    assert_eq!(fs::read(project.join("builder.invocations")).unwrap(), b"invoked\n");
    assert_eq!(load_artifact_manifest(&output.join("artifact-manifest.json")).unwrap(), expected.manifest);
    assert_eq!(fs::read(output.join("main")).unwrap(), b"native v3 fixture");
    assert!(!output.join("artifact-manifest.pending.json").exists());
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
    let endpoint = scratch.0.join("s");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("wosy.toml"), MANIFEST).unwrap();
    fs::write(project.join("fixture-builder.sh"), r#"
set -eu
test "$#" = 5
test "$1" = native-v3-fixture
test -s "$2"
test -s "$4"
test -s "$5"
printf 'invoked\n' >> builder.invocations
printf 'native v3 fixture' > "$3/main"
"#).unwrap();
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

        for (pinned, requested) in [("dev", "wasm"), ("wasm", "wasm"), ("dev", "dev")] {
            let output = project.join(".wosy/artifacts/app").join(requested).join("main");
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
            child.expect_exit();
            assert!(!endpoint.exists() && !output.exists());
            if pinned == requested {
                let AcknowledgedOrdinaryLlvm::Partition(plan) = plan else { panic!("expected native V3 plan"); };
                assert_eq!(plan.source().selection(), &selected);
                assert_eq!(plan.inputs().manifest.identity.artifact_identity, format!("app:{requested}:Main"));
                let digest = plan.executable_partition_digest().unwrap().hex();
                assert_eq!(blake3::hash(plan.partition().as_bytes()).to_hex().as_str(), digest);
                assert_eq!(plan.inputs().manifest.identity.llvm_input_identity, digest);
                assert_eq!(plan.partition().contains("@fixture_answer"), requested == "wasm");
                if requested == "dev" {
                    finalize_fixture_native_plan(&project, &plan, &selected, &output);
                }
            } else {
                assert!(matches!(plan, AcknowledgedOrdinaryLlvm::Refused(BuildSessionRefusal::OrdinaryLlvmUnavailable)));
            }
        }
    });
}
