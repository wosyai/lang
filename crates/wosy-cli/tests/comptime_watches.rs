#![cfg(target_os = "linux")]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use wosy_cli::source_preparation::{
    prepare, prepare_closed_comptime_input_facts, prepare_comptime_watches, prepare_forwarded_pair_comptime_input_facts,
    prepare_pair_call_graph_comptime_input_facts, ClosedComptimeInputFacts, ClosedComptimeInputSelection,
    ForwardedPairComptimeInputFacts, ForwardedPairComptimeInputSelection, PairCallGraphComptimeInputFacts,
    PreparedComptimeWatches, PreparedSources,
};
use wosy_compiler::{
    ComptimeParameterOrigin, ComptimeParameterValue, DirectComptimeArgument, PairStartupOccurrence,
    ScalarTargetLayout, ScalarType,
};
use wosy_project::{
    packages::{ComptimeWatchLimits, WatchRoot},
    ProjectConfiguration,
};
use wosy_protocol::{FileMetadata, Kind, Node, NodeId};

const APP_SOURCE: &str = r#"%%start
payload = namespace assets "main.w";
i32 value = comptime prepare { 11 };
%%end
"#;
const PACKAGE_SOURCE: &str = "%%start\n# WHY: café ☕\ni32 asset = comptime prepare { 42 };\n%%end\n";
const PACKAGE: &str = r#"
[package]
name = "assets"
source_root = "src"

[comptime.prepare]
watch = ["schemas/**/*.json", "schemas/root.json", "schemas/caf?.txt"]

[comptime.unused]
watch = [false]
"#;
const APP: &str = r#"
[package]
name = "app"
default_target = "app"

[dependencies.assets]
path = "../assets"

[comments]
categories = ["WHY"]

[builders.never]
command = ["hard-fail-if-a-builder-starts"]
toolchain_identity = "fixture"

[runners.never]
command = ["hard-fail-if-a-runner-starts"]

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

[comptime.prepare]
watch = ["local/*.txt"]

[comptime."assets:prepare"]
watch_add = ["recordings/**/*.json", "recordings/./unsupported.txt"]
"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("wosy-comptime-watch-{}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fixture.put("app/src/main.w", APP_SOURCE);
        fixture.put("app/wosy.toml", APP.replace(", \"recordings/./unsupported.txt\"", ""));
        fixture.put("assets/src/main.w", PACKAGE_SOURCE);
        fixture.put("assets/wosy.toml", PACKAGE);
        fixture.put("assets/schemas/root.json", "root");
        fixture.put("assets/schemas/deep/child.json", "nested");
        fixture.put("assets/schemas/café.txt", "utf8");
        fixture.put("app/recordings/extra.json", "consumer");
        fixture.put("app/local/value.txt", "local");
        fixture
    }

    fn put(&self, path: &str, content: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn app(&self) -> PathBuf { self.0.join("app") }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn observe(root: &Path, owner: &str) -> (NodeId, PreparedComptimeWatches) {
    let (sources, watch) = prepare_comptime_watches(
        root, "app", "dev", owner, "prepare", ComptimeWatchLimits::default(),
    ).unwrap();
    assert_eq!(sources.parsed, 2);
    assert_eq!(sources.reused, 0);
    (sources.root, watch)
}

fn rows(watches: &PreparedComptimeWatches) -> Vec<(WatchRoot, &Path, String)> {
    watches.files.iter().map(|file| (file.root, file.path.as_path(), file.revision.0.clone())).collect()
}

fn content<'a>(root: WatchRoot, path: &'a str, bytes: &str) -> (WatchRoot, &'a Path, String) {
    (root, Path::new(path), blake3::hash(bytes.as_bytes()).to_hex().to_string())
}

