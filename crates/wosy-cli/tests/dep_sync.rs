#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::DocumentMut;
use wosy_project::{load_artifact_manifest, ProjectConfiguration};

const OLD_SOURCE: &[u8] = b"%%start\r\ni32 value = 10;\r\n%%end\r\n";
const NEW_SOURCE: &[u8] = b"%%start\ni32 value = 20;\n%%end\n";
const LOCAL_SOURCE: &[u8] = b"%%start\ni32 value = 30;\n%%end\n";
const GIT_SOURCE: &str = "src/nested/dependency.w";
const ATTRIBUTES: &[u8] = b"**/dependency.w export-ignore\n";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wosy-dep-sync-{}-{time}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn local(&self) -> PathBuf {
        let path = self.0.join("local_lib");
        write_package(&path, "local_lib", "src/nested");
        fs::write(path.join("src/nested/library.w"), LOCAL_SOURCE).unwrap();
        path
    }

    fn origin(&self, package_path: &str, package_name: &str) -> (PathBuf, String) {
        let repository = self.0.join("upstream");
        fs::create_dir(&repository).unwrap();
        git(&repository, &["init", "--quiet", "--template=", "--object-format=sha1"]);
        let package_root = repository.join(package_path);
        write_package(&package_root, package_name, "src");
        fs::create_dir_all(package_root.join("src/nested")).unwrap();
        fs::write(package_root.join(GIT_SOURCE), OLD_SOURCE).unwrap();
        // An archive or checkout-based implementation must not omit tracked source
        // due to attributes or accidentally include a repository sibling.
        fs::write(repository.join(".gitattributes"), ATTRIBUTES).unwrap();
        if package_path != "." {
            fs::write(repository.join("unrelated.txt"), "outside the package").unwrap();
        }
        let revision = commit(&repository, "Initial package source");
        (repository, revision)
    }

    fn project(&self, name: &str, git_name: &str, package_path: &str) -> PathBuf {
        let root = self.0.join(name);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/main.w"),
            b"%%start\nlocal = namespace local_lib \"library.w\";\nremote = namespace remote_lib \"nested/dependency.w\";\ni32 sum = local.value + remote.value;\n%%end\n",
        )
        .unwrap();
        fs::write(
            root.join("wosy.toml"),
            format!(
                "[package]\nname = \"app\"\ndefault_target = \"app\"\n\n\
                [dependencies.local_lib]\npath = \"../local_lib\"\n\n\
                [dependencies.{git_name}]\ngit = \"../upstream\"\npath = \"{package_path}\"\n\n\
                [comments]\ncategories = []\n\
                [builders.fixture]\ncommand = [\"git\", \"version\"]\ntoolchain_identity = \"wosy-dep-sync-test\"\n\
                [runners.fixture]\ncommand = [\"git\", \"version\"]\n\
                [targets.app]\nroot = \"src/main.w\"\n\
                [targets.app.artifacts.main]\nartifact = \"Main\"\n\
                [targets.app.artifacts.main.profiles.dev]\nbackend = \"llvm\"\noutput = \"native\"\nbuilder = \"fixture\"\nrun_runner = \"fixture\"\n\
                [targets.app.profiles.dev]\nmain_artifact = \"main\"\n"
            ),
        )
        .unwrap();
        root
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    let output = command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args([
            "-c", "user.name=Wosy Package Fixture",
            "-c", "user.email=fixture@example.invalid",
            "-c", "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .unwrap();
    expect_success(&output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "--no-verify", "--no-gpg-sign", "-m", message]);
    git(root, &["rev-parse", "HEAD"])
}

fn write_package(root: &Path, name: &str, source_root: &str) {
    fs::create_dir_all(root.join(source_root)).unwrap();
    fs::write(
        root.join("wosy.toml"),
        format!("[package]\nname = \"{name}\"\nsource_root = \"{source_root}\"\n"),
    )
    .unwrap();
}

