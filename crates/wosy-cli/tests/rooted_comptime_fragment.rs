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
use wosy_compiler::{ClosedComptimeRejection, DirectComptimeArgument, ScalarTargetLayout, ScalarType};
use wosy_daemon::{
    BaseCstAdmissionLimits, RootedClosedComptimeFragment, RootedClosedComptimeProjection,
    RootedClosedComptimeRefusal, RootedClosedComptimeRequest,
    RootedDirectFunctionComptimeFragment, RootedDirectFunctionComptimeProjection,
    RootedDirectStartupComptimePlan, RootedDirectStartupComptimeProjection, SourceStorageWorker, SourceStore,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_entry_initializer_is_selected_only_in_exact_leased_startup_order() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let mut client = client(&project);
    let store = Arc::new(SourceStore::open(&fixture.0.join("daemon"), 1 << 20).unwrap());
    let initial = publish(&mut client, &store).await;
    let worker = worker(&store);
    let original = worker.open_build_diagnostics(initial.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let exact = emitted(original.project_first_entry_comptime(request("readback", layout)).await.unwrap());
        assert_ordinary_parity(&project, initial.root, SOURCE, layout, &exact);
    }

    fs::write(project.join("src/helper.w"), "%%start\ni32 another = comptime helper { 3 };\n%%end\n").unwrap();
    for (previous, has_import) in [
        ("i32 preceding = 7;", false),
        (r#"helper = namespace app "src/helper.w";"#, true),
    ] {
        let text = SOURCE.replacen("i32 answer = ", &format!("{previous}\ni32 answer = "), 1);
        fs::write(project.join("src/main.w"), &text).unwrap();
        let changed = publish(&mut client, &store).await;
        assert_ne!(initial.root, changed.root);
        let current = worker.open_build_diagnostics(changed.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let refused = current.project_first_entry_comptime(request("readback", layout)).await.unwrap();
            let RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::Compiler(
                ClosedComptimeRejection::PrecededByAuthoredItem { site, first },
            )) = refused else { panic!("unexpected startup answer: {refused:?}") };
            assert_eq!(site.source, first.source);
            assert_eq!(text[first.range.start as usize..first.range.end as usize].trim(), previous);
            assert_eq!(&text[site.range.start as usize..site.range.end as usize], "comptime readback { 42 }");

            let candidate = emitted(current.project_closed_comptime(request("readback", layout)).await.unwrap());
            assert_eq!(candidate.source_root, changed.root);
            assert_eq!(candidate.site, site);
            let retained = emitted(original.project_first_entry_comptime(request("readback", layout)).await.unwrap());
            assert_eq!(retained.source_root, initial.root);
            assert_eq!(retained.bytes, candidate.bytes);

            if has_import {
                let child = || {
                    let mut scoped = request("helper", layout);
                    scoped.file.path = PackageRelativePath::new("src/helper.w".into()).unwrap();
                    scoped
                };
                assert!(matches!(current.project_first_entry_comptime(child()).await.unwrap(),
                    RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::NotTargetEntry)));
                assert_eq!(emitted(current.project_closed_comptime(child()).await.unwrap()).id, "helper");
                assert!(matches!(original.project_first_entry_comptime(child()).await.unwrap(),
                    RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::FileNotRooted)));
            }
        }
        assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, changed.root);
    }
    drop(original);
    drop((worker, client));
    Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
}

const DIRECT_SOURCE: &str = r#"%%start
# WHY: chamada concreta à órbita ☕
i32(i32) solve = fn(input) { comptime value { input } };
i32 answer = solve(7);
%%end
"#;

fn emitted_direct(projection: RootedDirectFunctionComptimeProjection) -> RootedDirectFunctionComptimeFragment {
    match projection {
        RootedDirectFunctionComptimeProjection::Emitted(fragment) => fragment,
        RootedDirectFunctionComptimeProjection::Refused(reason) => panic!("unexpected direct refusal: {reason:?}"),
    }
}