#[test]
fn merges_selected_owner_and_consumer_with_one_canonical_identity_per_match() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let (root, first) = observe(&app, "assets");
    assert_eq!(root, prepare(&app, Some("app"), None).unwrap().root);
    assert_eq!((&first.target[..], &first.profile[..], &first.id[..]), ("app", "dev", "prepare"));
    assert_eq!((&first.site.source.package[..], &first.site.source.path[..]), ("assets", "src/main.w"));
    let range = first.site.range;
    assert_eq!(&PACKAGE_SOURCE[range.start as usize..range.end as usize], "comptime prepare { 42 }");
    assert_eq!(rows(&first), [
        content(WatchRoot::OwningPackage, "schemas/café.txt", "utf8"),
        content(WatchRoot::OwningPackage, "schemas/deep/child.json", "nested"),
        content(WatchRoot::OwningPackage, "schemas/root.json", "root"),
        content(WatchRoot::ConsumingProject, "recordings/extra.json", "consumer"),
    ]);

    let local = observe(&app, "app").1;
    assert_eq!(local.site.source.package, "app");
    assert_eq!(rows(&local), [content(WatchRoot::OwningPackage, "local/value.txt", "local")]);
    fixture.put("app/ignored/unused.json", "irrelevant");
    fixture.put("assets/schemas/other.bin", "not a match");
    fixture.put("assets/schemas/.hidden.json", "not selected by this dialect");
    let (same_root, same_watches) = observe(&app, "assets");
    assert_eq!(same_root, root);
    assert_eq!(same_watches, first);

    fixture.put("assets/schemas/root.json", "ROOT"); // same-length content edit
    fixture.put("app/recordings/deep/zero.json", "");
    fs::remove_file(fixture.0.join("assets/schemas/deep/child.json")).unwrap();
    let (next_root, changed) = observe(&app, "assets");
    assert_eq!(next_root, root);
    assert_eq!(changed.site, first.site);
    assert_eq!(rows(&changed), [
        content(WatchRoot::OwningPackage, "schemas/café.txt", "utf8"),
        content(WatchRoot::OwningPackage, "schemas/root.json", "ROOT"),
        content(WatchRoot::ConsumingProject, "recordings/deep/zero.json", ""),
        content(WatchRoot::ConsumingProject, "recordings/extra.json", "consumer"),
    ]);
    assert_ne!(first, changed);
    assert_eq!(observe(&app, "app").1, local);
    assert!(!app.join(".wosy").exists());
}

#[test]
fn checks_selected_site_glob_dialect_and_shared_budget_without_ordinary_side_effects() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let refusal = |owner, site, limits| prepare_comptime_watches(
        &app, "app", "dev", owner, site, limits,
    ).err().unwrap();
    assert!(refusal("unknown", "prepare", ComptimeWatchLimits::default()).contains("resolved project"));
    assert!(refusal("assets", "unused", ComptimeWatchLimits::default()).contains("reachable authored site"));
    assert!(refusal("assets", "prepare", ComptimeWatchLimits {
        files: 3, ..Default::default()
    }).contains("matched file limit")); // 3 in the owner and 1 in the consumer
    assert!(refusal("assets", "prepare", ComptimeWatchLimits {
        content_bytes: 16, ..Default::default()
    }).contains("file byte limit")); // 14 in the dependency, plus the consumer
    assert!(refusal("assets", "prepare", ComptimeWatchLimits {
        entries: 1, ..Default::default()
    }).contains("entry limit"));
    fixture.put("app/wosy.toml", APP);
    assert!(refusal("assets", "prepare", ComptimeWatchLimits::default()).contains("unsupported comptime watch glob"));
    assert!(prepare(&app, Some("app"), None).is_ok());
    fixture.put("assets/src/main.w",
        PACKAGE_SOURCE.replace("%%end", "i32 second = comptime prepare { 43 };\n%%end"));
    assert!(refusal("assets", "prepare", ComptimeWatchLimits::default()).contains("id is repeated"));
    assert!(!app.join(".wosy").exists());
}

const CLOSED_ASSET: &str = r#"%%start
# WHY: parâmetro real da aplicação ☕
i32 asset = comptime prepare {
    i32 value = core.comp_param("wanted", 41);
    value
};
%%end
"#;

fn with_evaluator_tools() -> String {
    let tools = r#"
[builders.compile]
command = ["unresolved fixture compiler", "literal *"]
args = ["--strict"]
toolchain_identity = "declared toolchain only"

[runners.evaluate]
command = ["unresolved fixture runner", "$literal"]
args = ["--typed"]
"#;
    let selected = "[targets.app.profiles.dev]\ncomptime_builder = \"compile\"\ncomptime_runner = \"evaluate\"\nbuild_permissions = []\nbuild_mounts = []";
    APP.replace(", \"recordings/./unsupported.txt\"", "")
        .replace("[targets.app.profiles.dev]", selected) + tools
}

fn closed_scope() -> ClosedComptimeInputSelection<'static> {
    ClosedComptimeInputSelection {
        target: "app", profile: "dev", owner: "assets", id: "prepare",
        evaluator_layout: ScalarTargetLayout::NATIVE64,
        watch_limits: ComptimeWatchLimits::default(),
        max_ir_bytes: 1 << 16,
    }
}

fn observe_closed(root: &Path) -> (PreparedSources, ClosedComptimeInputFacts) {
    prepare_closed_comptime_input_facts(root, closed_scope()).unwrap()
}