fn checked_in_lock(project: &Path, declared_name: &str, actual_name: &str, rev: &str, subdir: &str) -> String {
    // Compute the fixture's expected digest from known committed bytes without
    // calling the package implementation or using its installed files.
    let package = format!("[package]\nname = \"{actual_name}\"\nsource_root = \"src\"\n");
    let mut paths: Vec<(&str, &[u8])> = Vec::new();
    if subdir == "." {
        paths.push((".gitattributes", ATTRIBUTES));
    }
    paths.extend([(GIT_SOURCE, OLD_SOURCE), ("wosy.toml", package.as_bytes())]);
    let mut hash = blake3::Hasher::new();
    hash.update(b"wosy-package-source-v1\0");
    for (name, content) in paths {
        hash.update(&(name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update(blake3::hash(content).as_bytes());
    }
    let lock = format!(
        "version = 1\n\n[packages.{declared_name}]\ngit = \"../upstream\"\nrev = \"{rev}\"\n\
         path = \"{subdir}\"\nsource_hash = \"{}\"\n",
        hash.finalize().to_hex()
    );
    fs::write(project.join("wosy.lock"), &lock).unwrap();
    lock
}

fn wosy(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wosy"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn expect_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn expect_error(output: &Output, needle: &str) {
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(error.contains(needle), "expected {needle:?} in {error:?}");
}

fn stub_artifact(project: &Path) -> PathBuf {
    let artifact = project.join(".wosy/artifacts/app/dev/main");
    fs::create_dir_all(&artifact).unwrap();
    fs::write(artifact.join("main"), "fixture builder stand-in").unwrap();
    artifact
}

#[test]
fn dep_sync_materializes_the_exact_subpackage_and_keeps_the_local_path() {
    let fixture = Fixture::new();
    let local = fixture.local();
    let package_path = "packages/remote_lib";
    let (origin, revision) = fixture.origin(package_path, "remote_lib");
    let project = fixture.project("clean", "remote_lib", package_path);
    expect_error(&wosy(&project, &["dep", "sync"]), "require wosy.lock");
    assert!(!project.join("deps").exists());
    let locked = checked_in_lock(&project, "remote_lib", "remote_lib", &revision, package_path);
    expect_error(&wosy(&project, &["build"]), "run wosy dep sync");
    fs::write(origin.join(package_path).join(GIT_SOURCE), NEW_SOURCE).unwrap();
    let newer = commit(&origin, "New source on the branch");
    assert_ne!(revision, newer);

    expect_success(&wosy(&project.join("src"), &["dep", "sync"]));
    assert_eq!(fs::read_to_string(project.join("wosy.lock")).unwrap(), locked);
    assert!(locked.starts_with("version = 1\n\n[packages.remote_lib]\n"));
    assert!(locked.contains(&format!("rev = \"{revision}\"")));
    assert!(locked.contains("git = \"../upstream\""));
    assert!(locked.contains(&format!("path = \"{package_path}\"")));
    assert!(!locked.contains("local_lib"));
    let checkout = project.join("deps/remote_lib");
    assert_eq!(fs::read(checkout.join(GIT_SOURCE)).unwrap(), OLD_SOURCE);
    assert!(!checkout.join(".git").exists());
    assert!(!checkout.join("unrelated.txt").exists());
    assert!(!project.join("deps/local_lib").exists());
    assert_eq!(fs::read(local.join("src/nested/library.w")).unwrap(), LOCAL_SOURCE);
    let config = ProjectConfiguration::load(&project).unwrap();
    assert_eq!(
        config.resolved_packages["local_lib"].source_root,
        project.join("../local_lib/src/nested")
    );
    assert_eq!(config.resolved_packages["remote_lib"].source_root, checkout.join("src"));
    expect_success(&wosy(&project, &["parse"]));

    let twin = fixture.project("other_clean", "remote_lib", package_path);
    fs::write(twin.join("wosy.lock"), &locked).unwrap();
    expect_success(&wosy(&twin, &["dep", "sync"]));
    assert_eq!(fs::read_to_string(twin.join("wosy.lock")).unwrap(), locked);
    fs::remove_dir_all(&checkout).unwrap();
    expect_success(&wosy(&project, &["dep", "sync"]));
    assert_eq!(fs::read(checkout.join(GIT_SOURCE)).unwrap(), OLD_SOURCE);
    let offline = origin.with_file_name("unavailable_upstream");
    fs::rename(&origin, offline).unwrap();
    expect_success(&wosy(&project, &["dep", "sync"]));
    assert_eq!(fs::read_to_string(project.join("wosy.lock")).unwrap(), locked);
}

#[test]
fn dep_sync_refuses_edited_trees_bad_hashes_and_disagreeing_pins() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let project = fixture.project("clean", "remote_lib", ".");
    let original_lock = checked_in_lock(&project, "remote_lib", "remote_lib", &revision, ".");
    expect_success(&wosy(&project, &["dep", "sync"]));
    let lock_path = project.join("wosy.lock");
    let checkout = project.join("deps/remote_lib");

    // The builder/runner stub only verifies Wosy's existing compile and artifact
    // routing: no external compiler, executable execution, or tool install.
    let artifact = stub_artifact(&project);
    expect_success(&wosy(&project, &["build"]));
    expect_success(&wosy(&project, &["run"]));

    fs::write(checkout.join(GIT_SOURCE), NEW_SOURCE).unwrap();
    expect_error(&wosy(&project, &["dep", "sync"]), "installed tree differs");
    expect_success(&wosy(&project, &["parse"]));
    expect_error(&wosy(&project, &["run"]), "artifact manifest is stale");
    expect_success(&wosy(&project, &["build"]));
    let manifest = load_artifact_manifest(&artifact.join("artifact-manifest.json")).unwrap();
    let remote = manifest.source_observations.iter()
        .find(|entry| entry.source.package == "remote_lib").unwrap();
    assert_eq!(remote.content_revision.0, blake3::hash(NEW_SOURCE).to_hex().as_str());
    assert!(manifest.source_observations.iter().any(|entry| entry.source.package == "local_lib"));
    expect_success(&wosy(&project, &["run"]));
    assert_eq!(fs::read(checkout.join(GIT_SOURCE)).unwrap(), NEW_SOURCE);
    assert_eq!(fs::read_to_string(&lock_path).unwrap(), original_lock);

    let manifest_path = project.join("wosy.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    fs::write(&manifest_path, manifest.replace("../upstream", "../another")).unwrap();
    expect_error(&wosy(&project, &["dep", "sync"]), "disagrees with wosy.lock");
    fs::write(manifest_path, manifest).unwrap();

    // The content hash alone cannot authorize a different revision.
    fs::write(&lock_path, original_lock.replace(&revision, &"0".repeat(40))).unwrap();
    expect_error(&wosy(&project, &["dep", "sync"]), "not verified for its wosy.lock pin");
    expect_error(&wosy(&project, &["build"]), "not verified for its wosy.lock pin");
    fs::remove_dir_all(&checkout).unwrap();
    let hash_line = original_lock.lines().find(|line| line.starts_with("source_hash")).unwrap();
    let corrupt_lock = original_lock.replace(hash_line, &format!("source_hash = \"{}\"", "0".repeat(64)));
    assert_ne!(corrupt_lock, original_lock);
    fs::write(&lock_path, &corrupt_lock).unwrap();
    expect_error(&wosy(&project, &["dep", "sync"]), "source hash mismatch");
    assert!(!checkout.exists());
    assert_eq!(fs::read_to_string(&lock_path).unwrap(), corrupt_lock);
}

#[test]
fn dep_sync_rejects_other_package_names_and_unsupported_patches() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let wrong_name = fixture.project("wrong_name", "other_lib", ".");
    let lock = checked_in_lock(&wrong_name, "other_lib", "remote_lib", &revision, ".");
    expect_error(&wosy(&wrong_name, &["dep", "sync"]), "package name is remote_lib");
    assert_eq!(fs::read_to_string(wrong_name.join("wosy.lock")).unwrap(), lock);
    assert!(!wrong_name.join("deps").exists());

    let with_patches = fixture.project("with_patches", "remote_lib", ".");
    let lock = checked_in_lock(&with_patches, "remote_lib", "remote_lib", &revision, ".");
    let patches = with_patches.join("patches/remote_lib");
    fs::create_dir_all(&patches).unwrap();
    fs::write(patches.join("pending.patch"), "not yet supported").unwrap();
    expect_error(&wosy(&with_patches, &["dep", "sync"]), "does not apply patches");
    assert_eq!(fs::read_to_string(with_patches.join("wosy.lock")).unwrap(), lock);
    assert!(!with_patches.join("deps").exists());
}

#[cfg(unix)]
#[test]
fn dep_sync_and_project_loading_never_follow_a_symlinked_checkout() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let project = fixture.project("clean", "remote_lib", ".");
    checked_in_lock(&project, "remote_lib", "remote_lib", &revision, ".");
    expect_success(&wosy(&project, &["dep", "sync"]));
    let checkout = project.join("deps/remote_lib");
    let outside = fixture.0.join("outside");
    fs::rename(&checkout, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &checkout).unwrap();

    expect_error(&wosy(&project, &["dep", "sync"]), "not a symlink or file");
    expect_error(&wosy(&project, &["build"]), "not a symlink or file");
    assert_eq!(fs::read(outside.join(GIT_SOURCE)).unwrap(), OLD_SOURCE);
}

