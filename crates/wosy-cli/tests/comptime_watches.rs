#![cfg(target_os = "linux")]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use wosy_cli::source_preparation::{prepare, prepare_comptime_watches, PreparedComptimeWatches};
use wosy_project::{
    packages::{ComptimeWatchLimits, WatchRoot},
    ProjectConfiguration,
};
use wosy_protocol::NodeId;

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