#[test]
fn closed_facts_capture_the_exact_authored_file_and_evaluator_without_replacing_the_source_root() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let manifest = with_evaluator_tools();
    fixture.put("app/wosy.toml", &manifest);
    fixture.put("assets/src/main.w", CLOSED_ASSET);
    let (prepared, baseline) = observe_closed(&app);
    assert_eq!((prepared.parsed, prepared.reused), (2, 0));
    assert_eq!(baseline.source_root(), prepared.root);
    let (ordinary, watch_only) = prepare_comptime_watches(
        &app, "app", "dev", "assets", "prepare", ComptimeWatchLimits::default(),
    ).unwrap();
    assert_eq!(prepared.root, ordinary.root);
    assert_eq!(baseline.watches(), &watch_only);

    let source = &watch_only.site.source;
    assert_eq!((&source.package[..], &source.path[..]), ("assets", "src/main.w"));
    let root = Node::decode(&prepared.delta.nodes[&prepared.root]).unwrap();
    let dependency = root.children.into_iter().find_map(|id| {
        let node = Node::decode(&prepared.delta.nodes[&id]).unwrap();
        if node.kind != Kind::File { return None; }
        let metadata: FileMetadata = serde_json::from_slice(&node.data).unwrap();
        (metadata.source.package == "assets").then_some((node, metadata))
    }).expect("rooted authored file of selected package");
    assert_eq!(dependency.0.children[0], baseline.authored_cst());
    assert_eq!(dependency.1.revision.hex(), source.revision);
    assert_eq!(Node::id(&prepared.delta.nodes[&baseline.authored_cst()]), baseline.authored_cst());

    let [read] = baseline.parameters() else { panic!("exactly one consumed parameter") };
    assert_eq!((&read.name[..], &read.context, read.resolved),
        ("wanted", &ScalarType::I32, ComptimeParameterValue::Integer(41)));
    assert!(matches!(read.origin, ComptimeParameterOrigin::Default { .. }));
    assert_eq!(baseline.result_type(), &ScalarType::I32);
    assert_eq!(baseline.evaluator_layout(), ScalarTargetLayout::NATIVE64);
    let (fresh, diagnostics) = wosy_compiler::parse_source_with_snapshot(
        source.clone(), CLOSED_ASSET.into(), &["WHY".into()],
    );
    assert!(diagnostics.is_empty());
    let oracle = wosy_compiler::derive_closed_comptime_fragment(
        &fresh, "prepare", ScalarTargetLayout::NATIVE64, |_| None,
    ).unwrap();
    let ir = oracle.llvm().to_text();
    assert_eq!(baseline.evaluator_ir().bytes, ir.len());
    assert_eq!(baseline.evaluator_ir().blake3, blake3::hash(ir.as_bytes()));

    let tools = baseline.declared_tools();
    assert_eq!((tools.target(), tools.profile()), ("app", "dev"));
    assert_eq!(tools.builder().1.command, ["unresolved fixture compiler", "literal *"]);
    assert_eq!(tools.builder().1.args, ["--strict"]);
    assert_eq!(tools.builder().1.toolchain_identity, "declared toolchain only");
    assert_eq!(tools.runner().1.command, ["unresolved fixture runner", "$literal"]);
    assert_eq!(tools.runner().1.args, ["--typed"]);
    assert!(tools.build_permissions().unwrap().is_empty() && tools.build_mounts().unwrap().is_empty());
    assert_eq!(observe_closed(&app).1, baseline);

    fixture.put("assets/not-a-watch", "irrelevant");
    assert_eq!(observe_closed(&app).1, baseline);
    fixture.put("app/recordings/extra.json", "changed consumer watch");
    fixture.put("assets/schemas/root.json", "changed owner watch");
    let (after, changed) = observe_closed(&app);
    assert_eq!(after.root, prepared.root);
    assert_eq!(changed.authored_cst(), baseline.authored_cst());
    assert_eq!(changed.evaluator_ir(), baseline.evaluator_ir());
    assert_eq!(changed.declared_tools(), baseline.declared_tools());
    assert_eq!(changed.parameters(), baseline.parameters());
    assert_ne!(changed.watches(), baseline.watches());

    fixture.put("app/wosy.toml", manifest.replace("--typed", "--selected-argv-changed"));
    let (_, argv) = observe_closed(&app);
    assert_ne!(argv.configuration_identity(), changed.configuration_identity());
    assert_ne!(argv.declared_tools(), changed.declared_tools());
    assert_eq!(argv.evaluator_ir(), changed.evaluator_ir());
    assert_eq!(argv.watches(), changed.watches());

    fixture.put("app/wosy.toml", manifest + "\n[targets.app.profiles.dev.params]\nwanted = 42\n");
    let (_, overridden) = observe_closed(&app);
    assert_eq!(overridden.authored_cst(), baseline.authored_cst());
    assert_ne!(overridden.evaluator_ir(), baseline.evaluator_ir());
    assert_eq!(overridden.watches(), changed.watches());
    assert_eq!(overridden.parameters()[0].origin, ComptimeParameterOrigin::Profile);
    assert_eq!(overridden.parameters()[0].resolved, ComptimeParameterValue::Integer(42));
    assert!(!app.join(".wosy").exists());
}