#[test]
fn dep_sync_receipt_binds_the_context_and_the_exact_checkout() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let project = fixture.project("clean", "remote_lib", ".");
    let lock = checked_in_lock(&project, "remote_lib", "remote_lib", &revision, ".");
    expect_success(&wosy(&project, &["dep", "sync"]));
    let checkout = project.join("deps/remote_lib");
    let original = fixture.0.join("verified_directory");
    fs::rename(&checkout, &original).unwrap();
    write_package(&checkout, "remote_lib", "src/nested");
    fs::write(checkout.join(GIT_SOURCE), OLD_SOURCE).unwrap();
    expect_error(&wosy(&project, &["build"]), "not verified for its wosy.lock pin");
    expect_error(&wosy(&project, &["dep", "sync"]), "not verified for its wosy.lock pin");

    // Move the exact verified directory, not a copy, to another project with
    // the same declarations and lock. Even a matching filesystem ID is not
    // enough to trust a receipt copied across projects.
    let twin = fixture.project("another_project", "remote_lib", ".");
    fs::write(twin.join("wosy.lock"), lock).unwrap();
    fs::create_dir_all(twin.join("deps/.wosy-pins")).unwrap();
    fs::rename(original, twin.join("deps/remote_lib")).unwrap();
    fs::copy(
        project.join("deps/.wosy-pins/remote_lib"),
        twin.join("deps/.wosy-pins/remote_lib"),
    )
    .unwrap();
    expect_error(&wosy(&twin, &["build"]), "not verified for its wosy.lock pin");
    expect_error(&wosy(&twin, &["dep", "sync"]), "not verified for its wosy.lock pin");
}

