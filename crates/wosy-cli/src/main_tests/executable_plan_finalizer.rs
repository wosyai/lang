use super::*;
use std::{num::NonZeroUsize, sync::Arc, time::Duration};
use wosy_cli::{
    source_recovery::ClientSourceLimits,
    source_state::{AcknowledgedOrdinaryLlvm, AuthorizedOrdinaryBuilderPeer, ReplaySource, SqliteSourceState},
};
use wosy_daemon::{BaseCstAdmissionLimits, BuildSessionEnd, SourceStorageWorker, SourceStore};

#[test]
fn only_a_valid_local_fixture_plan_reaches_the_single_finalizer() {
    let scratch = snapshot_test_path("executable-plan-finalizer").with_extension("");
    fs::create_dir(&scratch).unwrap();
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(scratch);
    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("main.w"), "%%start\ni32 answer = 42;\n%%end\n").unwrap();
    // Sentinel builder checks that the sole production finalizer passes the
    // literal arguments and exactly four materialized inputs, once.
    fs::write(root.join("builder.sh"), r#"
set -eu
test "$#" = 5
test "$1" = selected
test -s "$2"
test -s "$4"
test -s "$5"
printf 'invoked\n' >> builder.invocations
printf 'fixture artifact' > "$3/main"
"#).unwrap();
    fs::write(root.join("wosy.toml"), r#"
[package]
name = "app"
default_target = "app"
[builders.fixture]
command = ["/bin/sh", "builder.sh"]
args = ["selected"]
toolchain_identity = "sentinel fixture"
[runners.fixture]
command = ["/bin/false"]
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "Main"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "fixture"
run_runner = "fixture"
[targets.app.profiles.dev]
main_artifact = "main"
"#).unwrap();
    let output = root.join(".wosy/artifacts/app/dev/main");
    let correct = AuthorizedOrdinaryBuilderPeer::from_open_executable(fs::File::open("/proc/self/exe").unwrap()).unwrap();
    let wrong = AuthorizedOrdinaryBuilderPeer::from_open_executable(fs::File::open(root.join("builder.sh")).unwrap()).unwrap();

    let limits = BaseCstAdmissionLimits {
        records: 1_000, record_bytes: 1 << 20, graph_edges: 1_000, expanded_record_bytes: 1 << 20,
        cst_occurrences: 1_000, cst_depth: 128, authored_bytes: 1 << 20,
    };
    let ttl = Duration::from_secs(30);
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let store = Arc::new(SourceStore::open(&scratch.0.join("store"), 8 << 20).unwrap());
        let one = NonZeroUsize::new(1).unwrap();
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let mut client = SqliteSourceState::open(&root, ClientSourceLimits { nodes: 1_000, encoded_bytes: 1 << 20 }).unwrap();
        let base = client.stage("app").unwrap();
        let pending = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
        store.publish(base.root, &pending.source.delta).await.unwrap();
        assert!(client.acknowledge("app", base.pending.unwrap()).unwrap());
        let selected = client.acknowledged_build_source("app").unwrap().unwrap();

        for (expected, valid) in [(&wrong, false), (&correct, true)] {
            // Both stream ends and the actual daemon worker belong to this
            // fixture. The digest check is not transport authentication.
            let (ours, theirs) = tokio::io::duplex(512);
            let mut ours = Some(ours);
            let client_plan = client.live_executable_builder_inputs("app", "dev", ttl, 1 << 20, 1 << 20, expected,
                output.clone(), |capture| {
                    assert_eq!(capture, selected);
                    std::future::ready(Ok(ours.take().expect("opens once")))
                });
            let (plan, end) = tokio::join!(client_plan,
                worker.serve_known_executable_builder_input_session(theirs, selected.clone(), limits, 1 << 20, ttl));
            assert_eq!(end.unwrap(), BuildSessionEnd::Disconnected);
            assert!(!output.exists() && !root.join("builder.invocations").exists());
            if !valid {
                assert!(plan.err().unwrap().to_string().contains("different compiler image"));
                continue;
            }
            let AcknowledgedOrdinaryLlvm::Partition(plan) = plan.unwrap() else { panic!("expected live fixture plan"); };
            assert_eq!(plan.source().selection(), &selected);
            assert_eq!(plan.project(), root);
            assert_eq!(plan.paths().output, output);
            let inputs = plan.inputs();
            assert_eq!(inputs.manifest.executable, output.join("main"));
            assert_eq!(inputs.manifest.identity.artifact_identity, "app:dev:Main");
            assert_eq!(inputs.builder.command, ["/bin/sh", "builder.sh"]);
            assert_eq!(inputs.builder.args, ["selected"]);
            let digest = plan.executable_partition_digest().expect("V3 comparison digest");
            assert_eq!(digest.hex(), inputs.manifest.identity.llvm_input_identity);
            assert_eq!(blake3::hash(plan.partition().as_bytes()).to_hex().as_str(), digest.hex());
            let paths = BuilderInputPaths::new(output.clone());
            fs::create_dir_all(&paths.output).unwrap();
            fs::write(&paths.partition, plan.partition()).unwrap();
            fs::write(&paths.runtime, plan.runtime()).unwrap();
            finalize_ordinary_build(&root, paths, PendingBuilderInputs {
                manifest: inputs.manifest.clone(), builder: inputs.builder.clone(),
            }).unwrap();
            assert_eq!(fs::read(root.join("builder.invocations")).unwrap(), b"invoked\n");
            assert_eq!(fs::read(output.join("main")).unwrap(), b"fixture artifact");
            assert_eq!(load_artifact_manifest(&output.join("artifact-manifest.json")).unwrap(), inputs.manifest);
            assert!(!output.join("artifact-manifest.pending.json").exists());
        }
        drop(worker);
        Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
    });
}