#[test]
fn closed_facts_refuse_missing_declarations_unresolved_capabilities_and_unsupported_bodies() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let error = || prepare_closed_comptime_input_facts(&app, closed_scope()).err().unwrap();
    assert!(error().contains("explicit comptime builder and runner"));
    let manifest = with_evaluator_tools();
    fixture.put("app/wosy.toml", manifest.replace("build_permissions = []", "build_permissions = [\"network\"]"));
    assert!(error().contains("does not resolve build permissions or mounts"));
    fixture.put("app/wosy.toml", &manifest);
    fixture.put("assets/src/main.w", CLOSED_ASSET.replace("    value\n", "    value + 1\n"));
    assert!(error().contains("not a closed scalar expression"));
    fixture.put("assets/src/main.w", CLOSED_ASSET);
    let mut scope = closed_scope();
    scope.max_ir_bytes = 0;
    assert!(prepare_closed_comptime_input_facts(&app, scope).err().unwrap().contains("exceeds byte budget"));
    assert!(prepare(&app, Some("app"), None).is_ok());
    assert!(!app.join(".wosy").exists());
}

const FORWARDED_APP: &str = r#"%%start
# WHY: argumentos com café ☕
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32(u32, u32) relay = fn(upstream_left, upstream_right) { solve(upstream_left, upstream_right) };
u32 first = relay(7, 9);
u32 reversed = relay(9, 7);
u32 repeated = relay(7, 9);
u32 changed = relay(7, 13);
%%end
"#;

fn forwarded_manifest() -> String {
    with_evaluator_tools().replace("[dependencies.assets]\npath = \"../assets\"\n\n", "")
}

fn forwarded_scope(layout: ScalarTargetLayout) -> ForwardedPairComptimeInputSelection<'static> {
    ForwardedPairComptimeInputSelection {
        target: "app", profile: "dev", id: "prepare", evaluator_layout: layout,
        watch_limits: ComptimeWatchLimits::default(), max_ir_bytes: 1 << 16,
    }
}

fn observe_forwarded(root: &Path, layout: ScalarTargetLayout) -> (PreparedSources, ForwardedPairComptimeInputFacts) {
    prepare_forwarded_pair_comptime_input_facts(root, forwarded_scope(layout)).unwrap()
}

fn assert_forwarded_tuples(facts: &ForwardedPairComptimeInputFacts, permutation: [usize; 2]) {
    assert_eq!(facts.forwarding().startup_argument_positions(), permutation);
    let tuples = [[7, 9], [9, 7], [7, 9], [7, 13]];
    assert_eq!(facts.calls().len(), tuples.len());
    for ((call, values), slot) in facts.calls().iter().zip(tuples).zip([0, 1, 0, 2]) {
        let expected = permutation.map(|position| DirectComptimeArgument::U32(values[position]));
        assert_eq!(call.facts().inputs().each_ref().map(|input| input.value()), expected);
        assert_eq!(call.facts().result(), &ScalarType::U32);
        assert_eq!(call.facts().site(), &facts.watches().site);
        assert_eq!(call.evaluator_index(), slot);
    }
    let [first, second, changed] = facts.evaluator_irs() else { panic!("three distinct evaluator slots"); };
    assert_eq!(first, second);
    assert_ne!(first, changed);
}