#[test]
fn dep_sync_does_not_expose_a_checkout_if_its_receipt_cannot_be_published() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let project = fixture.project("clean", "remote_lib", ".");
    let lock = checked_in_lock(&project, "remote_lib", "remote_lib", &revision, ".");
    let receipt_obstruction = project.join("deps/.wosy-pins/remote_lib");
    fs::create_dir_all(&receipt_obstruction).unwrap();

    expect_error(&wosy(&project, &["dep", "sync"]), "cannot publish provenance");
    assert!(!project.join("deps/remote_lib").exists());
    assert!(receipt_obstruction.is_dir());
    assert_eq!(fs::read_to_string(project.join("wosy.lock")).unwrap(), lock);
}

#[cfg(unix)]
#[test]
fn build_and_run_refuse_symlinked_files_and_nested_git_source_ancestors() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let project = fixture.project("clean", "remote_lib", ".");
    checked_in_lock(&project, "remote_lib", "remote_lib", &revision, ".");
    expect_success(&wosy(&project, &["dep", "sync"]));
    stub_artifact(&project);
    expect_success(&wosy(&project, &["build"]));
    let checkout = project.join("deps/remote_lib");
    let directory = checkout.join("src/nested");
    let external_dir = fixture.0.join("outside_package");
    fs::rename(&directory, &external_dir).unwrap();
    std::os::unix::fs::symlink(&external_dir, &directory).unwrap();
    expect_error(&wosy(&project, &["build"]), "must not traverse a symlink");
    expect_error(&wosy(&project, &["run"]), "must not traverse a symlink");

    // A final-component link with the same bytes would evade freshness checks
    // that simply read through a symlink.
    fs::remove_file(&directory).unwrap();
    fs::rename(external_dir, directory).unwrap();
    let file = checkout.join(GIT_SOURCE);
    let external_file = fixture.0.join("outside_source.w");
    fs::rename(&file, &external_file).unwrap();
    std::os::unix::fs::symlink(external_file, file).unwrap();
    expect_error(&wosy(&project, &["build"]), "must not traverse a symlink");
    expect_error(&wosy(&project, &["run"]), "must not traverse a symlink");
}

#[test]
fn changing_an_editable_git_source_root_invalidates_the_artifact() {
    let fixture = Fixture::new();
    fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    let project = fixture.project("clean", "remote_lib", ".");
    checked_in_lock(&project, "remote_lib", "remote_lib", &revision, ".");
    expect_success(&wosy(&project, &["dep", "sync"]));
    let artifact = stub_artifact(&project).join("artifact-manifest.json");
    expect_success(&wosy(&project, &["build"]));
    expect_success(&wosy(&project, &["run"]));
    let old = load_artifact_manifest(&artifact).unwrap();

    let checkout = project.join("deps/remote_lib");
    let alternate = checkout.join("alternate/nested");
    fs::create_dir_all(&alternate).unwrap();
    fs::write(alternate.join("dependency.w"), OLD_SOURCE).unwrap();
    write_package(&checkout, "remote_lib", "alternate");
    assert_eq!(fs::read(checkout.join(GIT_SOURCE)).unwrap(), OLD_SOURCE);
    expect_error(&wosy(&project, &["run"]), "artifact manifest is stale");

    expect_success(&wosy(&project, &["build"]));
    let rebuilt = load_artifact_manifest(&artifact).unwrap();
    assert_ne!(old.identity.configuration_identity, rebuilt.identity.configuration_identity);
    assert!(rebuilt.source_observations.iter().any(|entry| {
        entry.source.package == "remote_lib"
            && entry.source.path.as_path() == Path::new("alternate/nested/dependency.w")
    }));
    expect_success(&wosy(&project, &["run"]));
}

