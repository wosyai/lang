#![cfg(target_os = "linux")]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use wosy_cli::source_preparation::{
    prepare, prepare_closed_comptime_input_facts, prepare_comptime_watches, ClosedComptimeInputFacts,
    ClosedComptimeInputSelection, PreparedComptimeWatches, PreparedSources,
};
use wosy_compiler::{ComptimeParameterOrigin, ComptimeParameterValue, ScalarTargetLayout, ScalarType};
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
