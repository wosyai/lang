use super::{
    git_sync::{source_hash, Staging},
    graph, load_git, load_local, normalize_package_path, valid_origin, valid_package_name,
    GitDirectory, GitProject, LockedPackage, PackageLock, ProjectSnapshot, LOCKFILE,
};
use crate::{project_dependencies, scalar_string, DependencyDeclaration, DependencySource, GitSource};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, ImDocument, Item, Table};

use super::snapshot::MANIFEST;

/// Add one explicit direct dependency and pin every newly reached Git package.
/// This command plans in a disposable stage; only dep sync installs packages.
pub fn add_dependency(root: &Path, source: &str, package_path: Option<&str>) -> Result<(), String> {
    let snapshot = ProjectSnapshot::writer(root)?;
    let original_manifest = snapshot.manifest()?;
    let old = ImDocument::parse(original_manifest.clone()).map_err(|error| error.to_string())?;
    let root_name = manifest_name(&old)?;
    let old_declarations = project_dependencies(snapshot.root(), &root_name, &old)?;
    let mut preview = Preview {
        snapshot: &snapshot,
        git: None,
        pins: PackageLock::default(),
        staging: Staging::default(),
        checkouts: BTreeMap::new(),
        inspected: None,
    };
    let mut closure = preview.plan(&root_name, &old_declarations)?;
    snapshot.check_reached(&closure.git_packages)?;

    let (name, source) = preview.select(source, package_path)?;
    if !valid_package_name(&name) {
        return Err(format!("dependency name {name:?} must be a snake_case Wosy package name"));
    }
    if name == root_name || old_declarations.contains_key(&name) {
        return Err(format!("project already declares package {name}"));
    }
    let mut updated = old.into_mut();
    let dependencies = updated.entry("dependencies")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut().ok_or_else(|| "dependencies must be a table".to_owned())?;
    let mut entry = Table::new();
    match &source {
        DependencySource::Local { path } => {
            let path = path.to_str().ok_or_else(|| "local dependency path must be UTF-8".to_owned())?;
            entry["path"] = value(path);
        }
        DependencySource::Git(source) => {
            entry["git"] = value(&source.origin);
            if source.package_path != "." {
                entry["path"] = value(&source.package_path);
            }
        }
    }
    dependencies.insert(&name, Item::Table(entry));
    let declaration = DependencyDeclaration {
        name,
        source,
        span: crate::dependency_diagnostic(snapshot.root(), &root_name, "P0001", "", 0..0).span,
    };
    closure.add(snapshot.root(), &root_name, &declaration, |binding, parent, declaration| {
        preview.provide(binding, parent, declaration, true)
    })?;
    let updated_manifest = updated.to_string();
    if let Some(project) = &preview.git {
        preview.pins.check_reached(&closure.git_packages)?;
        let original = snapshot.parsed_lock()?;
        let updated_lock = (original != Some(&preview.pins)).then(|| preview.pins.render());
        let staging = preview.staging.workspace(project)?;
        publish_metadata(
            &snapshot, project, staging, &original_manifest, updated_manifest.as_str(), updated_lock.as_deref(), || {},
        )
    } else {
        // No Git input or lock update: no Git descriptor operations are needed.
        snapshot.require_unchanged(&original_manifest)?;
        snapshot.replace_local_manifest(&updated_manifest)
    }
}

struct Preview<'a> {
    snapshot: &'a ProjectSnapshot,
    git: Option<GitProject>,
    pins: PackageLock,
    staging: Staging,
    checkouts: BTreeMap<String, GitDirectory>,
    inspected: Option<(String, InspectedSource, ImDocument<String>)>,
}

#[derive(Eq, PartialEq)]
enum InspectedSource {
    Local(PathBuf),
    Git(GitSource),
}