fn append_manifest(path: &Path, text: &str) {
    let path = path.join("wosy.toml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(path, original + text).unwrap();
}

fn read_toml(path: &Path) -> DocumentMut {
    fs::read_to_string(path).unwrap().parse().unwrap()
}

fn without_git_declaration(project: &Path) {
    let path = project.join("wosy.toml");
    let mut manifest = read_toml(&path);
    manifest["dependencies"].as_table_mut().unwrap().remove("remote_lib");
    fs::write(path, manifest.to_string()).unwrap();
}

fn make_monorepo(fixture: &Fixture) -> (PathBuf, String) {
    let (repo, _) = fixture.origin("packages/remote_lib", "remote_lib");
    let remote = repo.join("packages/remote_lib");
    let leaf = repo.join("packages/leaf_lib");
    write_package(&leaf, "leaf_lib", "src");
    fs::write(leaf.join("src/library.w"), b"%%start\ni32 value = 51;\n%%end\n").unwrap();
    append_manifest(&remote, "\n[dependencies.leaf_lib]\npath = \"../leaf_lib\"\n");
    fs::write(
        remote.join(GIT_SOURCE),
        b"%%start\nleaf = namespace leaf_lib \"library.w\";\ni32 value = leaf.value;\n%%end\n",
    ).unwrap();
    let revision = commit(&repo, "Two Wosy subpackages in a single repository");
    (repo, revision)
}

#[test]
fn dep_add_and_sync_resolve_the_same_transitive_monorepo_graph() {
    let fixture = Fixture::new();
    fixture.local();
    let (repo, revision) = make_monorepo(&fixture);
    let project = fixture.project("clean", "remote_lib", "packages/remote_lib");
    without_git_declaration(&project);
    expect_success(&wosy(&project, &[
        "dep", "add", "../upstream", "--path", "packages/remote_lib/wosy.toml",
    ]));
    let manifest_path = project.join("wosy.toml");
    let manifest = read_toml(&manifest_path);
    let direct = manifest["dependencies"].as_table().unwrap();
    assert!(direct.contains_key("remote_lib"));
    assert!(!direct.contains_key("leaf_lib"));
    let lock_path = project.join("wosy.lock");
    let lock = fs::read_to_string(&lock_path).unwrap();
    let document: DocumentMut = lock.parse().unwrap();
    let pins = document["packages"].as_table().unwrap();
    let names = pins.iter().map(|(name, _)| name).collect::<Vec<_>>();
    assert_eq!(names, ["leaf_lib", "remote_lib"]);
    for (_, pin) in pins.iter() {
        assert_eq!(pin["rev"].as_str(), Some(revision.as_str()));
    }
    assert!(!project.join("deps").exists());
    expect_success(&wosy(&project, &["parse"]));
    expect_error(&wosy(&project, &["build"]), "run wosy dep sync");
    assert!(!project.join("deps").exists());

    let leaf = repo.join("packages/leaf_lib/src/library.w");
    fs::write(leaf, b"%%start\ni32 value = 999;\n%%end\n").unwrap();
    let later = commit(&repo, "A newer unpinned leaf");
    assert_ne!(revision, later);
    expect_success(&wosy(&project, &["dep", "sync"]));
    assert_eq!(fs::read_to_string(&lock_path).unwrap(), lock);
    assert_eq!(
        fs::read(project.join("deps/leaf_lib/src/library.w")).unwrap(),
        b"%%start\ni32 value = 51;\n%%end\n"
    );
    assert!(!project.join("deps/remote_lib/packages/leaf_lib").exists());

    // Both a local package and the Git package select the same locked leaf.
    // A single binding and source observation satisfy the diamond.
    append_manifest(&fixture.0.join("local_lib"),
        "\n[dependencies.leaf_lib]\ngit = \"../upstream\"\npath = \"packages/leaf_lib\"\n");
    fs::write(
        fixture.0.join("local_lib/src/nested/library.w"),
        b"%%start\nleaf = namespace leaf_lib \"library.w\";\ni32 value = leaf.value;\n%%end\n",
    ).unwrap();
    let artifact = stub_artifact(&project).join("artifact-manifest.json");
    expect_success(&wosy(&project, &["build"]));
    let built = load_artifact_manifest(&artifact).unwrap();
    assert_eq!(built.source_observations.iter().filter(|entry| entry.source.package == "leaf_lib").count(), 1);
    let offline = repo.with_file_name("unavailable_upstream");
    fs::rename(&repo, &offline).unwrap();
    expect_success(&wosy(&project, &["run"]));
    expect_success(&wosy(&project, &["dep", "sync"]));
    // Promoting a transitive Git package should not resolve its moving HEAD.
    expect_success(&wosy(&project, &[
        "dep", "add", "../upstream", "--path", "packages/leaf_lib",
    ]));
    assert_eq!(fs::read_to_string(&lock_path).unwrap(), lock);
    assert!(read_toml(&manifest_path)["dependencies"].as_table().unwrap().contains_key("leaf_lib"));
    let moved = project.with_file_name("moved_project");
    fs::rename(project, &moved).unwrap();
    expect_success(&wosy(&moved, &["dep", "sync"]));
    stub_artifact(&moved);
    expect_success(&wosy(&moved, &["build"]));
}

#[test]
fn adding_a_local_package_locks_git_reached_only_through_it() {
    let fixture = Fixture::new();
    let library = fixture.local();
    let (_, revision) = fixture.origin(".", "remote_lib");
    append_manifest(&library, "\n[dependencies.remote_lib]\ngit = \"../upstream\"\n");
    let project = fixture.project("clean", "remote_lib", ".");
    without_git_declaration(&project);
    let path = project.join("wosy.toml");
    let mut original = read_toml(&path);
    original.as_table_mut().remove("dependencies");
    fs::write(&path, original.to_string()).unwrap();

    expect_success(&wosy(&project, &["dep", "add", "../local_lib"]));
    let manifest = read_toml(&path);
    let direct = manifest["dependencies"].as_table().unwrap();
    assert!(direct.contains_key("local_lib"));
    assert!(!direct.contains_key("remote_lib"));
    let lock = read_toml(&project.join("wosy.lock"));
    assert_eq!(lock["packages"]["remote_lib"]["rev"].as_str(), Some(revision.as_str()));
    expect_success(&wosy(&project, &["dep", "sync"]));
    assert!(!project.join("deps/local_lib").exists());
    assert!(project.join("deps/remote_lib").is_dir());
    assert!(library.join("src/nested/library.w").is_file());
    stub_artifact(&project);
    expect_success(&wosy(&project, &["build"]));
}

#[test]
fn local_resolution_rejects_conflicts_and_cycles_before_publish() {
    let fixture = Fixture::new();
    let library = fixture.local();
    let other = fixture.0.join("other_lib");
    let shared = fixture.0.join("shared_lib");
    let replacement = fixture.0.join("alternative_shared");
    write_package(&other, "other_lib", "src");
    write_package(&shared, "shared_lib", "src");
    write_package(&replacement, "shared_lib", "src");
    append_manifest(&library, "\n[dependencies.shared_lib]\npath = \"../shared_lib\"\n");
    append_manifest(&other, "\n[dependencies.shared_lib]\npath = \"../alternative_shared\"\n");
    let project = fixture.project("clean", "remote_lib", ".");
    without_git_declaration(&project);
    let manifest_path = project.join("wosy.toml");
    let before = fs::read_to_string(&manifest_path).unwrap();
    expect_error(&wosy(&project, &["dep", "add", "../other_lib"]), "conflicting package bindings for shared_lib");
    assert_eq!(fs::read_to_string(&manifest_path).unwrap(), before);
    assert!(!project.join("wosy.lock").exists());
    assert!(!project.join("deps").exists());

    append_manifest(&shared, "\n[dependencies.local_lib]\npath = \"../local_lib\"\n");
    expect_success(&wosy(&project, &["parse"]));
    expect_error(&wosy(&project, &["build"]), "app -> local_lib -> shared_lib -> local_lib");
    assert!(!project.join("deps").exists());
}

#[test]
fn sync_rejects_missing_and_orphan_transitive_pins_without_exposing_a_tree() {
    let fixture = Fixture::new();
    fixture.local();
    make_monorepo(&fixture);
    let project = fixture.project("clean", "remote_lib", "packages/remote_lib");
    without_git_declaration(&project);
    expect_success(&wosy(&project, &["dep", "add", "../upstream", "--path", "packages/remote_lib"]));
    let lock_path = project.join("wosy.lock");
    let complete = fs::read_to_string(&lock_path).unwrap();
    let mut incomplete: DocumentMut = complete.parse().unwrap();
    incomplete["packages"].as_table_mut().unwrap().remove("leaf_lib");
    fs::write(&lock_path, incomplete.to_string()).unwrap();
    expect_error(&wosy(&project, &["dep", "sync"]), "dependency leaf_lib has no entry in wosy.lock");
    assert!(!project.join("deps").exists());

    let mut stale: DocumentMut = complete.parse().unwrap();
    let extra = stale["packages"]["remote_lib"].clone();
    stale["packages"].as_table_mut().unwrap().insert("unreachable", extra);
    let stale = stale.to_string();
    fs::write(&lock_path, &stale).unwrap();
    expect_error(&wosy(&project, &["dep", "sync"]), "wosy.lock contains undeclared Git dependency unreachable");
    assert_eq!(fs::read_to_string(&lock_path).unwrap(), stale);
    assert!(!project.join("deps").exists());
}

#[test]
fn git_relative_paths_cannot_escape_the_repository_root() {
    let fixture = Fixture::new();
    fixture.local();
    let (repo, _) = fixture.origin("packages/remote_lib", "remote_lib");
    append_manifest(&repo.join("packages/remote_lib"), "\n[dependencies.other_lib]\npath = \"../../../other_lib\"\n");
    commit(&repo, "Declare a filesystem package in a Git manifest");
    let project = fixture.project("clean", "remote_lib", "packages/remote_lib");
    without_git_declaration(&project);
    let before = fs::read_to_string(project.join("wosy.toml")).unwrap();
    assert!(!wosy(&project, &["dep", "add", "../upstream", "--path", "packages/remote_lib"]).status.success());
    assert_eq!(fs::read_to_string(project.join("wosy.toml")).unwrap(), before);
    assert!(!project.join("wosy.lock").exists());
    assert!(!project.join("deps").exists());
}

#[test]
fn repository_relative_dependency_requires_a_valid_git_package_name() {
    let fixture = Fixture::new();
    fixture.local();
    let (repo, _) = fixture.origin("packages/remote_lib", "remote_lib");
    let sibling = repo.join("packages/leaf-lib");
    write_package(&sibling, "leaf-lib", "src");
    fs::write(sibling.join("src/library.w"), LOCAL_SOURCE).unwrap();
    append_manifest(&repo.join("packages/remote_lib"), "\n[dependencies.leaf-lib]\npath = \"../leaf-lib\"\n");
    commit(&repo, "Declare a sibling without a Git package name");
    let project = fixture.project("clean", "remote_lib", "packages/remote_lib");
    without_git_declaration(&project);
    let manifest = project.join("wosy.toml");
    let original = fs::read(&manifest).unwrap();

    assert!(!wosy(&project, &["dep", "add", "../upstream", "--path", "packages/remote_lib"]).status.success());
    assert_eq!(fs::read(&manifest).unwrap(), original);
    assert!(!project.join("wosy.lock").exists());
    assert!(!project.join("deps").exists());
}

#[test]
fn repository_relative_dependency_never_uses_a_host_path_as_fallback() {
    let fixture = Fixture::new();
    fixture.local();
    let (repo, _) = fixture.origin("packages/remote_lib", "remote_lib");
    append_manifest(&repo.join("packages/remote_lib"), "\n[dependencies.decoy]\npath = \"../decoy\"\n");
    commit(&repo, "Declare a repository sibling that is not in the tree");
    let project = fixture.project("clean", "remote_lib", "packages/remote_lib");
    without_git_declaration(&project);
    let decoy = project.join("deps/decoy");
    write_package(&decoy, "decoy", "src");
    fs::write(decoy.join("src/host-only.w"), b"host path must remain independent").unwrap();
    let manifest_path = project.join("wosy.toml");
    let unchanged = fs::read(&manifest_path).unwrap();

    assert!(!wosy(&project, &["dep", "add", "../upstream", "--path", "packages/remote_lib"]).status.success());
    assert!(!project.join("wosy.lock").exists());
    assert!(!project.join("deps/remote_lib").exists());
    assert_eq!(fs::read(manifest_path).unwrap(), unchanged);
    assert_eq!(fs::read(decoy.join("src/host-only.w")).unwrap(), b"host path must remain independent");
}

#[test]
fn a_sibling_pin_cannot_be_advanced_separately_even_if_already_reached() {
    let fixture = Fixture::new();
    fixture.local();
    let (repo, original) = make_monorepo(&fixture);
    let project = fixture.project("selected", "remote_lib", "packages/remote_lib");
    without_git_declaration(&project);
    expect_success(&wosy(&project, &["dep", "add", "../upstream", "--path", "packages/remote_lib"]));
    let lock_path = project.join("wosy.lock");
    let mut lock = read_toml(&lock_path);
    assert_eq!(lock["packages"]["leaf_lib"]["rev"].as_str(), Some(original.as_str()));

    fs::write(repo.join("packages/leaf_lib/src/library.w"), b"%%start\ni32 value = 940;\n%%end\n").unwrap();
    let later = commit(&repo, "A sibling at a newer commit");
    let other = fixture.project("separate", "remote_lib", "packages/remote_lib");
    without_git_declaration(&other);
    expect_success(&wosy(&other, &["dep", "add", "../upstream", "--path", "packages/leaf_lib"]));
    let independent = read_toml(&other.join("wosy.lock"));
    assert_eq!(independent["packages"]["leaf_lib"]["rev"].as_str(), Some(later.as_str()));
    lock["packages"]["leaf_lib"] = independent["packages"]["leaf_lib"].clone();
    fs::write(&lock_path, lock.to_string()).unwrap();

    // First discovery checks the locked sibling constraint before a publish.
    assert!(!wosy(&project, &["dep", "sync"]).status.success());
    assert!(!project.join("deps").exists());

    // Now the explicit leaf edge sorts first and resolves the newer lock;
    // the later relative edge still must constrain that same graph binding.
    append_manifest(&project, "\n[dependencies.leaf_lib]\ngit = \"../upstream\"\npath = \"packages/leaf_lib\"\n");
    assert!(!wosy(&project, &["dep", "sync"]).status.success());
    assert!(!project.join("deps").exists());
    assert_eq!(fs::read_to_string(lock_path).unwrap(), lock.to_string());
}

#[test]
fn sync_follows_local_declarations_to_git_even_without_pins() {
    for old_lock in [None, Some("version = 1\n")] {
        let fixture = Fixture::new();
        let local = fixture.local();
        let project = fixture.project("clean", "remote_lib", ".");
        without_git_declaration(&project);
        // Sync only needs package declarations. There are no local-source
        // requirements unless an ordinary build plans source routing.
        fs::remove_dir_all(local.join("src")).unwrap();
        let lock = project.join("wosy.lock");
        if let Some(text) = old_lock {
            fs::write(&lock, text).unwrap();
        }
        expect_success(&wosy(&project, &["dep", "sync"]));

        append_manifest(&local, "\n[dependencies.remote_lib]\ngit = \"../upstream\"\n");
        assert!(!wosy(&project, &["dep", "sync"]).status.success());
        assert!(!project.join("deps").exists());
        assert_eq!(fs::read_to_string(&lock).ok().as_deref(), old_lock);
    }
}

#[test]
fn a_local_only_add_uses_no_git_or_checkout_staging() {
    let fixture = Fixture::new();
    fixture.local();
    let project = fixture.project("clean", "remote_lib", ".");
    let path = project.join("wosy.toml");
    let mut manifest = read_toml(&path);
    manifest.as_table_mut().remove("dependencies");
    fs::write(&path, manifest.to_string()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_wosy"))
        .current_dir(&project)
        .env_remove("PATH")
        .args(["dep", "add", "../local_lib"])
        .output().unwrap();
    expect_success(&output);
    let actual = read_toml(&path);
    assert_eq!(actual["dependencies"]["local_lib"]["path"].as_str(), Some("../local_lib"));
    assert!(!project.join("deps").exists());
    assert!(!project.join("wosy.lock").exists());
}

#[test]
fn legacy_lock_is_ignored_and_preserved_only_while_the_graph_is_local() {
    for legacy in [b"not a Wosy lock".as_slice(), b"\xff\x00"] {
        let fixture = Fixture::new();
        let local = fixture.local();
        let project = fixture.project("clean", "remote_lib", ".");
        let manifest_path = project.join("wosy.toml");
        let mut document = read_toml(&manifest_path);
        document.as_table_mut().remove("dependencies");
        fs::write(&manifest_path, document.to_string()).unwrap();
        fs::write(
            project.join("src/main.w"),
            b"%%start\nlocal = namespace local_lib \"library.w\";\ni32 value = local.value;\n%%end\n",
        ).unwrap();
        let lock_path = project.join("wosy.lock");
        fs::write(&lock_path, legacy).unwrap();

        expect_success(&wosy(&project, &["dep", "add", "../local_lib"]));
        expect_success(&wosy(&project, &["dep", "sync"]));
        stub_artifact(&project);
        expect_success(&wosy(&project, &["build"]));
        expect_success(&wosy(&project, &["run"]));
        assert_eq!(fs::read(&lock_path).unwrap(), legacy);

        append_manifest(&local, "\n[dependencies.remote_lib]\ngit = \"../upstream\"\n");
        let unrelated = fixture.0.join("unrelated");
        write_package(&unrelated, "unrelated", "src");
        let expected_manifest = fs::read(&manifest_path).unwrap();
        expect_success(&wosy(&project, &["parse"]));
        for args in [
            &["dep", "sync"][..],
            &["dep", "add", "../unrelated"][..],
            &["build"][..],
            &["run"][..],
        ] {
            assert!(!wosy(&project, args).status.success());
        }
        assert_eq!(fs::read(&manifest_path).unwrap(), expected_manifest);
        assert_eq!(fs::read(&lock_path).unwrap(), legacy);
        assert!(!project.join("deps").exists());
    }
}

#[test]
fn a_valid_managed_lock_does_not_hide_orphans_in_a_local_graph() {
    let fixture = Fixture::new();
    fixture.local();
    let unrelated = fixture.0.join("unrelated");
    write_package(&unrelated, "unrelated", "src");
    let project = fixture.project("clean", "remote_lib", ".");
    without_git_declaration(&project);
    fs::write(
        project.join("src/main.w"),
        b"%%start\nlocal = namespace local_lib \"library.w\";\ni32 value = local.value;\n%%end\n",
    ).unwrap();
    stub_artifact(&project);
    expect_success(&wosy(&project, &["build"]));
    let lock = checked_in_lock(
        &project, "remote_lib", "remote_lib",
        "0123456789abcdef0123456789abcdef01234567", ".",
    );
    let expected_manifest = fs::read(project.join("wosy.toml")).unwrap();
    for args in [
        &["dep", "sync"][..],
        &["dep", "add", "../unrelated"][..],
        &["build"][..],
        &["run"][..],
    ] {
        assert!(!wosy(&project, args).status.success());
    }
    assert_eq!(fs::read_to_string(project.join("wosy.lock")).unwrap(), lock);
    assert_eq!(fs::read(project.join("wosy.toml")).unwrap(), expected_manifest);
    assert!(!project.join("deps").exists());
}

#[test]
fn local_add_preserves_a_linked_manifest_and_its_target() {
    let fixture = Fixture::new();
    fixture.local();
    let project = fixture.project("clean", "remote_lib", ".");
    let manifest = project.join("wosy.toml");
    let mut document = read_toml(&manifest);
    document.as_table_mut().remove("dependencies");
    let original = document.to_string();
    let actual = fixture.0.join("shared-manifest.toml");
    fs::write(&actual, &original).unwrap();
    fs::remove_file(&manifest).unwrap();
    std::os::unix::fs::symlink(&actual, &manifest).unwrap();
    fs::write(project.join("src/main.w"), b"%%start\ni32 value = 1;\n%%end\n").unwrap();

    expect_success(&wosy(&project, &["parse"]));
    expect_success(&wosy(&project, &["dep", "sync"]));
    stub_artifact(&project);
    expect_success(&wosy(&project, &["build"]));
    assert!(!wosy(&project, &["dep", "add", "../local_lib"]).status.success());
    assert_eq!(fs::read_link(&manifest).unwrap(), actual);
    assert_eq!(fs::read_to_string(actual).unwrap(), original);
    assert!(!project.join("wosy.lock").exists());
}
