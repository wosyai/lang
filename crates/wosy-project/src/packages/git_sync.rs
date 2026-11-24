use super::{git_wire::{self, GitOid}, graph, load_git, load_local_declarations, GitDirectory, GitProject, LockedInputId, LockedPackage, ProjectSnapshot, DEPS};
use crate::{project_dependencies, scalar_string, GitSource};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::Item;

const PINS: &str = ".wosy-pins";

pub fn sync_dependencies(root: &Path) -> Result<(), String> {
    let snapshot = ProjectSnapshot::sync(root)?;
    let document = toml_edit::ImDocument::parse(snapshot.manifest()?).map_err(|error| error.to_string())?;
    let package = document.get("package").and_then(Item::as_table)
        .ok_or_else(|| "package is missing".to_owned())?;
    let package = scalar_string(package, "name")?;
    let dependencies = project_dependencies(root, &package, &document)?;
    let mut git = None;
    let mut downloads = Staging::default();
    let resolution = graph::resolve(root, &package, &dependencies, |binding, parent, declaration| {
        let name = declaration.name.as_str();
        match binding {
            graph::Binding::Local(_) => load_local_declarations(parent, declaration),
            graph::Binding::Git(requirement) => {
                let pin = snapshot.pin_for(name, requirement)?;
                if git.is_none() {
                    git = Some(snapshot.git_project()?);
                }
                let git = git.as_ref().expect("opened Git project");
                let checkout = if let Some(checkout) = git.installed(name, pin)? {
                    if source_hash(&checkout)? != pin.source_hash {
                        return Err(format!(
                            "dependency {name} installed tree differs from its pinned source; dep sync did not overwrite {}",
                            git.root.join(DEPS).join(name).display()
                        ));
                    }
                    checkout
                } else {
                    downloads.install_candidate(git, name, pin)?
                };
                load_git(root, declaration, checkout, pin, None)
            }
        }
    })?;
    snapshot.check_reached(&resolution.git_packages)?;
    if let Some(git) = &git {
        downloads.publish(git)?;
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct Staging {
    stage: Option<Stage>,
    pending: Vec<PendingInstall>,
    tips: BTreeMap<String, (PathBuf, GitOid)>,
    exact: BTreeMap<(String, String), PathBuf>,
}

impl Staging {
    fn install_candidate(
        &mut self,
        git: &GitProject,
        name: &str,
        pin: &LockedPackage,
    ) -> Result<GitDirectory, String> {
        let checkout = self.fetch(git, name, &pin.source, &pin.revision)?;
        let actual = source_hash(&checkout)?;
        if pin.source_hash != actual {
            return Err(format!(
                "dependency {name} source hash mismatch: expected {}, got {actual}", pin.source_hash,
            ));
        }
        let staging = self.stage.as_ref().expect("prepared checkout staging");
        let receipt = staging.path.join(DEPS).join(PINS).join(name);
        fs::create_dir_all(receipt.parent().expect("receipt staging parent"))
            .map_err(|error| format!("cannot create provenance staging directory: {error}"))?;
        let expected_receipt = InstallEvidence::new(&git.directory, &checkout, name, pin).receipt();
        fs::write(&receipt, &expected_receipt)
            .map_err(|error| format!("cannot stage provenance for {name}: {error}"))?;
        self.pending.push(PendingInstall {
            name: name.to_owned(),
            expected_checkout: checkout.clone(),
            expected_receipt,
            expected_source_hash: pin.source_hash.clone(),
            destination: git.root.join(DEPS).join(name),
            previous_receipt: false,
            receipt_published: false,
            checkout_published: false,
        });
        Ok(checkout)
    }

    pub(super) fn workspace(&mut self, git: &GitProject) -> Result<&Stage, String> {
        get_stage(&mut self.stage, &git.root, &git.directory)
    }

    pub(super) fn fetch(
        &mut self,
        git: &GitProject,
        slot: &str,
        source: &GitSource,
        revision: &GitOid,
    ) -> Result<GitDirectory, String> {
        let (checkout, unused_repository, stage) = {
            let staging = self.workspace(git)?;
            let (checkout, repository) = staging.git_paths(slot)?;
            (checkout, repository, staging.handle.clone())
        };
        let key = (source.origin.clone(), revision.as_str().to_owned());
        let repository = if let Some(repository) = self.exact.get(&key) {
            repository.clone()
        } else {
            fetch_exact(&git.directory, &stage, &unused_repository, source, revision)?;
            self.exact.insert(key, unused_repository.clone());
            unused_repository
        };
        materialize_tree(&git.directory, &stage, &repository, &checkout, source, revision)?;
        self.stage.as_ref().expect("prepared checkout staging").checkout(slot)
    }

    pub(super) fn fetch_tip(
        &mut self,
        git: &GitProject,
        slot: &str,
        source: &GitSource,
    ) -> Result<(GitDirectory, GitOid), String> {
        let (checkout, unused_repository, stage) = {
            let staging = self.workspace(git)?;
            let (checkout, repository) = staging.git_paths(slot)?;
            (checkout, repository, staging.handle.clone())
        };
        let (repository, revision) = if let Some(existing) = self.tips.get(&source.origin) {
            existing.clone()
        } else {
            checked_output(
                git_command(&git.directory, &stage, None)?
                    .args(["clone", "--quiet", "--bare", "--single-branch", "--depth=1", "--no-tags", "--no-local", "--template=", "--"])
                    .arg(&source.origin)
                    .arg(&unused_repository),
                "clone",
            )?;
            let revision = read_commit(&git.directory, &stage, &unused_repository, "HEAD^{commit}")?;
            let resolved = (unused_repository, revision);
            self.exact.insert((source.origin.clone(), resolved.1.as_str().to_owned()), resolved.0.clone());
            self.tips.insert(source.origin.clone(), resolved.clone());
            resolved
        };
        materialize_tree(&git.directory, &stage, &repository, &checkout, source, &revision)?;
        let staged = self.stage.as_ref().expect("prepared checkout staging").checkout(slot)?;
        Ok((staged, revision))
    }

    fn publish(self, git: &GitProject) -> Result<(), String> {
        publish_staging(&git.directory, self.stage, self.pending)
    }
}

fn publish_staging(
    project: &GitDirectory,
    stage: Option<Stage>,
    mut new_packages: Vec<PendingInstall>,
) -> Result<(), String> {
    // Validate every package before any publish. A clean project gets the
    // complete dependency tree and its receipts from a single rename.
    let Some(staging) = stage else {
        return Ok(());
    };
    let stage_handle = &staging.handle;
    let stage_deps = stage_handle.directory(Path::new(DEPS))?
        .ok_or_else(|| "staged deps disappeared".to_owned())?;
    let dep_name = Path::new(DEPS);
    let Some(deps) = project.directory(dep_name)? else {
        verify_staged(&stage_deps, &new_packages)?;
        stage_handle.rename(dep_name, project, dep_name)
            .map_err(|error| format!("cannot publish {DEPS}: {error}"))?;
        return Ok(());
    };
    publish_updates(
        project, dep_name, &deps, stage_handle, &stage_deps, &staging,
        &mut new_packages, GitDirectory::rename,
    )
}

fn verify_staged(deps: &GitDirectory, pending: &[PendingInstall]) -> Result<(), String> {
    let receipts = deps.directory(Path::new(PINS))?
        .ok_or_else(|| "staged provenance directory disappeared".to_owned())?;
    for install in pending {
        let name = Path::new(&install.name);
        if deps.directory(name)?.as_ref() != Some(&install.expected_checkout)
            || receipts.read_text(name)?.as_deref() != Some(install.expected_receipt.as_str())
        {
            return Err(format!("staged dependency {} changed during publication", install.name));
        }
    }
    Ok(())
}

struct PendingInstall {
    name: String,
    expected_checkout: GitDirectory,
    expected_receipt: String,
    expected_source_hash: String,
    destination: PathBuf,
    previous_receipt: bool,
    receipt_published: bool,
    checkout_published: bool,
}

fn validate_receipt_destination(directory: &GitDirectory, name: &Path) -> Result<bool, String> {
    directory.source(name).map(|file| file.is_some())
        .map_err(|error| format!("cannot publish provenance at {}: {error}", name.display()))
}

fn publish_updates(
    project: &GitDirectory,
    dep_name: &Path,
    deps: &GitDirectory,
    stage: &GitDirectory,
    stage_deps: &GitDirectory,
    staging: &Stage,
    pending: &mut [PendingInstall],
    mut rename: impl FnMut(&GitDirectory, &Path, &GitDirectory, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    verify_staged(stage_deps, pending)?;
    let pins_name = Path::new(PINS);
    if deps.directory(pins_name)?.is_none() {
        deps.mkdir(pins_name)
            .map_err(|error| format!("cannot create installed provenance directory: {error}"))?;
    }
    let receipts = deps.directory(pins_name)?
        .ok_or_else(|| "installed provenance directory disappeared".to_owned())?;
    let staged_receipts = stage_deps.directory(pins_name)?
        .ok_or_else(|| "staged provenance directory disappeared".to_owned())?;
    let preexisting_receipts = pending.iter().map(|install| {
        validate_receipt_destination(&receipts, Path::new(&install.name))
    }).collect::<Result<Vec<_>, _>>()?;
    let previous_name = Path::new("previous-receipts");
    stage.mkdir(previous_name)
        .map_err(|error| format!("cannot stage previous dependency provenance: {error}"))?;
    let previous = stage.directory(previous_name)?
        .ok_or_else(|| "previous provenance staging directory disappeared".to_owned())?;

    let publish = (|| {
        // Checkouts become visible only once their exact receipts are ready.
        for (install, preexisting) in pending.iter_mut().zip(preexisting_receipts) {
            let name = Path::new(&install.name);
            if preexisting {
                rename(&receipts, name, &previous, name).map_err(|error| {
                    format!("cannot preserve previous provenance for {}: {error}", install.name)
                })?;
                install.previous_receipt = true;
            }
            rename(&staged_receipts, name, &receipts, name).map_err(|error| {
                format!("cannot publish provenance for {}: {error}", install.name)
            })?;
            install.receipt_published = true;
        }
        for install in pending.iter_mut() {
            let name = Path::new(&install.name);
            rename(stage_deps, name, deps, name).map_err(|error| {
                format!("cannot publish {}: {error}", install.destination.display())
            })?;
            install.checkout_published = true;

        }
        if project.directory(dep_name)?.as_ref() != Some(deps)
            || deps.directory(pins_name)?.as_ref() != Some(&receipts)
        {
            return Err("installed dependency or provenance directory changed during publication".to_owned());
        }
        Ok(())
    })();
    let Err(cause) = publish else { return Ok(()); };

    let mut rollback_errors = Vec::new();
    for install in pending.iter().rev() {
        let name = Path::new(&install.name);
        if install.checkout_published {
            // Only roll back this sync's own newly published checkout, not a
            // path that was subsequently replaced or edited by an external process.
            let unchanged = deps.directory(name).ok().flatten().is_some_and(|checkout| {
                checkout == install.expected_checkout
                    && source_hash(&checkout).is_ok_and(|hash| hash == install.expected_source_hash)
            });
            if !unchanged {
                rollback_errors.push(format!("{} changed before rollback and was left in place", install.destination.display()));
                continue;
            }
            if let Err(error) = rename(deps, name, stage_deps, name) {
                rollback_errors.push(format!("could not move {} back to staging: {error}", install.destination.display()));
                // Its matching receipt should remain usable.
                continue;
            }
        }
        if install.receipt_published {
            if let Err(error) = rename(&receipts, name, &staged_receipts, name) {
                rollback_errors.push(format!("could not move new provenance for {} back to staging: {error}", install.name));
                continue;
            }
        }
        if install.previous_receipt {
            if let Err(error) = rename(&previous, name, &receipts, name) {
                rollback_errors.push(format!("could not restore previous provenance for {}: {error}", install.name));
            }
        }
    }
    if rollback_errors.is_empty() {
        Err(cause)
    } else {
        staging.preserve.set(true);
        Err(format!("{cause}; rollback incomplete: {}; files left in {}", rollback_errors.join("; "), staging.display.display()))
    }
}

pub(super) fn reject_unsupported_patches(project: &GitDirectory, name: &str) -> Result<(), String> {
    let path = Path::new("patches").join(name);
    if let Some(patches) = project.directory(&path)? {
        let files = patches.entries().map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if !files.is_empty() {
            return Err(format!("dependency {name} has patches; this version of dep sync does not apply patches"));
        }
    }
    Ok(())
}

pub(super) fn verify_installed(
    root: &Path,
    project: &GitDirectory,
    name: &str,
    pin: &LockedPackage,
) -> Result<Option<GitDirectory>, String> {
    let Some(deps) = project.directory(Path::new(DEPS))? else { return Ok(None); };
    let Some(checkout) = deps.directory(Path::new(name))? else { return Ok(None); };
    let expected = InstallEvidence::new(project, &checkout, name, pin).receipt();
    let receipt = deps.read_text(&Path::new(PINS).join(name))?;
    if receipt.as_deref() != Some(expected.as_str()) {
        return Err(format!(
            "dependency {name} is not verified for its wosy.lock pin; move {} aside and run wosy dep sync",
            root.join(DEPS).join(name).display()
        ));
    }
    Ok(Some(checkout))
}

/// Physical installation evidence is distinct from the portable locked input.
/// Moving the same project and checkout preserves it; copying either does not.
struct InstallEvidence<'a> {
    name: &'a str,
    input: LockedInputId,
    project: &'a [u8],
    checkout: &'a [u8],
}

impl<'a> InstallEvidence<'a> {
    fn new(
        project: &'a GitDirectory,
        checkout: &'a GitDirectory,
        name: &'a str,
        pin: &LockedPackage,
    ) -> Self {
        Self { name, input: pin.input_identity(), project: project.identity(), checkout: checkout.identity() }
    }

    fn receipt(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"wosy-package-install-v1\0");
        for field in [self.name.as_bytes(), self.input.as_bytes(), self.project, self.checkout] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        format!("{}\n", hasher.finalize().to_hex())
    }
}

pub(super) struct Stage {
    parent: GitDirectory,
    pub(super) handle: GitDirectory,
    name: PathBuf,
    pub(super) path: PathBuf,
    pub(super) display: PathBuf,
    pub(super) preserve: Cell<bool>,
}

impl Stage {
    fn git_paths(&self, slot: &str) -> Result<(PathBuf, PathBuf), String> {
        let checkout = self.path.join(DEPS).join(slot);
        let repository = self.path.join("repositories").join(slot);
        fs::create_dir_all(repository.parent().expect("stage repository parent"))
            .map_err(|error| format!("cannot create Git staging directory: {error}"))?;
        Ok((checkout, repository))
    }

    fn checkout(&self, slot: &str) -> Result<GitDirectory, String> {
        self.handle.directory(&Path::new(DEPS).join(slot))?
            .ok_or_else(|| format!("staged dependency {slot} disappeared"))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.preserve.get()
            || self.parent.directory(&self.name).ok().flatten().as_ref() != Some(&self.handle)
        {
            return;
        }
        // Recursive deletion is confined to the originally opened private
        // stage even if the project path has been moved or reused.
        if let Ok(entries) = self.handle.entries() {
            for entry in entries {
                let path = self.path.join(entry);
                match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_dir() => { let _ = fs::remove_dir_all(path); }
                    Ok(_) => { let _ = fs::remove_file(path); }
                    Err(_) => {}
                }
            }
        }
        if self.parent.directory(&self.name).ok().flatten().as_ref() == Some(&self.handle) {
            let _ = self.parent.remove_empty_directory(&self.name);
        }
    }
}

