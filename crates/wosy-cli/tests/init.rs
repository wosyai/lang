#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use wosy_project::{packages::PackageLock, ProjectConfiguration, ProjectContext};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        loop {
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("wosy-init-{}-{serial}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => return Self(dir),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot create fixture: {error}"),
            }
        }
    }

    fn directory(&self, name: &str) -> PathBuf {
        let dir = self.0.join(name);
        fs::create_dir(&dir).unwrap();
        dir
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wosy(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wosy"))
        .current_dir(project)
        .env("PATH", "") // Init, parse and local package commands need no external tools.
        .args(args)
        .output()
        .unwrap()
}

fn pass(result: Output) {
    assert!(
        result.status.success(),
        "status: {}; stderr: {}",
        result.status,
        String::from_utf8_lossy(&result.stderr),
    );
}

fn fail(result: Output, message: &str) {
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success() && stderr.contains(message), "{stderr}");
}

#[test]
fn init_creates_a_parseable_package_with_empty_lock_and_named_profiles() {
    let temp = Temp::new();
    let app = temp.directory("my_app");
    let library = temp.directory("my_library");
    fs::write(app.join("notes.txt"), "keep me").unwrap();
    for package in [&app, &library] {
        pass(wosy(package, &["init"]));
        pass(wosy(package, &["parse"]));
        assert!(package.join(".wosy/cst/current.json").is_file());
        assert_eq!(
            ProjectContext::load_parse_target(package, None).unwrap().comment_categories,
            [
                "INTENT", "WHY", "ASSUMPTION", "RISK", "SOURCE", "WORKAROUND", "TODO",
                "CONSTRAINT", "SECURITY", "AUDIT", "OWNERSHIP", "TYPE", "REPLAY", "NOISE",
            ],
        );
        assert!(!package.join("references").exists());
        let lock = fs::read_to_string(package.join("wosy.lock")).unwrap();
        assert_eq!(
            PackageLock::parse(lock.clone()).unwrap(),
            PackageLock::default(),
        );
        assert_eq!(lock, PackageLock::default().render());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &fs::read_to_string(package.join("wosy.meta.json")).unwrap()
            )
            .unwrap(),
            serde_json::json!({}),
        );
        assert_eq!(
            fs::read_to_string(package.join(".gitignore")).unwrap(),
            "/deps/\n/.wosy/\n/references/wosy/\n",
        );
    }
    assert_eq!(fs::read_to_string(app.join("notes.txt")).unwrap(), "keep me");
    let lock = fs::read(app.join("wosy.lock")).unwrap();
    pass(wosy(&app, &["dep", "add", "../my_library"]));
    pass(wosy(&app, &["dep", "sync"]));
    assert_eq!(fs::read(app.join("wosy.lock")).unwrap(), lock);
    assert!(!app.join("deps").exists());

    let config = ProjectConfiguration::load(&app).unwrap();
    assert_eq!(config.default_target, "app");
    assert_eq!(config.module_source_root.unwrap().as_path(), Path::new("src"));
    assert_eq!(config.targets["app"].root, Path::new("src/main.w"));
    assert_eq!(
        config.builders["llvm_wasm"].command,
        ["python3", "scripts/build-wasm"],
    );
    assert_eq!(
        config.runners["wasmtime"].command,
        ["python3", "scripts/run-wasmtime"],
    );
    assert_eq!(
        config.targets["app"].artifacts["main"].profiles["dev"].output,
        "native",
    );
    assert_eq!(
        config.targets["app"].artifacts["main"].profiles["wasm"].output,
        "wasm-wasip1",
    );
    assert_eq!(
        config.resolved_packages["my_library"].identity.path.as_path(),
        Path::new("src"),
    );
}

#[test]
fn init_does_not_clobber_a_conflicting_file_or_symlink() {
    let temp = Temp::new();
    let root = temp.directory("existing_app");
    let other = temp.directory("unrelated");
    std::os::unix::fs::symlink(&other, root.join("scripts")).unwrap();
    fs::write(root.join("README"), "leave this alone").unwrap();
    fail(wosy(&root, &["init"]), "scripts already exists");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    assert_eq!(fs::read_link(root.join("scripts")).unwrap(), other);
    assert_eq!(fs::read_dir(&other).unwrap().count(), 0);
    assert_eq!(fs::read_to_string(root.join("README")).unwrap(), "leave this alone");
    assert!(!root.join("wosy.toml").exists());

    let existing = temp.directory("already_initialized");
    pass(wosy(&existing, &["init"]));
    fs::write(existing.join("src/main.w"), "my own source").unwrap();
    let manifest = fs::read(existing.join("wosy.toml")).unwrap();
    fail(wosy(&existing, &["init"]), "already exists");
    assert_eq!(fs::read(existing.join("wosy.toml")).unwrap(), manifest);
    assert_eq!(fs::read(existing.join("src/main.w")).unwrap(), b"my own source");

    let invalid = temp.directory("Not-Snake-Case");
    fail(wosy(&invalid, &["init"]), "snake_case project directory name");
    assert_eq!(fs::read_dir(&invalid).unwrap().count(), 0);
}