#[test]
fn forwarded_facts_authenticate_watched_bytes_and_literal_tuple_sites_separately() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let manifest = forwarded_manifest();
    let swapped = FORWARDED_APP.replace("solve(upstream_left, upstream_right)", "solve(upstream_right, upstream_left)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        fixture.put("app/wosy.toml", &manifest);
        fixture.put("app/src/main.w", FORWARDED_APP);
        fixture.put("app/local/value.txt", "local");
        let (source, baseline) = observe_forwarded(&app, layout);
        assert_eq!((source.parsed, source.reused), (1, 0));
        assert_eq!(source.root, baseline.source_root());
        let ordinary = prepare(&app, Some("app"), None).unwrap();
        assert_eq!(source.root, ordinary.root);
        assert_eq!(source.delta.nodes, ordinary.delta.nodes);
        let (_, watch_only) = prepare_comptime_watches(
            &app, "app", "dev", "app", "prepare", ComptimeWatchLimits::default(),
        ).unwrap();
        assert_eq!(baseline.watches(), &watch_only);
        assert_eq!(rows(baseline.watches()), [content(WatchRoot::OwningPackage, "local/value.txt", "local")]);
        let rooted = Node::decode(&source.delta.nodes[&source.root]).unwrap();
        let (file, metadata) = rooted.children.into_iter().find_map(|id| {
            let node = Node::decode(&source.delta.nodes[&id]).unwrap();
            (node.kind == Kind::File)
                .then(|| (serde_json::from_slice::<FileMetadata>(&node.data).unwrap(), node))
                .map(|(metadata, node)| (node, metadata))
        }).expect("the actual singleton source record");
        assert_eq!(file.children[0], baseline.authored_cst());
        assert_eq!(metadata.revision.hex(), baseline.watches().site.source.revision);
        assert_eq!(baseline.evaluator_layout(), layout);
        assert_forwarded_tuples(&baseline, [0, 1]);

        let preview = wosy_cli::comptime_fragment::plan_forwarded_pair_comptime_startup(&app, "app", "dev", "prepare", layout).unwrap();
        assert_eq!(preview.plan().calls().len(), baseline.calls().len());
        assert_eq!(preview.plan().evaluators().len(), baseline.evaluator_irs().len());
        for (call, actual) in preview.plan().calls().iter().zip(baseline.calls()) {
            assert_eq!(call.facts(), actual.facts());
            assert_eq!(call.evaluator_index(), actual.evaluator_index());
        }
        for (expected, actual) in preview.plan().evaluators().iter().zip(baseline.evaluator_irs()) {
            let text = expected.llvm().to_text();
            assert_eq!(actual.bytes, text.len());
            assert_eq!(actual.blake3, blake3::hash(text.as_bytes()));
        }
        assert_eq!(baseline.declared_tools().builder().1.toolchain_identity, "declared toolchain only");
        assert_eq!(baseline.declared_tools().runner().1.args, ["--typed"]);

        fixture.put("app/ignored/not-selected.tsv", "unrelated");
        fixture.put("app/local/value.txt", "LOCAL"); // same-length watched edit
        let (_, watched) = observe_forwarded(&app, layout);
        assert_eq!(watched.source_root(), baseline.source_root());
        assert_eq!(watched.authored_cst(), baseline.authored_cst());
        assert_eq!(watched.configuration_identity(), baseline.configuration_identity());
        assert_eq!(watched.declared_tools(), baseline.declared_tools());
        assert_eq!(watched.evaluator_irs(), baseline.evaluator_irs());
        assert_eq!(watched.watches().site, baseline.watches().site);
        assert_eq!(rows(watched.watches()), [content(WatchRoot::OwningPackage, "local/value.txt", "LOCAL")]);
        assert_forwarded_tuples(&watched, [0, 1]);

        fixture.put("app/src/main.w", &swapped);
        let (_, now_swapped) = observe_forwarded(&app, layout);
        assert_ne!(now_swapped.source_root(), baseline.source_root());
        assert_ne!(now_swapped.authored_cst(), baseline.authored_cst());
        assert_eq!(now_swapped.evaluator_irs(), watched.evaluator_irs());
        assert_eq!(now_swapped.watches().files, watched.watches().files);
        assert_forwarded_tuples(&now_swapped, [1, 0]);

        fixture.put("app/wosy.toml", manifest.replace("--typed", "--selected-literal-changed"));
        let (_, changed_tools) = observe_forwarded(&app, layout);
        assert_ne!(changed_tools.source_root(), now_swapped.source_root());
        assert_ne!(changed_tools.configuration_identity(), now_swapped.configuration_identity());
        assert_ne!(changed_tools.declared_tools(), now_swapped.declared_tools());
        assert_eq!(changed_tools.authored_cst(), now_swapped.authored_cst());
        assert_eq!(changed_tools.evaluator_irs(), now_swapped.evaluator_irs());
        assert_eq!(changed_tools.watches(), now_swapped.watches());
        assert!(!app.join(".wosy").exists());
    }
}