impl Preview<'_> {
    fn plan(&mut self, root_name: &str, declarations: &graph::Declarations) -> Result<graph::Resolution, String> {
        graph::resolve(self.snapshot.root(), root_name, declarations, |binding, parent, declaration| {
            self.provide(binding, parent, declaration, false)
        })
    }

    fn provide(
        &mut self,
        binding: &graph::Binding,
        parent: &Path,
        declaration: &DependencyDeclaration,
        allow_new: bool,
    ) -> Result<graph::LoadedPackage, String> {
        match binding {
            graph::Binding::Local(_) => {
                let DependencySource::Local { path } = &declaration.source else { unreachable!() };
                let inspected = self.take_inspection(&declaration.name, &InspectedSource::Local(parent.join(path)));
                load_local(parent, declaration, inspected)
            }
            graph::Binding::Git(requirement) => self.load_git(declaration, requirement, allow_new),
        }
    }

    fn ensure_git(&mut self) -> Result<(), String> {
        if self.git.is_none() {
            self.pins = self.snapshot.parsed_lock()?.cloned().unwrap_or_default();
            self.git = Some(self.snapshot.git_project()?);
        }
        Ok(())
    }

    fn take_inspection(&mut self, name: &str, source: &InspectedSource) -> Option<ImDocument<String>> {
        if self.inspected.as_ref().is_some_and(|(package, actual, _)| package == name && actual == source) {
            self.inspected.take().map(|(_, _, document)| document)
        } else {
            None
        }
    }

    fn select(&mut self, source: &str, package_path: Option<&str>) -> Result<(String, DependencySource), String> {
        if source.is_empty() {
            return Err("dep add source must not be empty".to_owned());
        }
        let local_path = self.snapshot.root().join(source).join(MANIFEST);
        if package_path.is_none() && local_path.is_file() {
            let text = fs::read_to_string(&local_path)
                .map_err(|error| format!("cannot read {}: {error}", local_path.display()))?;
            let manifest = ImDocument::parse(text).map_err(|error| error.to_string())?;
            let name = manifest_name(&manifest)?;
            self.inspected = Some((name.clone(), InspectedSource::Local(self.snapshot.root().join(source)), manifest));
            return Ok((name, DependencySource::Local { path: PathBuf::from(source) }));
        }

        let package_path = package_directory(package_path.unwrap_or("."))?;
        if !valid_origin(source) {
            return Err("invalid Git origin".to_owned());
        }
        let source = GitSource { origin: source.to_owned(), package_path };
        self.ensure_git()?;
        if let Some((name, _)) = self.pins.packages.iter().find(|(_, pin)| pin.source == source) {
            // Promoting a transitive package to a direct dependency preserves
            // its reviewed lock even when HEAD has advanced.
            return Ok((name.clone(), DependencySource::Git(source)));
        }
        let git = self.git.as_ref().expect("opened Git project");
        let (checkout, revision) = self.staging.fetch_tip(git, ".selected", &source)?;
        let text = checkout.read_text(Path::new(MANIFEST))?
            .ok_or_else(|| format!("selected Git package has no {MANIFEST}"))?;
        let manifest = ImDocument::parse(text).map_err(|error| error.to_string())?;
        let name = manifest_name(&manifest)?;
        self.inspected = Some((name.clone(), InspectedSource::Git(source.clone()), manifest));
        if self.pins.packages.contains_key(&name) {
            return Err(format!("conflicting package binding for {name}; existing pin was not changed"));
        }
        let pin = LockedPackage { source: source.clone(), revision, source_hash: source_hash(&checkout)? };
        self.checkouts.insert(name.clone(), checkout);
        self.pins.packages.insert(name.clone(), pin);
        Ok((name, DependencySource::Git(source)))
    }

    fn load_git(
        &mut self,
        declaration: &DependencyDeclaration,
        requirement: &graph::GitRequirement,
        allow_new: bool,
    ) -> Result<graph::LoadedPackage, String> {
        self.ensure_git()?;
        let source = &requirement.source;
        let inspected = self.take_inspection(&declaration.name, &InspectedSource::Git(source.clone()));
        let git = self.git.as_ref().expect("opened Git project");
        let name = declaration.name.as_str();
        if !self.pins.packages.contains_key(name) && allow_new {
            let (checkout, revision) = match &requirement.revision {
                Some(revision) => (
                    self.staging.fetch(git, name, source, revision)?,
                    revision.clone(),
                ),
                None => self.staging.fetch_tip(git, name, source)?,
            };
            let pin = LockedPackage { source: source.clone(), revision, source_hash: source_hash(&checkout)? };
            self.checkouts.insert(name.to_owned(), checkout);
            self.pins.packages.insert(name.to_owned(), pin);
        }
        let pin = self.pins.pin_for_requirement(name, requirement)?;
        if !self.checkouts.contains_key(name) {
            let checkout = if let Some(existing) = git.installed(name, pin)? {
                existing
            } else {
                let downloaded = self.staging.fetch(git, name, source, &pin.revision)?;
                if source_hash(&downloaded)? != pin.source_hash {
                    return Err(format!("dependency {name} source hash mismatch"));
                }
                downloaded
            };
            self.checkouts.insert(name.to_owned(), checkout);
        }
        load_git(
            self.snapshot.root(),
            declaration,
            self.checkouts[name].clone(),
            pin,
            inspected,
        )
    }
}

fn manifest_name(document: &ImDocument<String>) -> Result<String, String> {
    let package = document.get("package").and_then(Item::as_table)
        .ok_or_else(|| "package is missing from wosy.toml".to_owned())?;
    scalar_string(package, "name")
}

