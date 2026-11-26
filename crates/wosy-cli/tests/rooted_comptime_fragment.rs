#![cfg(target_os = "linux")]

use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use wosy_cli::{
    comptime_fragment::plan_closed_comptime_fragment,
    source_recovery::ClientSourceLimits,
    source_state::{ReplaySource, SqliteSourceState},
};
use wosy_compiler::{ScalarTargetLayout, ScalarType};
use wosy_daemon::{
    BaseCstAdmissionLimits, RootedClosedComptimeFragment, RootedClosedComptimeProjection,
    RootedClosedComptimeRefusal, RootedClosedComptimeRequest, SourceStorageWorker, SourceStore,
};
use wosy_project::PackageRelativePath;
use wosy_protocol::{BuildSourceSelection, FileKey, NodeId};

const SOURCE: &str = r#"%%start
# WHY: saída do pacote âncora ☕
i32 answer = comptime readback { 42 };
i32 with_default = comptime default_param { core.comp_param("limit", 41) };
i32 not_defaulted = comptime required_param { core.comp_param("limit") };
%%end
"#;
const MANIFEST: &str = r#"
[package]
name = "app"
default_target = "app"

[comments]
categories = ["WHY"]

[builders.never]
command = ["must-not-run-any-builder"]
toolchain_identity = "fixture"

[runners.never]
command = ["must-not-run-any-runner"]

[targets.app]
root = "src/main.w"

[targets.app.artifacts.main]
artifact = "Main"

[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "never"
run_runner = "never"

[targets.app.profiles.dev]
main_artifact = "main"

[targets.app.profiles.dev.params]
limit = 43
"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("wosy-rooted-comptime-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        fs::create_dir_all(fixture.project().join("src")).unwrap();
        fs::write(fixture.project().join("wosy.toml"), MANIFEST).unwrap();
        fs::write(fixture.project().join("src/main.w"), SOURCE).unwrap();
        fs::write(fixture.project().join("src/unreachable.w"), SOURCE).unwrap();
        fixture
    }

    fn project(&self) -> PathBuf { self.0.join("project") }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn client(project: &Path) -> SqliteSourceState {
    SqliteSourceState::open(project,
        ClientSourceLimits { nodes: 4096, encoded_bytes: 1 << 20 }).unwrap()
}

async fn publish(client: &mut SqliteSourceState, store: &SourceStore) -> BuildSourceSelection {
    let stage = client.stage("app").unwrap();
    let attempt = stage.pending.unwrap();
    let replay = client.replay("app", ReplaySource::Pending).unwrap().unwrap();
    store.publish(replay.source.root, &replay.source.delta).await.unwrap();
    assert!(client.acknowledge("app", attempt).unwrap());
    let selected = client.acknowledged_build_source("app").unwrap().unwrap();
    assert_eq!(selected.root, stage.root);
    selected
}

fn worker(store: &Arc<SourceStore>) -> SourceStorageWorker {
    SourceStorageWorker::new(Arc::clone(store), tokio::runtime::Handle::current(),
        NonZeroUsize::new(1).unwrap(), NonZeroUsize::new(2).unwrap(), NonZeroUsize::new(8).unwrap()).unwrap()
}

fn request(id: &str, evaluator_layout: ScalarTargetLayout) -> RootedClosedComptimeRequest {
    RootedClosedComptimeRequest {
        profile: "dev".into(),
        file: FileKey { package: "app".into(), path: PackageRelativePath::new("src/main.w".into()).unwrap() },
        id: id.into(), evaluator_layout,
        source_limits: BaseCstAdmissionLimits {
            records: 4096, record_bytes: 1 << 20, graph_edges: 4096,
            expanded_record_bytes: 1 << 20, cst_occurrences: 4096, cst_depth: 128,
            authored_bytes: 1 << 16,
        },
        output_bytes: 1 << 16,
    }
}

fn emitted(projection: RootedClosedComptimeProjection) -> RootedClosedComptimeFragment {
    match projection {
        RootedClosedComptimeProjection::Emitted(fragment) => fragment,
        RootedClosedComptimeProjection::Refused(reason) => panic!("unexpected refusal: {reason:?}"),
    }
}