fn assert_direct_evidence(
    fragment: &RootedDirectFunctionComptimeFragment, root: NodeId, layout: ScalarTargetLayout,
    text: &str, expected: i32,
) {
    let rooted = fragment.rooted();
    let call = fragment.call();
    assert_eq!(rooted.source_root, root);
    assert_eq!(rooted.target, "app");
    assert_eq!(rooted.profile, "dev");
    assert_eq!(rooted.id, "value");
    assert_eq!(rooted.evaluator_layout, layout);
    assert_eq!(rooted.result_type, ScalarType::I32);
    assert_eq!(rooted.site, *call.site());
    assert_eq!(call.value(), DirectComptimeArgument::I32(expected));
    assert_eq!(call.result(), &ScalarType::I32);
    let spelling = |span: &wosy_syntax::SourceSpan| {
        assert_eq!(span.source, rooted.site.source);
        &text[span.range.start as usize..span.range.end as usize]
    };
    assert_eq!(spelling(call.function()), "solve");
    assert_eq!(spelling(call.call()), format!("solve({expected})"));
    assert_eq!(spelling(call.argument()), expected.to_string());
    assert_eq!(spelling(call.formal()), "input");
    assert_eq!(spelling(call.read()), "input");
    assert_ne!(call.formal(), call.read());
    assert_eq!(rooted.digest, NodeId(*blake3::hash(&rooted.bytes).as_bytes()));
    let llvm = std::str::from_utf8(&rooted.bytes).unwrap();
    assert!(llvm.lines().any(|line| line.trim() == format!("ret i32 {expected}")));
    assert!(!llvm.contains("@solve("));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_evaluator_binds_the_call_to_the_exact_leased_singleton_root() {
    let fixture = Fixture::new();
    let project = fixture.project();
    fs::write(project.join("src/main.w"), DIRECT_SOURCE).unwrap();
    let mut client = client(&project);
    let store = Arc::new(SourceStore::open(&fixture.0.join("daemon"), 1 << 20).unwrap());
    let initial = publish(&mut client, &store).await;
    let worker = worker(&store);
    let old = worker.open_build_diagnostics(initial.clone(), Duration::from_secs(20)).await.unwrap().unwrap();

    let changed_text = DIRECT_SOURCE.replace("solve(7)", "solve(9)");
    fs::write(project.join("src/main.w"), &changed_text).unwrap();
    let changed = publish(&mut client, &store).await;
    let current = worker.open_build_diagnostics(changed.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    assert_ne!(initial.root, changed.root);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        // Never take the old observation before editing live source. The
        // query must reconstruct the actual old immutable call in the lease.
        let retained = emitted_direct(old.project_direct_function_comptime(request("value", layout)).await.unwrap());
        assert_direct_evidence(&retained, initial.root, layout, DIRECT_SOURCE, 7);
        let next = emitted_direct(current.project_direct_function_comptime(request("value", layout)).await.unwrap());
        assert_direct_evidence(&next, changed.root, layout, &changed_text, 9);
        assert_eq!(retained.rooted().compiler_image, next.rooted().compiler_image);
        assert_ne!(retained.rooted().authored_cst, next.rooted().authored_cst);
        assert_ne!(retained.rooted().digest, next.rooted().digest);
    }
    let mut capped = request("value", ScalarTargetLayout::WASM32);
    capped.output_bytes = 1;
    assert!(current.project_direct_function_comptime(capped).await.unwrap_err().to_string().contains("output byte budget"));
    drop(current);

    fs::write(project.join("src/helper.w"), "%%start\ni32 helper_value = 1;\n%%end\n").unwrap();
    let multi_text = changed_text.replace("i32(i32) solve", "helper = namespace app \"src/helper.w\";\ni32(i32) solve");
    fs::write(project.join("src/main.w"), multi_text).unwrap();
    let multi = publish(&mut client, &store).await;
    let session = worker.open_build_diagnostics(multi.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    assert!(matches!(session.project_direct_function_comptime(request("value", ScalarTargetLayout::WASM32)).await.unwrap(),
        RootedDirectFunctionComptimeProjection::Refused(RootedClosedComptimeRefusal::DirectCallRequiresSingleModule)));
    let mut helper = request("value", ScalarTargetLayout::NATIVE64);
    helper.file.path = PackageRelativePath::new("src/helper.w".into()).unwrap();
    assert!(matches!(session.project_direct_function_comptime(helper).await.unwrap(),
        RootedDirectFunctionComptimeProjection::Refused(RootedClosedComptimeRefusal::NotTargetEntry)));
    assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, multi.root);
    drop((old, session));
    drop((worker, client));
    Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
}

fn emitted_startup(projection: RootedDirectStartupComptimeProjection) -> RootedDirectStartupComptimePlan {
    match projection {
        RootedDirectStartupComptimeProjection::Emitted(plan) => plan,
        RootedDirectStartupComptimeProjection::Refused(reason) => panic!("unexpected startup plan refusal: {reason:?}"),
    }
}

fn assert_startup_evidence(
    plan: &RootedDirectStartupComptimePlan, root: NodeId, layout: ScalarTargetLayout, source: &str, middle: i32,
) -> usize {
    assert_eq!(plan.source_root(), root);
    assert_eq!(plan.target(), "app");
    assert_eq!(plan.profile(), "dev");
    assert_eq!(plan.id(), "value");
    assert_eq!(plan.evaluator_layout(), layout);
    let [first, second, third] = plan.calls() else { panic!("three leased startup calls"); };
    let [seven, other] = plan.evaluators() else { panic!("two leased evaluator tuples"); };
    assert_eq!([first.evaluator_index(), second.evaluator_index(), third.evaluator_index()], [0, 1, 0]);
    for (call, expected) in [(first, 7), (second, middle), (third, 7)] {
        let facts = call.facts();
        assert_eq!(facts.value(), DirectComptimeArgument::I32(expected));
        assert_eq!(facts.function(), first.facts().function());
        assert_eq!(facts.formal(), first.facts().formal());
        assert_eq!(facts.site(), first.facts().site());
        assert_eq!(&source[facts.argument().range.start as usize..facts.argument().range.end as usize], expected.to_string());
    }
    assert!(first.facts().call().range.end < second.facts().call().range.start);
    assert!(second.facts().call().range.end < third.facts().call().range.start);
    for (evaluator, expected) in [(seven, 7), (other, middle)] {
        assert_eq!(evaluator.digest(), NodeId(*blake3::hash(evaluator.bytes()).as_bytes()));
        let llvm = std::str::from_utf8(evaluator.bytes()).unwrap();
        assert!(llvm.lines().any(|line| line.trim() == format!("ret i32 {expected}")));
        assert!(!llvm.contains("@solve("));
    }
    assert_ne!(seven.digest(), other.digest());
    seven.bytes().len() + other.bytes().len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_plan_retains_ordered_old_root_and_checks_total_egress() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let original_text = DIRECT_SOURCE.replace("i32 answer = solve(7);",
        "i32 first = solve(7);\ni32 second = solve(9);\ni32 third = solve(7);");
    fs::write(project.join("src/main.w"), &original_text).unwrap();
    let mut client = client(&project);
    let store = Arc::new(SourceStore::open(&fixture.0.join("daemon"), 1 << 20).unwrap());
    let original = publish(&mut client, &store).await;
    let worker = worker(&store);
    let retained = worker.open_build_diagnostics(original.clone(), Duration::from_secs(20)).await.unwrap().unwrap();

    let updated_text = original_text.replace("solve(9)", "solve(11)");
    fs::write(project.join("src/main.w"), &updated_text).unwrap();
    let updated = publish(&mut client, &store).await;
    let current = worker.open_build_diagnostics(updated.clone(), Duration::from_secs(20)).await.unwrap().unwrap();
    assert_ne!(original.root, updated.root);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let historical = emitted_startup(retained.project_direct_startup_comptime(request("value", layout)).await.unwrap());
        assert_startup_evidence(&historical, original.root, layout, &original_text, 9);
        let fresh = emitted_startup(current.project_direct_startup_comptime(request("value", layout)).await.unwrap());
        let emitted_bytes = assert_startup_evidence(&fresh, updated.root, layout, &updated_text, 11);
        assert_eq!(historical.compiler_image(), fresh.compiler_image());
        assert_ne!(historical.authored_cst(), fresh.authored_cst());
        assert_eq!(historical.evaluators()[0].digest(), fresh.evaluators()[0].digest());
        assert_ne!(historical.evaluators()[1].digest(), fresh.evaluators()[1].digest());

        // Each partition independently fits both limits. Only the aggregate
        // distinguishes the whole successful reply from the failed one.
        let mut too_little = request("value", layout);
        too_little.output_bytes = emitted_bytes - 1;
        assert!(fresh.evaluators().iter().all(|part| part.bytes().len() <= too_little.output_bytes));
        assert!(current.project_direct_startup_comptime(too_little).await.unwrap_err().to_string()
            .contains("output byte budget"));
        let mut exact = request("value", layout);
        exact.output_bytes = emitted_bytes;
        let whole = emitted_startup(current.project_direct_startup_comptime(exact).await.unwrap());
        assert_eq!(assert_startup_evidence(&whole, updated.root, layout, &updated_text, 11), emitted_bytes);
        assert!(whole.evaluators().iter().zip(fresh.evaluators())
            .all(|(left, right)| left.digest() == right.digest() && left.bytes() == right.bytes()));
        assert!(matches!(current.project_direct_function_comptime(request("value", layout)).await.unwrap(),
            RootedDirectFunctionComptimeProjection::Refused(RootedClosedComptimeRefusal::Compiler(
                ClosedComptimeRejection::Unsupported { feature: "direct comptime startup call limit exceeded", .. }))));
    }
    assert_eq!(client.acknowledged_build_source("app").unwrap().unwrap().root, updated.root);
    drop((retained, current));
    drop((worker, client));
    Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
}