#[test]
fn forwarded_input_observation_refuses_out_of_scope_inputs_and_cumulative_budgets() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let layout = ScalarTargetLayout::NATIVE64;
    let manifest = forwarded_manifest();
    fixture.put("app/src/main.w", FORWARDED_APP);
    let refuse = |scope| prepare_forwarded_pair_comptime_input_facts(&app, scope).err().unwrap();
    fixture.put("app/wosy.toml", manifest.replace("build_permissions = []", "build_permissions = [\"network\"]"));
    assert!(refuse(forwarded_scope(layout)).contains("does not resolve build permissions or mounts"));
    fixture.put("app/wosy.toml", &manifest);
    let mut scope = forwarded_scope(layout);
    scope.watch_limits.content_bytes = 4; // real five-byte selected watch
    assert!(refuse(scope).contains("file byte limit"));
    let (_, full) = observe_forwarded(&app, layout);
    let total: usize = full.evaluator_irs().iter().map(|item| item.bytes).sum();
    let mut scope = forwarded_scope(layout);
    scope.max_ir_bytes = total - 1;
    assert!(full.evaluator_irs().iter().all(|item| item.bytes <= scope.max_ir_bytes));
    assert!(refuse(scope).contains("evaluator IR exceeds byte budget"));
    let mut scope = forwarded_scope(layout);
    scope.max_ir_bytes = total;
    let (_, exact) = prepare_forwarded_pair_comptime_input_facts(&app, scope).unwrap();
    assert_eq!(exact.evaluator_irs(), full.evaluator_irs());

    fixture.put("app/src/main.w", " ".repeat(wosy_compiler::CLOSED_COMPTIME_MAX_SOURCE_BYTES + 1));
    assert!(refuse(forwarded_scope(layout)).contains("authored byte budget"));
    // A real declared local namespace may not cause its source to be opened
    // when this exact consumer-singleton route is requested.
    fixture.put("app/wosy.toml", with_evaluator_tools());
    fixture.put("app/src/main.w", FORWARDED_APP.replace("u32 first =", "more = namespace assets \"must-not-open.w\";\nu32 first ="));
    assert_eq!(refuse(forwarded_scope(layout)), "forwarded comptime input requires the selected singleton entry");
    assert!(!app.join(".wosy").exists());
}

const GRAPH_PREFIX: &str = r#"%%start
# WHY: startup com café ☕
u32(u32, u32) solve = fn(left, right) { comptime prepare { right ^ left } };
u32(u32, u32) swap_data = fn(swap_left, swap_right) { solve(swap_right, swap_left) };
u32(bool, u32, u32) choose = fn(select_path, data_left, data_right) {
    if (select_path) { swap_data(data_left, data_right) } else { data_right }
};
u32 seed_value = choose(false, 1, 37);
u32 carried = choose(false, 1, seed_value);"#;

fn graph_app() -> String {
    format!("{GRAPH_PREFIX}\nu32 reached = choose(true, 7, carried);\n\
             u32 same = solve(carried, 7);\nu32 changed = solve(13, 7);\n%%end\n")
}

fn graph_manifest() -> String {
    forwarded_manifest() + r#"
[runners.audit]
command = ["unresolved audit runner"]
args = ["--audit"]

[targets.app.artifacts.main.profiles.audit]
backend = "llvm"
output = "native"
builder = "never"
run_runner = "never"

[targets.app.profiles.audit]
main_artifact = "main"
comptime_builder = "compile"
comptime_runner = "audit"
build_permissions = []
build_mounts = []
"#
}

fn assert_graph_facts(facts: &PairCallGraphComptimeInputFacts, text: &str, layout: ScalarTargetLayout, seed: u32) {
    assert_eq!(facts.evidence().site(), &facts.watches().site);
    assert_eq!(facts.evidence().layout(), layout);
    let slots: Vec<_> = facts.evidence().occurrences().collect();
    let [
        PairStartupOccurrence::Bypass(seed_slot),
        PairStartupOccurrence::Bypass(carried),
        PairStartupOccurrence::Specialization(reached, path),
        PairStartupOccurrence::Specialization(same, _),
        PairStartupOccurrence::Specialization(changed, _),
    ] = slots.as_slice() else { panic!("two bypasses and three reached calls in ordinary order"); };
    let slice = |range: wosy_syntax::ByteSpan| &text[range.start as usize..range.end as usize];
    assert_eq!(seed_slot.returned_value(), DirectComptimeArgument::U32(seed));
    assert_eq!(slice(carried.returned_argument().range), "seed_value");
    assert_eq!(carried.returned_origin().literal(), seed_slot.returned_argument());
    assert_eq!(carried.returned_origin().prior_bypass_slot(), Some(0));
    assert_eq!(slice(reached.facts().destination().range), "reached");
    assert_eq!(path.startup_argument_positions(), [2, 1]);
    assert_eq!(path.hop_indices(), &[1, 0]);
    for (use_, expected, index) in [(reached, seed, 0), (same, seed, 0), (changed, 13, 1)] {
        assert_eq!(use_.evaluator_index(), index);
        assert_eq!(use_.facts().inputs().each_ref().map(|input| input.value()),
            [DirectComptimeArgument::U32(expected), DirectComptimeArgument::U32(7)]);
    }
    let [argument, _] = reached.facts().inputs();
    assert_eq!(slice(argument.argument().range), "carried");
    assert_eq!(argument.origin().prior_bypass_slot(), Some(1));
    assert_eq!(argument.origin().literal(), seed_slot.returned_argument());
    let [a, b] = facts.evaluator_irs() else { panic!("two ordered concrete tuples"); };
    assert_ne!(a, b);

    // Independent direct closed scalar oracle verifies exact egress, without
    // invoking the project builder/runner or compiling the evaluator module.
    for (observed, literal) in [(a, seed), (b, 13)] {
        let oracle_source = format!("%%start\nu32 answer = comptime prepare {{ 7 ^ {literal} }};\n%%end\n");
        // LLVM uses the logical source path in the module header. Use
        // the same path for whole-IR comparison, but keep the oracle's
        // own project, package and digest for its different source bytes.
        let identity = wosy_syntax::SourceIdentity::new(
            "test oracle".into(), "oracle".into(), facts.watches().site.source.path.clone(),
            blake3::hash(oracle_source.as_bytes()).to_hex().to_string(),
        );
        let (fresh, diagnostics) = wosy_compiler::parse_source_with_snapshot(identity, oracle_source, &[]);
        assert!(diagnostics.is_empty());
        let oracle = wosy_compiler::derive_closed_comptime_fragment(&fresh, "prepare", layout, |_| None).unwrap();
        let text = oracle.llvm().to_text();
        assert_eq!(observed.bytes, text.len());
        assert_eq!(observed.blake3, blake3::hash(text.as_bytes()));
    }
}