fn get_stage<'a>(
    stage: &'a mut Option<Stage>,
    root: &Path,
    project: &GitDirectory,
) -> Result<&'a Stage, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if stage.is_none() {
        loop {
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = PathBuf::from(format!(".wosy-dep-sync-{}-{serial}", std::process::id()));
            match project.mkdir_private(&name) {
                Ok(()) => {
                    let handle = project.directory(&name)?
                        .ok_or_else(|| "dep sync staging directory disappeared".to_owned())?;
                    let path = handle.descriptor_path()
                        .map_err(|error| format!("cannot access dep sync staging descriptor: {error}"))?;
                    *stage = Some(Stage {
                        parent: project.clone(),
                        handle,
                        path,
                        display: root.join(&name),
                        name,
                        preserve: Cell::new(false),
                    });
                    break;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("cannot create dep sync staging directory: {error}")),
            }
        }
    }
    Ok(stage.as_ref().expect("staging directory exists"))
}

fn git_command(project: &GitDirectory, stage: &GitDirectory, repository: Option<&Path>) -> Result<Command, String> {
    let mut command = Command::new("git");
    let cwd = project.descriptor_path()
        .map_err(|error| format!("cannot access Git project descriptor: {error}"))?;
    stage.inherit_in(&mut command)
        .map_err(|error| format!("cannot pass Git staging descriptor: {error}"))?;
    command
        .current_dir(cwd)
        .env("GIT_ALLOW_PROTOCOL", "file:https:http:ssh:git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .stdin(Stdio::null());
    // These variables can redirect Git away from the temporary repository.
    for key in [
        "GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_NAMESPACE",
        "GIT_QUARANTINE_PATH",
    ] {
        command.env_remove(key);
    }
    if let Some(repository) = repository {
        command.arg("--git-dir").arg(repository);
    }
    Ok(command)
}

fn checked_output(command: &mut Command, stage: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("could not execute git {stage}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {stage} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn read_commit(project: &GitDirectory, stage: &GitDirectory, repository: &Path, reference: &str) -> Result<GitOid, String> {
    let output = checked_output(
        git_command(project, stage, Some(repository))?.args(["rev-parse", "--verify", reference]),
        "rev-parse",
    )?;
    GitOid::from_rev_parse(&output.stdout)
}

fn fetch_exact(
    project: &GitDirectory,
    stage: &GitDirectory,
    repository: &Path,
    source: &GitSource,
    revision: &GitOid,
) -> Result<(), String> {
    let object_format = if revision.as_str().len() == 64 { "--object-format=sha256" } else { "--object-format=sha1" };
    checked_output(
        git_command(project, stage, None)?
            .args(["init", "--quiet", "--bare", "--template=", object_format, "--"])
            .arg(repository),
        "init",
    )?;
    checked_output(
        git_command(project, stage, Some(repository))?
            .args(["fetch", "--quiet", "--no-tags", "--no-recurse-submodules", "--depth=1", "--"])
            .arg(&source.origin)
            .arg(revision.as_str()),
        "fetch",
    )?;
    if read_commit(project, stage, repository, "FETCH_HEAD^{commit}")? != *revision {
        return Err(format!("fetch did not provide the pinned commit {}", revision.as_str()));
    }
    Ok(())
}

fn materialize_tree(
    project: &GitDirectory,
    stage: &GitDirectory,
    repository: &Path,
    checkout: &Path,
    source: &GitSource,
    revision: &GitOid,
) -> Result<(), String> {
    let mut command = git_command(project, stage, Some(repository))?;
    command.args(["--literal-pathspecs", "ls-tree", "-r", "-z", "--full-tree"])
        .arg(revision.as_str()).arg("--");
    let subdirectory = (source.package_path != ".").then(|| Path::new(&source.package_path));
    if let Some(path) = subdirectory {
        command.arg(path);
    }
    let indexed = checked_output(&mut command, "ls-tree")?;
    fs::create_dir_all(checkout)
        .map_err(|error| format!("cannot create package staging directory: {error}"))?;
    // Exact Git blobs bypass checkout filters and archive export exclusions.
    for entry in git_wire::blobs(&indexed.stdout)? {
        let entry = entry?;
        let path = entry.path.as_path();
        let path = match subdirectory {
            Some(prefix) => path.strip_prefix(prefix)
                .ok().filter(|path| !path.as_os_str().is_empty())
                .ok_or_else(|| format!("{} is not a package directory", prefix.display()))?,
            None => path,
        };
        let destination = checkout.join(path);
        fs::create_dir_all(destination.parent().expect("checkout file parent"))
            .map_err(|error| format!("cannot create parent for {}: {error}", path.display()))?;
        let file = OpenOptions::new().write(true).create_new(true).open(&destination)
            .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
        checked_output(
            git_command(project, stage, Some(repository))?
                .args(["cat-file", "blob", entry.oid.as_str()])
                .stdout(Stdio::from(file)),
            "cat-file",
        )?;
        restore_mode(&destination, entry.executable)
            .map_err(|error| format!("cannot set permissions for {}: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn restore_mode(path: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if executable {
        let mut permissions = fs::metadata(path)?.permissions();
        let bits = permissions.mode();
        permissions.set_mode(bits | ((bits >> 2) & 0o111));
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restore_mode(_path: &Path, _executable: bool) -> std::io::Result<()> {
    Ok(())
}

pub(super) fn source_hash(root: &GitDirectory) -> Result<String, String> {
    fn collect(
        root: &GitDirectory,
        directory: &GitDirectory,
        relative: &Path,
        files: &mut BTreeMap<String, blake3::Hash>,
    ) -> Result<(), String> {
        let read_error = |error| format!("cannot read Git dependency path {}: {error}", relative.display());
        for name in directory.entries().map_err(read_error)? {
            let path = relative.join(name);
            let file = root.open(&path)?
                .ok_or_else(|| format!("Git dependency path {} changed while hashing", path.display()))?;
            let metadata = file.metadata().map_err(read_error)?;
            if metadata.is_dir() {
                // The handle whose type was checked also determines which directory is listed.
                let child = GitDirectory::new(file)?;
                collect(root, &child, &path, files)?;
            } else if metadata.is_file() {
                let mut content = blake3::Hasher::new();
                content.update_reader(file).map_err(read_error)?;
                let path = path.to_str().ok_or_else(|| "non-UTF-8 Git dependency path".to_owned())?;
                files.insert(path.to_owned(), content.finalize());
            } else {
                return Err(format!("{} is not a regular dependency file", path.display()));
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    collect(root, root, Path::new(""), &mut files)?;
    let mut tree = blake3::Hasher::new();
    tree.update(b"wosy-package-source-v1\0");
    for (path, content) in files {
        tree.update(&(path.len() as u64).to_le_bytes());
        tree.update(path.as_bytes());
        tree.update(content.as_bytes());
    }
    Ok(tree.finalize().to_hex().to_string())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn stage_and_sync_lock_follow_the_open_project_not_its_reused_path() {
        let mut outer = None;
        let (temp, temp_handle) = GitDirectory::project(&std::env::temp_dir()).unwrap();
        let outer = get_stage(&mut outer, &temp, &temp_handle).unwrap();
        let original = outer.display.join("project");
        let moved = outer.display.join("moved");
        fs::create_dir(&original).unwrap();
        let original = original.canonicalize().unwrap();
        let metadata = ProjectSnapshot::writer(&original).unwrap();
        let project = metadata.git_project().unwrap().directory;

        let mut stage = None;
        let staging = get_stage(&mut stage, &original, &project).unwrap();
        let name = staging.name.clone();
        fs::rename(&original, &moved).unwrap();
        fs::create_dir_all(original.join(&name)).unwrap();
        fs::write(original.join(&name).join("keep.txt"), "replacement tree").unwrap();

        fs::write(staging.path.join("ours.txt"), "original stage").unwrap();
        assert_eq!(fs::read_to_string(moved.join(&name).join("ours.txt")).unwrap(), "original stage");
        let moved_handle = fs::File::open(&moved).unwrap();
        assert!(matches!(moved_handle.try_lock(), Err(fs::TryLockError::WouldBlock)));
        let replacement = fs::File::open(&original).unwrap();
        assert!(replacement.try_lock().is_ok());

        drop(stage);
        drop(metadata);
        assert!(!moved.join(&name).exists());
        assert_eq!(fs::read_to_string(original.join(&name).join("keep.txt")).unwrap(), "replacement tree");
    }

    #[test]
    fn publication_restores_checkout_and_old_receipts_when_later_rename_fails() {
        let mut stage = None;
        let (root, project) = GitDirectory::project(&std::env::temp_dir()).unwrap();
        let staging = get_stage(&mut stage, &root, &project).unwrap();
        let deps_path = staging.path.join("existing");
        let receipts_path = deps_path.join(PINS);
        fs::create_dir_all(&receipts_path).unwrap();
        fs::write(receipts_path.join("first"), "previous receipt").unwrap();
        let existing = deps_path.join("unchanged");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("edit"), "keep user edit").unwrap();
        let staged_path = staging.path.join(DEPS);
        fs::create_dir_all(staged_path.join(PINS)).unwrap();
        let stage = staging.handle.clone();
        let deps = stage.directory(Path::new("existing")).unwrap().unwrap();
        let stage_deps = stage.directory(Path::new(DEPS)).unwrap().unwrap();

        let mut installs = ["first", "second"].map(|name| {
            let checkout = staged_path.join(name);
            let receipt = staged_path.join(PINS).join(name);
            fs::create_dir(&checkout).unwrap();
            fs::write(&receipt, "new receipt").unwrap();
            PendingInstall {
                name: name.to_owned(),
                expected_checkout: stage_deps.directory(Path::new(name)).unwrap().unwrap(),
                expected_receipt: "new receipt".to_owned(),
                expected_source_hash: String::new(),
                destination: deps_path.join(name),
                previous_receipt: false,
                receipt_published: false,
                checkout_published: false,
            }
        });
        let expected_hash = source_hash(&installs[0].expected_checkout).unwrap();
        for install in &mut installs {
            install.expected_source_hash.clone_from(&expected_hash);
        }
        let error = publish_updates(
            &stage, Path::new("existing"), &deps, &stage, &stage_deps, staging,
            &mut installs, |from, name, into, dest| {
            if from == &stage_deps && name == Path::new("second") {
                Err(std::io::Error::other("simulated checkout rename failure"))
            } else {
                from.rename(name, into, dest)
            }
        }).unwrap_err();

        assert!(error.contains("simulated checkout rename failure"));
        assert!(!error.contains("rollback incomplete"));
        assert!(!deps_path.join("first").exists());
        assert!(!deps_path.join("second").exists());
        assert_eq!(fs::read_to_string(receipts_path.join("first")).unwrap(), "previous receipt");
        assert!(!receipts_path.join("second").exists());
        assert_eq!(fs::read_to_string(existing.join("edit")).unwrap(), "keep user edit");
    }
}