fn assert_ordinary_parity(
    project: &Path, root: NodeId, source: &str, layout: ScalarTargetLayout, actual: &RootedClosedComptimeFragment,
) {
    let expected = plan_closed_comptime_fragment(project, "app", "dev", "readback", layout).unwrap();
    let mut bytes = Vec::new();
    expected.llvm().write_text(&mut bytes).unwrap();
    assert_eq!(actual.bytes, bytes);
    assert_eq!(actual.digest, NodeId(*blake3::hash(&bytes).as_bytes()));
    assert_eq!(actual.source_root, root);
    assert_eq!(actual.site, *expected.site());
    assert_eq!(actual.target, "app");
    assert_eq!(actual.profile, "dev");
    assert_eq!(actual.id, "readback");
    assert_eq!(actual.evaluator_layout, layout);
    assert_eq!(actual.result_type, ScalarType::I32);
    let range = actual.site.range;
    assert_eq!(&source[range.start as usize..range.end as usize], if source == SOURCE {
        "comptime readback { 42 }"
    } else {
        "comptime readback { 43 }"
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leased_compiler_fragment_matches_authored_native_and_wasi_without_live_reopen() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let mut client = client(&project);
    let store = Arc::new(SourceStore::open(&fixture.0.join("daemon"), 1 << 20).unwrap());
    let source = publish(&mut client, &store).await;
    let worker = worker(&store);
    let old = worker.open_build_diagnostics(source.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    let mut baseline = Vec::new();
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let actual = emitted(old.project_closed_comptime(request("readback", layout)).await.unwrap());
        assert_ordinary_parity(&project, source.root, SOURCE, layout, &actual);
        baseline.push((layout, actual));
    }

    let changed_text = SOURCE.replace("{ 42 }", "{ 43 }");
    fs::write(project.join("src/main.w"), &changed_text).unwrap();
    let changed = publish(&mut client, &store).await;
    assert_ne!(source.root, changed.root);
    let current = worker.open_build_diagnostics(changed.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    for (layout, original) in baseline {
        let retained = emitted(old.project_closed_comptime(request("readback", layout)).await.unwrap());
        assert_eq!((retained.source_root, retained.authored_cst, retained.digest),
            (original.source_root, original.authored_cst, original.digest));
        assert_eq!(retained.site, original.site);
        assert_eq!(retained.bytes, original.bytes);
        let actual = emitted(current.project_closed_comptime(request("readback", layout)).await.unwrap());
        assert_ordinary_parity(&project, changed.root, &changed_text, layout, &actual);
        assert_ne!(actual.authored_cst, original.authored_cst);
        assert_ne!(actual.digest, original.digest);
        assert_eq!(actual.compiler_image, original.compiler_image);
        assert_ne!(actual.compiler_image, NodeId([0; 32]));
    }
    assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, changed.root);
    drop((old, current));
    drop((worker, client));
    Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_query_refuses_nonrooted_scope_parameters_and_tight_budgets() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let mut client = client(&project);
    let store = Arc::new(SourceStore::open(&fixture.0.join("daemon"), 1 << 20).unwrap());
    let source = publish(&mut client, &store).await;
    let worker = worker(&store);
    let session = worker.open_build_diagnostics(source.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    let layout = ScalarTargetLayout::WASM32;

    let mut unknown = request("readback", layout);
    unknown.file.path = PackageRelativePath::new("src/unreachable.w".into()).unwrap();
    assert!(matches!(session.project_closed_comptime(unknown).await.unwrap(),
        RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::FileNotRooted)));

    let mut other_profile = request("readback", layout);
    other_profile.profile = "other".to_owned();
    assert!(matches!(session.project_closed_comptime(other_profile).await.unwrap(),
        RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::ProfileNotSelected)));

    for (id, authored_call) in [
        ("default_param", r#"core.comp_param("limit", 41)"#),
        ("required_param", r#"core.comp_param("limit")"#),
    ] {
        // The actual CLI has authority for the authored consumer profile.
        let cli = plan_closed_comptime_fragment(&project, "app", "dev", id, layout).unwrap();
        assert_eq!(cli.parameter_uses().len(), 1);
        let refused = session.project_closed_comptime(request(id, layout)).await.unwrap();
        let RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::ParameterAuthorityUnavailable { site, name })
            = refused else { panic!("unexpected parameter answer: {refused:?}") };
        assert_eq!(name, "limit");
        assert_eq!(&SOURCE[site.range.start as usize..site.range.end as usize], authored_call);
        assert_eq!(site.source, cli.site().source);
    }

    let mut tight_output = request("readback", layout);
    tight_output.output_bytes = 1;
    assert!(session.project_closed_comptime(tight_output).await.unwrap_err().to_string().contains("output byte budget"));
    let mut tight_index = request("readback", layout);
    tight_index.source_limits.records = 1;
    assert!(session.project_closed_comptime(tight_index).await.unwrap_err().to_string().contains("record budget"));
    assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, source.root);
    drop(session);
    drop((worker, client));
    Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
}