fn package_directory(path: &str) -> Result<String, String> {
    let normalized = normalize_package_path(path)
        .map_err(|_| "Git package path must be a repository-relative directory or wosy.toml".to_owned())?;
    let path = Path::new(&normalized);
    if path.file_name() == Some(std::ffi::OsStr::new(MANIFEST)) {
        let parent = path.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        Ok(parent.to_str().expect("normalized UTF-8 package path").to_owned())
    } else {
        Ok(normalized)
    }
}

fn publish_metadata(
    snapshot: &ProjectSnapshot,
    project: &GitProject,
    stage: &super::git_sync::Stage,
    original_manifest: &str,
    updated_manifest: &str,
    updated_lock: Option<&str>,
    before_manifest_publish: impl FnOnce(),
) -> Result<(), String> {
    let original_lock = snapshot.lock_bytes()?;
    let manifest = Path::new(MANIFEST);
    let lock = Path::new(LOCKFILE);
    let new_manifest = Path::new("replacement-manifest");
    let new_lock = Path::new("replacement-lock");
    let live = &project.directory;
    let stage_dir = &stage.handle;
    let stage_text = |name: &Path, target: &Path, text: &str| -> Result<(), String> {
        let path = stage.path.join(name);
        fs::write(&path, text).map_err(|error| format!("cannot stage {}: {error}", target.display()))?;
        if let Some(old) = live.source(target)? {
            let permissions = old.metadata().map_err(|error| error.to_string())?.permissions();
            fs::set_permissions(path, permissions).map_err(|error| format!("cannot preserve {} permissions: {error}", target.display()))?;
        }
        Ok(())
    };
    stage_text(new_manifest, manifest, updated_manifest)?;
    if let Some(text) = updated_lock {
        stage_text(new_lock, lock, text)?;
    }
    snapshot.require_unchanged(original_manifest)?;

    let mut changed_lock = false;
    let publish = (|| {
        if updated_lock.is_some() {
            if original_lock.is_some() {
                stage_dir.exchange(new_lock, live, lock)
            } else {
                stage_dir.rename(new_lock, live, lock)
            }.map_err(|error| format!("cannot publish {LOCKFILE}: {error}"))?;
            changed_lock = true;
        }
        before_manifest_publish();
        stage_dir.exchange(new_manifest, live, manifest)
            .map_err(|error| format!("cannot publish {MANIFEST}: {error}"))?;
            Ok(())
    })();
    let Err(cause) = publish else { return Ok(()); };
    let mut rollback = Vec::new();
    if changed_lock {
        let result = if original_lock.is_some() {
            stage_dir.exchange(new_lock, live, lock)
        } else {
            live.rename(lock, stage_dir, new_lock)
        };
        if let Err(error) = result {
            rollback.push(format!("cannot restore {LOCKFILE}: {error}"));
        }
    }
    if rollback.is_empty() {
        Err(cause)
    } else {
        stage.preserve.set(true);
        Err(format!("{cause}; rollback incomplete: {}; backups left in {}", rollback.join("; "), stage.display.display()))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs::{File, TryLockError};

    #[test]
    fn second_publication_failure_restores_both_generations_while_readers_are_excluded() {
        for existing in [None, Some("version = 1\n# previous lock\n")] {
            let root = std::env::temp_dir().join(format!(
                "wosy-metadata-rollback-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            ));
            fs::create_dir(&root).unwrap();
            let manifest = "[package]\nname = \"app\"\n";
            let updated = "[package]\nname = \"app\"\n[dependencies.source]\npath = \"../source\"\n";
            let new_lock = "version = 1\n# staged lock\n";
            fs::write(root.join(MANIFEST), manifest).unwrap();
            if let Some(old) = existing {
                fs::write(root.join(LOCKFILE), old).unwrap();
            }
            {
                let snapshot = ProjectSnapshot::writer(&root).unwrap();
                let git = snapshot.git_project().unwrap();
                let mut downloads = Staging::default();
                let stage = downloads.workspace(&git).unwrap();
                let result = publish_metadata(&snapshot, &git, stage, manifest, updated, Some(new_lock), || {
                    // The lock publication happened. A cooperating reader still
                    // cannot open either generation until the writer finishes.
                    let reader = File::open(&root).unwrap();
                    assert!(matches!(reader.try_lock_shared(), Err(TryLockError::WouldBlock)));
                    assert_eq!(fs::read_to_string(root.join(LOCKFILE)).unwrap(), new_lock);
                    // Make the next real atomic exchange fail with ENOENT.
                    fs::remove_file(stage.path.join("replacement-manifest")).unwrap();
                });
                assert!(result.is_err());
            }

            let recovered = ProjectSnapshot::reader(&root).unwrap();
            assert_eq!(recovered.manifest().unwrap(), manifest);
            assert_eq!(recovered.lock_bytes().unwrap(), existing.map(str::as_bytes));
            drop(recovered);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