#[test]
fn pair_graph_observation_binds_current_source_profile_watches_and_unchanged_mapping() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let text = graph_app();
    fixture.put("app/wosy.toml", graph_manifest());
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        fixture.put("app/src/main.w", &text);
        fixture.put("app/local/value.txt", "local");
        let (source, baseline) = prepare_pair_call_graph_comptime_input_facts(&app, forwarded_scope(layout)).unwrap();
        assert_eq!((source.parsed, source.reused), (1, 0));
        assert_eq!(source.root, baseline.source_root());
        let ordinary = prepare(&app, Some("app"), None).unwrap();
        assert_eq!(ordinary.root, source.root);
        assert_eq!(ordinary.delta.nodes, source.delta.nodes);
        let root = Node::decode(&source.delta.nodes[&source.root]).unwrap();
        let (file, metadata) = root.children.iter().find_map(|id| {
            let node = Node::decode(&source.delta.nodes[id]).unwrap();
            if node.kind != Kind::File { return None; }
            let metadata: FileMetadata = serde_json::from_slice(&node.data).unwrap();
            Some((node, metadata))
        }).expect("singleton file record");
        assert_eq!(file.children[0], baseline.authored_cst());
        assert_eq!(metadata.revision.hex(), baseline.evidence().site().source.revision);
        let (_, watch_only) = prepare_comptime_watches(
            &app, "app", "dev", "app", "prepare", ComptimeWatchLimits::default(),
        ).unwrap();
        assert_eq!(baseline.watches(), &watch_only);
        assert_eq!(rows(baseline.watches()), [content(WatchRoot::OwningPackage, "local/value.txt", "local")]);
        assert_eq!(baseline.declared_tools().builder().1.toolchain_identity, "declared toolchain only");
        assert_eq!(baseline.declared_tools().runner().1.args, ["--typed"]);
        assert_graph_facts(&baseline, &text, layout, 37);

        let mut scope = forwarded_scope(layout);
        scope.profile = "audit";
        let (_, audit) = prepare_pair_call_graph_comptime_input_facts(&app, scope).unwrap();
        assert_eq!(audit.source_root(), baseline.source_root());
        assert_eq!(audit.authored_cst(), baseline.authored_cst());
        assert_eq!(audit.configuration_identity(), baseline.configuration_identity());
        assert_eq!(audit.evaluator_irs(), baseline.evaluator_irs());
        assert_eq!(audit.watches().profile, "audit");
        assert_eq!(audit.watches().files, baseline.watches().files);
        assert_eq!(audit.declared_tools().runner().1.args, ["--audit"]);
        assert_graph_facts(&audit, &text, layout, 37);

        fixture.put("app/ignored/other.tsv", "unselected");
        let (_, unchanged) = prepare_pair_call_graph_comptime_input_facts(&app, forwarded_scope(layout)).unwrap();
        assert_eq!(unchanged.watches(), baseline.watches());
        fixture.put("app/local/value.txt", "LOCAL");
        let (_, watched) = prepare_pair_call_graph_comptime_input_facts(&app, forwarded_scope(layout)).unwrap();
        assert_eq!(watched.source_root(), baseline.source_root());
        assert_eq!(watched.authored_cst(), baseline.authored_cst());
        assert_eq!(watched.evaluator_irs(), baseline.evaluator_irs());
        assert_eq!(watched.watches().site, baseline.watches().site);
        assert_eq!(rows(watched.watches()), [content(WatchRoot::OwningPackage, "local/value.txt", "LOCAL")]);
        assert_graph_facts(&watched, &text, layout, 37);

        let edited = text.replace("1, 37", "1, 45");
        fixture.put("app/src/main.w", &edited);
        let (_, changed) = prepare_pair_call_graph_comptime_input_facts(&app, forwarded_scope(layout)).unwrap();
        assert_ne!(changed.source_root(), baseline.source_root());
        assert_ne!(changed.authored_cst(), baseline.authored_cst());
        assert_eq!(changed.watches().files, watched.watches().files);
        assert_ne!(changed.evaluator_irs()[0], baseline.evaluator_irs()[0]);
        assert_eq!(changed.evaluator_irs()[1], baseline.evaluator_irs()[1]);
        assert_graph_facts(&changed, &edited, layout, 45);
        assert!(!app.join(".wosy").exists());
    }
}

#[test]
fn pair_graph_observation_keeps_all_bypass_site_and_refuses_budget_or_scope_limits() {
    let fixture = Fixture::new();
    let app = fixture.app();
    let manifest = graph_manifest();
    fixture.put("app/wosy.toml", &manifest);
    let layout = ScalarTargetLayout::NATIVE64;
    let run = |scope| prepare_pair_call_graph_comptime_input_facts(&app, scope);
    let mut scope = forwarded_scope(layout);
    scope.max_ir_bytes = 0;
    fixture.put("app/src/main.w", format!("{GRAPH_PREFIX}\n%%end\n"));
    let (prepared, zero) = run(scope).unwrap();
    assert_eq!(prepared.root, prepare(&app, Some("app"), None).unwrap().root);
    let evidence = zero.evidence();
    assert_eq!(evidence.calls().len(), 0);
    assert_eq!(evidence.occurrences().len(), 2);
    assert!(evidence.occurrences().all(|item| matches!(item, PairStartupOccurrence::Bypass(_))));
    assert_eq!(evidence.site(), &zero.watches().site);
    assert_eq!(evidence.layout(), layout);
    assert!(zero.evaluator_irs().is_empty());
    assert_eq!(rows(zero.watches()), [content(WatchRoot::OwningPackage, "local/value.txt", "local")]);
    fixture.put("app/src/main.w", graph_app());
    let (_, full) = run(forwarded_scope(layout)).unwrap();
    let total: usize = full.evaluator_irs().iter().map(|ir| ir.bytes).sum();
    let mut scope = forwarded_scope(layout);
    scope.max_ir_bytes = total - 1;
    assert!(full.evaluator_irs().iter().all(|ir| ir.bytes <= scope.max_ir_bytes));
    assert!(run(scope).err().unwrap().contains("evaluator IR exceeds byte budget"));
    let mut scope = forwarded_scope(layout);
    scope.max_ir_bytes = total;
    assert_eq!(run(scope).unwrap().1.evaluator_irs(), full.evaluator_irs());

    let mut scope = forwarded_scope(layout);
    scope.watch_limits.content_bytes = 4;
    assert!(run(scope).err().unwrap().contains("file byte limit"));
    fixture.put("app/wosy.toml", manifest.replace("build_permissions = []", "build_permissions = [\"network\"]"));
    assert!(run(forwarded_scope(layout)).err().unwrap().contains("does not resolve build permissions or mounts"));
    assert!(!app.join(".wosy").exists());
}

#[test]
#[ignore = "requires one authorized file-only ordinary Dev5 Git installation in a disposable fixture"]
fn installed_git_owner_watch_uses_actual_pinned_package_root_without_git_process() {
    let root = PathBuf::from(std::env::var_os("WOSY_COMPTIME_WATCH_GIT_PROJECT").expect("owned fixture"));
    let cfg = ProjectConfiguration::load(&root).unwrap();
    let (package, _) = cfg.git_package("assets").unwrap().expect("not an installed pinned Git package");
    assert!(package.git_pin_identity.is_some() && package.patch_stack_id.is_some());
    let (prepared, initial) = observe(&root, "assets");
    assert_eq!(prepared, prepare(&root, Some("app"), None).unwrap().root);
    assert_eq!(initial.site.source.project, package.identity.project);
    assert_eq!(rows(&initial), [
        content(WatchRoot::OwningPackage, "schemas/owned.json", "pinned Git asset"),
        content(WatchRoot::ConsumingProject, "recordings/extra.json", "consumer"),
    ]);
    // Only the disposable consumer file is edited. The installed tree and
    // lock are read-only throughout this test.
    fs::write(root.join("recordings/extra.json"), "CONSUMER").unwrap();
    let (next, changed) = observe(&root, "assets");
    assert_eq!(next, prepared);
    assert_eq!(changed.site, initial.site);
    assert_eq!(rows(&changed), [
        content(WatchRoot::OwningPackage, "schemas/owned.json", "pinned Git asset"),
        content(WatchRoot::ConsumingProject, "recordings/extra.json", "CONSUMER"),
    ]);
    assert!(!root.join(".wosy").exists());
}
