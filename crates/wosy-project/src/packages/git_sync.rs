use super::{git_wire::{self, GitOid, GitTreeOid}, graph, load_git, load_local_declarations, GitDirectory, GitProject, LockedInputId, LockedPackage, ProjectSnapshot, DEPS};
use super::patches::{GitOverlay, PatchStack, PatchStackId};
use crate::{dependency_diagnostic, project_dependencies, scalar_string, DependencyDeclaration, DependencySource, GitSource, PackageRelativePath};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::Item;

pub(super) const PINS: &str = ".wosy-pins";

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
                let stack = PatchStack::load(&git.directory, name)?;
                let mut install = None;
                let mut live = None;
                let checkout = if let Some(installed) = git.installed(name, pin)? {
                    let actual = tree_identity(&installed.checkout)?;
                    let stack_changed = installed.stack != stack.identity();
                    let pristine = if stack_changed && installed.baseline.is_none() {
                        Some(downloads.pristine(git, name, pin)?)
                    } else {
                        None
                    };
                    let baseline = installed.baseline.or_else(|| pristine.as_ref().map(|pristine| pristine.identity.working));
                    let unchanged = baseline.map_or(actual.original_v1 == pin.source_hash, |baseline| baseline == actual.working);
                    if !unchanged {
                        return Err(dirty_checkout(git, name));
                    }
                    if stack_changed {
                        let before = PreviousCheckout { checkout: installed.checkout, baseline: actual.working };
                        let (checkout, working, tree) = if let Some(pristine) = pristine {
                            let (checkout, working) = downloads.replay(git, name, &pristine, &stack)?;
                            (checkout, working, pristine.tree)
                        } else {
                            downloads.overlay(git, name, pin, &stack)?
                        };
                        install = Some((working, tree, Some(before)));
                        checkout
                    } else {
                        let checkout = installed.checkout.clone();
                        live = Some(installed);
                        checkout
                    }
                } else {
                    let (checkout, working, tree) = downloads.overlay(git, name, pin, &stack)?;
                    install = Some((working, tree, None));
                    checkout
                };
                let loaded = load_git(root, declaration, checkout.clone(), pin, None)?;
                let routes = FrozenRoutes::capture(&loaded.dependencies);
                if let Some((working, tree, previous)) = install {
                    downloads.prepare_install(git, name, pin, &stack, checkout, working, previous, &routes, &tree)?;
                } else if let Some(live) = live {
                    if let Some(baseline) = live.baseline {
                        if live.routes.as_ref() != Some(&routes) {
                            downloads.refresh_receipt(git, name, pin, &stack, live.checkout, baseline, &routes, live.base_tree.as_ref())?;
                        }
                    }
                }
                Ok(loaded)

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
    retirements: Vec<Retirement>,
    tips: BTreeMap<String, (PathBuf, GitOid)>,
    exact: BTreeMap<(String, GitOid), PathBuf>,
    ancestry: BTreeSet<(String, GitOid, GitOid)>,
}

/// Exact fetched Git object tree and the identities recorded while extracting
/// its blob bytes. Replays can use these objects directly, without re-reading
/// the immutable worktree to rebuild an equivalent Git index.
pub(super) struct PristinePackage {
    pub(super) checkout: GitDirectory,
    pub(super) identity: TreeIdentity,
    pub(super) tree: GitTreeOid,
    pub(super) repository: PathBuf,
}

/// One observed working generation. Native Git reads only these frozen files,
/// never a second view of the live checkout.
pub(super) struct FrozenTree {
    pub(super) directory: GitDirectory,
    pub(super) files: BTreeMap<String, (blake3::Hash, bool)>,
    pub(super) working: WorkingTreeId,
}

impl PristinePackage {
    pub(super) fn require_pin_hash(&self, name: &str, pin: &LockedPackage) -> Result<(), String> {
        if pin.source_hash != self.identity.original_v1 {
            return Err(format!(
                "dependency {name} source hash mismatch: expected {}, got {}",
                pin.source_hash, self.identity.original_v1,
            ));
        }
        Ok(())
    }
}

impl Staging {
    pub(super) fn freeze_working(
        &mut self,
        git: &GitProject,
        name: &str,
        live: &GitDirectory,
    ) -> Result<FrozenTree, String> {
        let stage = self.workspace(git)?;
        let slot = Path::new("snapshots").join(name);
        let frozen = stage.path.join(&slot);
        fs::create_dir_all(&frozen).map_err(|error| format!("cannot stage snapshot for {name}: {error}"))?;
        let mut files = BTreeMap::new();
        collect_tree_files(live, |path, contents, executable| {
            let destination = frozen.join(path);
            let snapshot = (|| -> std::io::Result<blake3::Hash> {
                fs::create_dir_all(destination.parent().expect("snapshot file parent"))?;
                let output = OpenOptions::new().write(true).create_new(true).open(destination)?;
                copy_hashed(contents, output)
            })().map_err(|error| format!("cannot capture {name} file {path}: {error}"))?;
            files.insert(path.to_owned(), (snapshot, executable));
            Ok(())
        })?;
        let working = identity_from_files(&files).working;
        let directory = stage.handle.directory(&slot)?.ok_or_else(|| format!("staged snapshot for {name} disappeared"))?;
        Ok(FrozenTree { directory, files, working })
    }

    pub(super) fn prepare_install(
        &mut self,
        git: &GitProject,
        name: &str,
        pin: &LockedPackage,
        stack: &PatchStack,
        checkout: GitDirectory,
        working: WorkingTreeId,
        previous_checkout: Option<PreviousCheckout>,
        routes: &FrozenRoutes,
        tree: &GitTreeOid,
    ) -> Result<(), String> {
        self.prepare(git, name, pin, stack, checkout, working, previous_checkout, routes, Some(tree), false)
    }

    pub(super) fn refresh_receipt(
        &mut self,
        git: &GitProject,
        name: &str,
        pin: &LockedPackage,
        stack: &PatchStack,
        checkout: GitDirectory,
        baseline: WorkingTreeId,
        routes: &FrozenRoutes,
        tree: Option<&GitTreeOid>,
    ) -> Result<(), String> {
        self.prepare(git, name, pin, stack, checkout, baseline, None, routes, tree, true)
    }

    fn prepare(
        &mut self,
        git: &GitProject,
        name: &str,
        pin: &LockedPackage,
        stack: &PatchStack,
        checkout: GitDirectory,
        working: WorkingTreeId,
        previous_checkout: Option<PreviousCheckout>,
        routes: &FrozenRoutes,
        tree: Option<&GitTreeOid>,
        receipt_only: bool,
    ) -> Result<(), String> {
        let staging = self.workspace(git)?;
        let receipt = staging.path.join(DEPS).join(PINS).join(name);
        fs::create_dir_all(receipt.parent().expect("receipt staging parent"))
            .map_err(|error| format!("cannot create provenance staging directory: {error}"))?;
        let expected_receipt = overlay_receipt(&git.directory, &checkout, name, pin, stack.identity(), working, routes, tree)?;
        fs::write(&receipt, &expected_receipt)
            .map_err(|error| format!("cannot stage provenance for {name}: {error}"))?;
        self.pending.push(PendingInstall {
            name: name.to_owned(),
            expected_checkout: checkout.clone(),
            expected_receipt,
            expected_working: working,
            previous_checkout,
            receipt_only,
            destination: git.root.join(DEPS).join(name),
            previous_receipt: false,
            receipt_published: false,
            checkout_published: false,
        });
        Ok(())
    }

    pub(super) fn pristine(
        &mut self,
        git: &GitProject,
        slot: &str,
        pin: &LockedPackage,
    ) -> Result<PristinePackage, String> {
        let pristine = self.fetch(git, slot, &pin.source, &pin.revision)?;
        pristine.require_pin_hash(slot, pin)?;
        Ok(pristine)
    }

    pub(super) fn overlay(
        &mut self,
        git: &GitProject,
        slot: &str,
        pin: &LockedPackage,
        stack: &PatchStack,
    ) -> Result<(GitDirectory, WorkingTreeId, GitTreeOid), String> {
        let pristine = self.pristine(git, slot, pin)?;
        let (checkout, working) = self.replay(git, slot, &pristine, stack)?;
        Ok((checkout, working, pristine.tree))
    }

    pub(super) fn replay(
        &mut self,
        git: &GitProject,
        slot: &str,
        pristine: &PristinePackage,
        stack: &PatchStack,
    ) -> Result<(GitDirectory, WorkingTreeId), String> {
        if stack.is_empty() {
            return Ok((pristine.checkout.clone(), pristine.identity.working));
        }
        let staging = self.workspace(git)?;
        let overlay = GitOverlay::for_pristine(git, staging, pristine);
        let applied = overlay.apply(slot, &pristine.tree, stack)?;
        let destination = staging.path.join(DEPS).join(slot);
        // The pristine objects and digest remain available in the private
        // repository; only the effective source tree needs to stay on disk.
        fs::remove_dir_all(&destination)
            .map_err(|error| format!("cannot replace private pristine stage for {slot}: {error}"))?;
        let identity = overlay.materialize(&applied, &destination)?;
        Ok((staging.checkout(slot)?, identity.working))
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
    ) -> Result<PristinePackage, String> {
        let (checkout, unused_repository, stage) = {
            let staging = self.workspace(git)?;
            let (checkout, repository) = staging.git_paths(slot)?;
            (checkout, repository, staging.handle.clone())
        };
        let key = (source.origin.clone(), revision.clone());
        let repository = if let Some(repository) = self.exact.get(&key) {
            repository.clone()
        } else {
            fetch_exact(&git.directory, &stage, &unused_repository, source, revision)?;
            self.exact.insert(key, unused_repository.clone());
            unused_repository
        };
        let staging = self.stage.as_ref().expect("prepared checkout staging");
        staging.fetched(git, slot, repository, &checkout, source, revision)
    }

    pub(super) fn fetch_requirement(
        &mut self,
        git: &GitProject,
        slot: &str,
        requirement: &graph::GitRequirement,
        history: bool,
    ) -> Result<(PristinePackage, GitOid), String> {
        let source = &requirement.source;
        match &requirement.revision {
            Some(revision) => Ok((self.fetch(git, slot, source, revision)?, revision.clone())),
            None => self.fetch_tip_with_history(git, slot, source, history),
        }
    }

    pub(super) fn fetch_tip(
        &mut self,
        git: &GitProject,
        slot: &str,
        source: &GitSource,
    ) -> Result<(PristinePackage, GitOid), String> {
        self.fetch_tip_with_history(git, slot, source, false)
    }

    fn fetch_tip_with_history(
        &mut self,
        git: &GitProject,
        slot: &str,
        source: &GitSource,
        history: bool,
    ) -> Result<(PristinePackage, GitOid), String> {
        let (checkout, unused_repository, stage) = {
            let staging = self.workspace(git)?;
            let (checkout, repository) = staging.git_paths(slot)?;
            (checkout, repository, staging.handle.clone())
        };
        let (repository, revision) = if let Some(existing) = self.tips.get(&source.origin) {
            existing.clone()
        } else {
            let mut command = git_command(&git.directory, &stage, None)?;
            command.args(["clone", "--quiet", "--bare", "--single-branch", "--no-tags", "--no-local", "--template="]);
            if !history {
                command.arg("--depth=1");
            }
            checked_output(command.arg("--").arg(&source.origin).arg(&unused_repository), "clone")?;
            let revision = read_oid(&git.directory, &stage, &unused_repository, "HEAD^{commit}")?;
            let resolved = (unused_repository, revision);
            self.exact.insert((source.origin.clone(), resolved.1.clone()), resolved.0.clone());
            self.tips.insert(source.origin.clone(), resolved.clone());
            resolved
        };
        let staging = self.stage.as_ref().expect("prepared checkout staging");
        let pristine = staging.fetched(git, slot, repository, &checkout, source, &revision)?;
        Ok((pristine, revision))
    }

    pub(super) fn require_forward(
        &mut self,
        git: &GitProject,
        pristine: &PristinePackage,
        source: &GitSource,
        old: &GitOid,
        new: &GitOid,
    ) -> Result<(), String> {
        if old == new {
            return Ok(());
        }
        let key = (source.origin.clone(), old.clone(), new.clone());
        if self.ancestry.contains(&key) {
            return Ok(());
        }
        let staging = self.stage.as_ref().expect("prepared candidate staging");
        let output = git_command(&git.directory, &staging.handle, Some(&pristine.repository))?
            .args(["merge-base", "--is-ancestor", old.as_str(), new.as_str()])
            .output().map_err(|error| format!("cannot check revision ancestry: {error}"))?;
        match output.status.code() {
            Some(0) => {
                self.ancestry.insert(key);
                Ok(())
            }
            Some(1) => Err(format!(
                "Git origin {} diverged: {} is not an ancestor of {}",
                source.origin, old.as_str(), new.as_str()
            )),
            _ => Err(format!(
                "cannot establish forward history for {} ({} to {}): {}",
                source.origin, old.as_str(), new.as_str(), String::from_utf8_lossy(&output.stderr)
            )),
        }
    }

    pub(super) fn retire(
        &mut self,
        git: &GitProject,
        name: &str,
        previous: Option<PreviousCheckout>,
    ) -> Result<(), String> {
        self.workspace(git)?;
        self.retirements.push(Retirement {
            name: name.to_owned(),
            previous,
            moved_checkout: false,
            moved_receipt: false,
        });
        Ok(())
    }

    fn publish(self, git: &GitProject) -> Result<(), String> {
        self.publish_with(git, |_| Ok(()))
    }

    pub(super) fn publish_with(
        self,
        git: &GitProject,
        commit: impl FnOnce(Option<&Stage>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut retirements = self.retirements;
        publish_staging(&git.directory, self.stage, self.pending, |stage| {
            if retirements.is_empty() {
                commit(stage)
            } else {
                let stage = stage.expect("prepared retirements");
                publish_retirements(git, stage, &mut retirements, || commit(Some(stage)))
            }
        })
    }
}

struct Retirement {
    name: String,
    previous: Option<PreviousCheckout>,
    moved_checkout: bool,
    moved_receipt: bool,
}

fn publish_retirements(
    git: &GitProject,
    stage: &Stage,
    entries: &mut [Retirement],
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let Some(deps) = git.directory.directory(Path::new(DEPS))? else {
        if entries.iter().any(|entry| entry.previous.is_some()) {
            return Err("installed dependencies disappeared during update".to_owned());
        }
        return commit();
    };
    let receipts = deps.directory(Path::new(PINS))?;
    let retired = Path::new("retired-checkouts");
    let retired_receipts = Path::new("retired-receipts");
    for dir in [retired, retired_receipts] {
        stage.handle.mkdir(dir).map_err(|error| format!("cannot stage retired dependencies: {error}"))?;
    }
    let retired = stage.handle.directory(retired)?.expect("created retirement stage");
    let retired_receipts = stage.handle.directory(retired_receipts)?.expect("created receipt retirement stage");
    let apply = (|| {
        for entry in entries.iter_mut() {
            let path = Path::new(&entry.name);
            let ready = deps.directory(path)?;
            if ready.as_ref() != entry.previous.as_ref().map(|previous| &previous.checkout) {
                return Err(format!("cannot retire {}: installed checkout changed", entry.name));
            }
            if let Some(previous) = &entry.previous {
                deps.rename(path, &retired, path)
                    .map_err(|error| format!("cannot stage retirement of {}: {error}", entry.name))?;
                entry.moved_checkout = true;
                if tree_identity(&previous.checkout)?.working != previous.baseline {
                    return Err(dirty_checkout(git, &entry.name));
                }
            }
            if let Some(receipts) = &receipts {
                if receipts.source(path)?.is_some() {
                    receipts.rename(path, &retired_receipts, path)
                        .map_err(|error| format!("cannot stage receipt retirement for {}: {error}", entry.name))?;
                    entry.moved_receipt = true;
                }
            }
        }
        commit()
    })();
    let Err(error) = apply else { return Ok(()); };
    let mut recovery = Vec::new();
    for entry in entries.iter().rev() {
        let path = Path::new(&entry.name);
        if entry.moved_checkout {
            if let Err(error) = retired.rename(path, &deps, path) {
                recovery.push(format!("{} checkout: {error}", entry.name));
            }
        }
        if entry.moved_receipt {
            if let Err(error) = retired_receipts.rename(path, receipts.as_ref().expect("previous receipt directory"), path) {
                recovery.push(format!("{} receipt: {error}", entry.name));
            }
        }
    }
    if recovery.is_empty() {
        Err(error)
    } else {
        stage.preserve.set(true);
        Err(format!("{error}; retirement rollback incomplete: {}; recovery under {}", recovery.join("; "), stage.display.display()))
    }
}

fn publish_staging(
    project: &GitDirectory,
    stage: Option<Stage>,
    mut new_packages: Vec<PendingInstall>,
    commit: impl FnOnce(Option<&Stage>) -> Result<(), String>,
) -> Result<(), String> {
    if new_packages.is_empty() {
        return commit(stage.as_ref());
    }
    // Validate the whole graph before any publish. A clean project gets
    // checkouts and their receipts with one directory rename.
    let staging = stage.expect("all installs have staging");
    let stage_handle = &staging.handle;
    let stage_deps = stage_handle.directory(Path::new(DEPS))?
        .ok_or_else(|| "staged deps disappeared".to_owned())?;
    let dep_name = Path::new(DEPS);
    let Some(deps) = project.directory(dep_name)? else {
        verify_staged(&stage_deps, &new_packages)?;
        stage_handle.rename(dep_name, project, dep_name)
            .map_err(|error| format!("cannot publish {DEPS}: {error}"))?;
        if let Err(cause) = commit(Some(&staging)) {
            let unchanged = new_packages.iter().all(|entry| {
                tree_identity(&entry.expected_checkout)
                    .is_ok_and(|identity| identity.working == entry.expected_working)
            });
            if unchanged {
                if let Err(error) = project.rename(dep_name, stage_handle, dep_name) {
                    staging.preserve.set(true);
                    return Err(format!("{cause}; cannot restore absent {DEPS}: {error}"));
                }
                return Err(cause);
            }
            staging.preserve.set(true);
            return Err(format!("{cause}; changed published checkouts were left in place"));
        }
        return Ok(());
    };
    publish_updates(
        project, dep_name, &deps, stage_handle, &stage_deps, &staging,
        &mut new_packages, GitDirectory::rename, || commit(Some(&staging)),
    )
}

fn verify_staged(deps: &GitDirectory, pending: &[PendingInstall]) -> Result<(), String> {
    let receipts = deps.directory(Path::new(PINS))?
        .ok_or_else(|| "staged provenance directory disappeared".to_owned())?;
    for install in pending {
        let name = Path::new(&install.name);
        if (!install.receipt_only && deps.directory(name)?.as_ref() != Some(&install.expected_checkout))
            || receipts.read_text(name)?.as_deref() != Some(install.expected_receipt.as_str())
        {
            return Err(format!("staged dependency {} changed during publication", install.name));
        }
    }
    Ok(())
}

pub(super) struct PreviousCheckout {
    pub(super) checkout: GitDirectory,
    pub(super) baseline: WorkingTreeId,
}

pub(super) fn dirty_checkout(git: &GitProject, name: &str) -> String {
    format!(
        "dependency {name} installed tree differs (unsnapshotted edits); did not overwrite {}",
        git.root.join(DEPS).join(name).display()
    )
}

struct PendingInstall {
    name: String,
    expected_checkout: GitDirectory,
    expected_receipt: String,
    expected_working: WorkingTreeId,
    previous_checkout: Option<PreviousCheckout>,
    receipt_only: bool,
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
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    verify_staged(stage_deps, pending)?;
    let pins_name = Path::new(PINS);
    let live_receipts = deps.directory(pins_name)?;
    let staged_receipts = stage_deps.directory(pins_name)?
        .ok_or_else(|| "staged provenance directory disappeared".to_owned())?;
    let preexisting_receipts = pending.iter().map(|install| {
        live_receipts.as_ref().map_or(Ok(false), |receipts| {
            validate_receipt_destination(receipts, Path::new(&install.name))
        })
    }).collect::<Result<Vec<_>, _>>()?;
    let previous_name = Path::new("previous-receipts");
    stage.mkdir(previous_name)
        .map_err(|error| format!("cannot stage previous dependency provenance: {error}"))?;
    let previous = stage.directory(previous_name)?
        .ok_or_else(|| "previous provenance staging directory disappeared".to_owned())?;
    let made_receipts = live_receipts.is_none();
    let receipts = match live_receipts {
        Some(receipts) => receipts,
        None => {
            deps.mkdir(pins_name)
                .map_err(|error| format!("cannot create installed provenance directory: {error}"))?;
            match deps.directory(pins_name) {
                Ok(Some(directory)) => directory,
                Ok(None) => return Err("created provenance directory disappeared".to_owned()),
                Err(error) => {
                    let _ = deps.remove_empty_directory(pins_name);
                    return Err(error);
                }
            }
        }
    };

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
            if install.receipt_only {
                if deps.directory(name)?.as_ref() != Some(&install.expected_checkout) {
                    return Err(format!("{} changed during publication", install.destination.display()));
                }
                continue;
            }
            if let Some(previous) = &install.previous_checkout {
                if deps.directory(name)?.as_ref() != Some(&previous.checkout) {
                    return Err(format!("{} changed during publication", install.destination.display()));
                }
                stage_deps.exchange(name, deps, name)
                    .map_err(|error| format!("cannot replace {}: {error}", install.destination.display()))?;
                install.checkout_published = true;
                // The old inode remains reachable in staging. Checking it
                // after displacement protects edits during preparation.
                if tree_identity(&previous.checkout)?.working != previous.baseline {
                    return Err(format!("{} was edited before publication", install.destination.display()));
                }
            } else {
                rename(stage_deps, name, deps, name)
                    .map_err(|error| format!("cannot publish {}: {error}", install.destination.display()))?;
                install.checkout_published = true;
            }
        }
        if project.directory(dep_name)?.as_ref() != Some(deps)
            || deps.directory(pins_name)?.as_ref() != Some(&receipts)
        {
            return Err("installed dependency or provenance directory changed during publication".to_owned());
        }
        commit()
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
                    && tree_identity(&checkout).is_ok_and(|identity| identity.working == install.expected_working)
            });
            if !unchanged {
                rollback_errors.push(format!("{} changed before rollback and was left in place", install.destination.display()));
                continue;
            }
            let restore = if install.previous_checkout.is_some() {
                deps.exchange(name, stage_deps, name)
            } else {
                rename(deps, name, stage_deps, name)
            };
            if let Err(error) = restore {
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
    if made_receipts && deps.directory(pins_name).ok().flatten().as_ref() == Some(&receipts) {
        // Never remove a pre-existing directory or concurrently added contents.
        if let Err(error) = deps.remove_empty_directory(pins_name) {
            if error.kind() != ErrorKind::NotFound {
                rollback_errors.push(format!("could not remove newly created provenance directory: {error}"));
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

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(super) struct FrozenRoutes(BTreeMap<String, DependencySource>);

impl FrozenRoutes {
    pub(super) fn capture(declarations: &graph::Declarations) -> Self {
        Self(declarations.iter().map(|(name, declaration)| (name.clone(), declaration.source.clone())).collect())
    }

    pub(super) fn declarations(&self, root: &Path, package: &str) -> graph::Declarations {
        self.0.iter().map(|(name, source)| {
            let declaration = DependencyDeclaration {
                name: name.clone(),
                source: source.clone(),
                span: dependency_diagnostic(root, package, "P0002", "", 0..0).span,
            };
            (name.clone(), declaration)
        }).collect()
    }

    fn hash(&self, digest: &mut blake3::Hasher) {
        fn field(digest: &mut blake3::Hasher, bytes: &[u8]) {
            digest.update(&(bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
        digest.update(&(self.0.len() as u64).to_le_bytes());
        for (name, source) in &self.0 {
            field(digest, name.as_bytes());
            match source {
                DependencySource::Local { path } => {
                    digest.update(b"L");
                    field(digest, path.as_os_str().as_encoded_bytes());
                }
                DependencySource::Git(source) => {
                    digest.update(b"G");
                    field(digest, source.origin.as_bytes());
                    field(digest, source.package_path.as_bytes());
                }
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Installation {
    pub(super) checkout: GitDirectory,
    pub(super) stack: PatchStackId,
    pub(super) baseline: Option<WorkingTreeId>,
    pub(super) routes: Option<FrozenRoutes>,
    pub(super) base_tree: Option<GitTreeOid>,
}

impl Installation {
    pub(super) fn require_stack(&self, name: &str, stack: PatchStackId) -> Result<(), String> {
        if self.stack != stack {
            Err(format!("dependency {name} has unapplied patch changes; run wosy dep sync"))
        } else {
            Ok(())
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct OverlayReceipt {
    version: u8,
    stack: PatchStackId,
    baseline: WorkingTreeId,
    routes: FrozenRoutes,
    seal: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "optional_verified_base")]
    verified_base: Option<VerifiedBaseReceipt>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct VerifiedBaseReceipt {
    version: u8,
    tree: String,
    seal: String,
}

fn optional_verified_base<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<VerifiedBaseReceipt>, D::Error> {
    let value = <Option<serde_json::Value> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

fn verified_base_seal(legacy_seal: &str, tree: &str) -> String {
    let mut digest = blake3::Hasher::new_derive_key("wosy installed verified Git base receipt v1");
    // The parent is an already-verified, fixed-length legacy digest. No
    // second scan or serialization of the selected installation is needed.
    digest.update(legacy_seal.as_bytes());
    digest.update(tree.as_bytes());
    digest.finalize().to_hex().to_string()
}

pub(super) fn overlay_receipt(
    project: &GitDirectory,
    checkout: &GitDirectory,
    name: &str,
    pin: &LockedPackage,
    stack: PatchStackId,
    baseline: WorkingTreeId,
    routes: &FrozenRoutes,
    base_tree: Option<&GitTreeOid>,
) -> Result<String, String> {
    let evidence = InstallEvidence::new(project, checkout, name, pin);
    let seal = evidence.seal(Some((stack, baseline, routes)));
    let verified_base = base_tree.map(|tree| VerifiedBaseReceipt {
        version: 1,
        tree: tree.as_str().to_owned(),
        seal: verified_base_seal(&seal, tree.as_str()),
    });
    let record = OverlayReceipt {
        version: 1,
        stack,
        baseline,
        routes: routes.clone(),
        seal,
        verified_base,
    };
    serde_json::to_string(&record).map_err(|error| error.to_string())
}

pub(super) fn verify_installed(
    root: &Path,
    project: &GitDirectory,
    name: &str,
    pin: &LockedPackage,
) -> Result<Option<Installation>, String> {
    let Some(deps) = project.directory(Path::new(DEPS))? else { return Ok(None); };
    let Some(checkout) = deps.directory(Path::new(name))? else { return Ok(None); };
    let evidence = InstallEvidence::new(project, &checkout, name, pin);
    let receipt = deps.read_text(&Path::new(PINS).join(name))?;
    if let Some(receipt) = receipt {
        if let Ok(record) = serde_json::from_str::<OverlayReceipt>(&receipt) {
            if record.version == 1
                && record.seal == evidence.seal(Some((record.stack, record.baseline, &record.routes)))
            {
                let base_tree = record.verified_base.and_then(|base| {
                    if base.version != 1 { return None; }
                    let tree = GitOid::parse(&base.tree).ok()?;
                    (tree.as_str().len() == pin.revision.as_str().len()
                        && base.seal == verified_base_seal(&record.seal, tree.as_str()))
                        .then(|| GitTreeOid::from_installation_receipt(tree))
                });
                return Ok(Some(Installation {
                    checkout,
                    stack: record.stack,
                    baseline: Some(record.baseline),
                    routes: Some(record.routes),
                    base_tree,
                }));
            }
        } else if receipt == evidence.receipt() {
            // Phase 5 proved original bytes but did not record a mode-aware
            // baseline. Preserve the empty stack until a real base is needed.
            return Ok(Some(Installation { checkout, stack: PatchStack::empty().identity(), baseline: None, routes: None, base_tree: None }));
        }
    }
    Err(format!(
        "dependency {name} is not verified for its wosy.lock pin; move {} aside and run wosy dep sync",
        root.join(DEPS).join(name).display()
    ))
}

/// A base previously materialized at the exact selected pin. This
/// receipt identifies a Git tree, not a live filesystem or a compiler root.
pub struct InstalledGitBase {
    locked: LockedInputId,
    patches: PatchStackId,
    project: PathBuf,
    package: String,
    checkout: PathBuf,
    revision: GitOid,
    package_path: String,
    tree: GitTreeOid,
    source_hash: String,
    visited: BTreeMap<PackageRelativePath, [u8; 32]>,
}

impl InstalledGitBase {
    pub fn locked(&self) -> &LockedInputId { &self.locked }
    pub fn patches(&self) -> PatchStackId { self.patches }
    pub fn project(&self) -> &Path { &self.project }
    pub fn package(&self) -> &str { &self.package }
    pub fn checkout_path(&self) -> &Path { &self.checkout }
    pub fn revision(&self) -> &str { self.revision.as_str() }
    pub fn package_path(&self) -> &str { &self.package_path }
    pub fn tree(&self) -> &str { self.tree.as_str() }
    pub fn source_hash(&self) -> &str { &self.source_hash }
    /// Only the caller-selected file revisions actually compared with the
    /// anchored install. A metadata observation alone has no visited files.
    pub fn visited_files(&self) -> &BTreeMap<PackageRelativePath, [u8; 32]> { &self.visited }
}

/// Verify a direct installed package against its original Git source. No Git
/// process, fetch, receipt migration, or new install occurs on this path.
/// Legacy receipts and patched packages remain usable by the ordinary resolver
/// but have no pristine capture here. Only this observation is verified; it is
/// not a lock against subsequent external edits.
pub fn observe_installed_git_base(root: &Path, name: &str) -> Result<Option<InstalledGitBase>, String> {
    match_installed_git_base_files(root, name, std::iter::empty())
}

/// Compare existing source revision digests to the same anchored file bytes
/// used to verify the installed base. Hash and open each file once; no second
/// reading by filename and no retention of source bytes. Only visited files in
/// the supplied projection are matched; this is not a complete package CST.
pub fn match_installed_git_base_files<'a>(
    root: &Path, name: &str, expected: impl IntoIterator<Item = (&'a PackageRelativePath, &'a [u8; 32])>,
) -> Result<Option<InstalledGitBase>, String> {
    let snapshot = ProjectSnapshot::reader(root)?;
    let git = snapshot.git_project()?;
    let manifest = git.directory.read_text(Path::new(super::snapshot::MANIFEST))?
        .ok_or("installed Git base observation requires wosy.toml")?;
    let document = toml_edit::ImDocument::parse(manifest).map_err(|_| "invalid manifest for installed Git base observation")?;
    let package = document.get("package").and_then(Item::as_table).ok_or("package is missing")?;
    let package = scalar_string(package, "name")?;
    let dependencies = project_dependencies(snapshot.root(), &package, &document)
        .map_err(|_| "invalid dependencies for installed Git base observation")?;
    let Some(DependencyDeclaration { source: DependencySource::Git(source), .. }) = dependencies.get(name) else {
        return Ok(None);
    };
    let requirement = graph::GitRequirement { source: source.clone(), revision: None };
    let pin = snapshot.pin_for(name, &requirement)
        .map_err(|_| "installed Git dependency does not match a valid lock entry")?;
    let Some(installed) = git.installed(name, pin)? else { return Ok(None) };
    let stack = PatchStack::load(&git.directory, name)?;
    let patches = stack.identity();
    installed.require_stack(name, patches)?;
    if !stack.is_empty() { return Ok(None) }
    let (Some(tree), Some(baseline)) = (installed.base_tree, installed.baseline) else { return Ok(None) };
    let files = tree_file_digests(&installed.checkout)?;
    let actual = identity_from_files(&files);
    if actual.working != baseline || actual.original_v1 != pin.source_hash {
        return Err(dirty_checkout(&git, name));
    }
    let mut visited = BTreeMap::new();
    for (file, revision) in expected {
        let path = file.as_path().to_str().ok_or("staged Git source path is not UTF-8")?;
        if files.get(path).is_none_or(|(actual, _)| actual.as_bytes() != revision) {
            return Err("staged source revision does not match the installed Git base".to_owned());
        }
        visited.insert(file.clone(), *revision);
    }
    Ok(Some(InstalledGitBase {
        locked: pin.input_identity(), patches, project: git.root.clone(), package: name.to_owned(),
        checkout: git.root.join(DEPS).join(name), visited,
        revision: pin.revision.clone(), package_path: pin.source.package_path.clone(),
        tree, source_hash: pin.source_hash.clone(),
    }))
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
        format!("{}\n", self.seal(None))
    }

    fn seal(&self, overlay: Option<(PatchStackId, WorkingTreeId, &FrozenRoutes)>) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"wosy-package-install-v1\0");
        for field in [self.name.as_bytes(), self.input.as_bytes(), self.project, self.checkout] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        if let Some((stack, baseline, routes)) = overlay {
            hasher.update(b"wosy-installed-overlay-v1\0");
            hasher.update(stack.as_bytes());
            hasher.update(&baseline.0);
            routes.hash(&mut hasher);
        }
        hasher.finalize().to_hex().to_string()
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

    fn fetched(
        &self,
        git: &GitProject,
        slot: &str,
        repository: PathBuf,
        checkout: &Path,
        source: &GitSource,
        revision: &GitOid,
    ) -> Result<PristinePackage, String> {
        let selected_oid = if source.package_path == "." {
            let selector = format!("{}^{{tree}}", revision.as_str());
            read_oid(&git.directory, &self.handle, &repository, &selector)?
        } else {
            let path = Path::new(&source.package_path);
            let selected = checked_output(
                git_command(&git.directory, &self.handle, Some(&repository))?
                    .args(["--literal-pathspecs", "ls-tree", "-z", "--full-tree", revision.as_str(), "--"])
                    .arg(path),
                "ls-tree",
            )?;
            git_wire::selected_tree(&selected.stdout, path)?
        };
        let object_type = checked_output(
            git_command(&git.directory, &self.handle, Some(&repository))?
                .args(["cat-file", "-t", selected_oid.as_str()]),
            "cat-file",
        )?;
        let tree = GitTreeOid::from_cat_file_type(selected_oid, &object_type.stdout)?;
        let identity = materialize_tree(&git.directory, &self.handle, &repository, checkout, &tree)?;
        Ok(PristinePackage { checkout: self.checkout(slot)?, identity, tree, repository })
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

pub(super) fn git_command(project: &GitDirectory, stage: &GitDirectory, repository: Option<&Path>) -> Result<Command, String> {
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

fn read_oid(project: &GitDirectory, stage: &GitDirectory, repository: &Path, reference: &str) -> Result<GitOid, String> {
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
    if read_oid(project, stage, repository, "FETCH_HEAD^{commit}")? != *revision {
        return Err(format!("fetch did not provide the pinned commit {}", revision.as_str()));
    }
    Ok(())
}

pub(super) fn materialize_tree(
    project: &GitDirectory,
    stage: &GitDirectory,
    repository: &Path,
    checkout: &Path,
    tree: &GitTreeOid,
) -> Result<TreeIdentity, String> {
    let mut command = git_command(project, stage, Some(repository))?;
    command.args(["ls-tree", "-r", "-z", "--full-tree"]).arg(tree.as_str()).arg("--");
    let indexed = checked_output(&mut command, "ls-tree")?;
    fs::create_dir_all(checkout)
        .map_err(|error| format!("cannot create package staging directory: {error}"))?;
    let mut digests = BTreeMap::new();
    // Exact blobs are written and hashed together. No checkout filters, archive
    // exclusions, or later immutable-tree capture participate.
    for entry in git_wire::blobs(&indexed.stdout)? {
        let entry = entry?;
        let path = entry.path.as_path();
        let destination = checkout.join(path);
        fs::create_dir_all(destination.parent().expect("checkout file parent"))
            .map_err(|error| format!("cannot create parent for {}: {error}", path.display()))?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(&destination)
            .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
        let mut command = git_command(project, stage, Some(repository))?;
        command.args(["cat-file", "blob", entry.oid.as_str()]);
        let contents = stream_blob(command, &mut file)
            .map_err(|error| format!("cannot extract {}: {error}", path.display()))?;
        restore_mode(&destination, entry.executable)
            .map_err(|error| format!("cannot set permissions for {}: {error}", path.display()))?;
        let path = path.to_str().expect("Git adapter requires UTF-8 paths");
        digests.insert(path.to_owned(), (contents, entry.executable));
    }
    Ok(identity_from_files(&digests))
}

fn copy_hashed(mut source: impl Read, mut output: impl Write) -> std::io::Result<blake3::Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut chunk = [0; 32 * 1024];
    loop {
        let n = source.read(&mut chunk)?;
        if n == 0 {
            return Ok(hasher.finalize());
        }
        output.write_all(&chunk[..n])?;
        hasher.update(&chunk[..n]);
    }
}

fn stream_blob(mut command: Command, file: &mut File) -> Result<blake3::Hash, String> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| error.to_string())?;
    let mut stdout = child.stdout.take().expect("piped blob");
    let mut stderr = child.stderr.take().expect("piped Git diagnostics");
    std::thread::scope(|scope| {
        let diagnostics = scope.spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        });
        let received = copy_hashed(&mut stdout, file);
        drop(stdout);
        let status = child.wait().map_err(|error| error.to_string())?;
        let diagnostics = diagnostics.join().map_err(|_| "Git diagnostic reader failed".to_owned())?
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("git cat-file failed: {}", String::from_utf8_lossy(&diagnostics)));
        }
        received.map_err(|error| error.to_string())
    })
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

fn collect_tree_files(
    root: &GitDirectory,
    mut visit: impl FnMut(&str, File, bool) -> Result<(), String>,
) -> Result<(), String> {
    fn collect(
        root: &GitDirectory,
        directory: &GitDirectory,
        relative: &Path,
        visit: &mut impl FnMut(&str, File, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let read_error = |error| format!("cannot read Git dependency path {}: {error}", relative.display());
        for name in directory.entries().map_err(read_error)? {
            let path = relative.join(name);
            let file = root.open(&path)?
                .ok_or_else(|| format!("Git dependency path {} changed while reading", path.display()))?;
            let metadata = file.metadata().map_err(read_error)?;
            if metadata.is_dir() {
                let child = GitDirectory::new(file)?;
                collect(root, &child, &path, visit)?;
            } else if metadata.is_file() {
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o100 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                let path = path.to_str().ok_or_else(|| "non-UTF-8 Git dependency path".to_owned())?;
                visit(path, file, executable)?;
            } else {
                return Err(format!("{} is not a regular dependency file", path.display()));
            }
        }
        Ok(())
    }

    collect(root, root, Path::new(""), &mut visit)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct WorkingTreeId(pub(super) [u8; 32]);

pub(super) struct TreeIdentity {
    pub(super) original_v1: String,
    pub(super) working: WorkingTreeId,
}

fn tree_file_digests(root: &GitDirectory) -> Result<BTreeMap<String, (blake3::Hash, bool)>, String> {
    let mut files = BTreeMap::new();
    collect_tree_files(root, |path, file, executable| {
        let mut content = blake3::Hasher::new();
        content.update_reader(file)
            .map_err(|error| format!("cannot read Git dependency path {path}: {error}"))?;
        files.insert(path.to_owned(), (content.finalize(), executable));
        Ok(())
    })?;
    Ok(files)
}

pub(super) fn tree_identity(root: &GitDirectory) -> Result<TreeIdentity, String> {
    Ok(identity_from_files(&tree_file_digests(root)?))
}

fn identity_from_files(files: &BTreeMap<String, (blake3::Hash, bool)>) -> TreeIdentity {
    let mut original = blake3::Hasher::new();
    original.update(b"wosy-package-source-v1\0");
    let mut working = blake3::Hasher::new();
    working.update(b"wosy-working-package-v1\0");
    for (path, (content, executable)) in files {
        for digest in [&mut original, &mut working] {
            digest.update(&(path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
            digest.update(content.as_bytes());
        }
        working.update(&[u8::from(*executable)]);
    }
    TreeIdentity {
        original_v1: original.finalize().to_hex().to_string(),
        working: WorkingTreeId(*working.finalize().as_bytes()),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn installation_receipts_keep_legacy_readers_and_bind_the_exact_verified_tree() {
        let mut stage = None;
        let (temp, project) = GitDirectory::project(&std::env::temp_dir()).unwrap();
        let fixture = get_stage(&mut stage, &temp, &project).unwrap();
        let root = &fixture.handle;
        let package = "sample";
        let checkout_path = fixture.path.join("deps/sample");
        fs::create_dir_all(&checkout_path).unwrap();
        fs::create_dir_all(fixture.path.join("deps/.wosy-pins")).unwrap();
        let receipt_path = fixture.path.join("deps/.wosy-pins/sample");
        let checkout = root.directory(Path::new("deps/sample")).unwrap().unwrap();
        let pin = LockedPackage {
            source: GitSource { origin: "local-fixture".to_owned(), package_path: ".".to_owned() },
            revision: GitOid::parse(&"1".repeat(40)).unwrap(), source_hash: "2".repeat(64),
        };
        let tree = GitTreeOid::from_cat_file_type(GitOid::parse(&"3".repeat(40)).unwrap(), b"tree\n").unwrap();
        let stack = PatchStack::empty().identity();
        let baseline = WorkingTreeId([4; 32]);
        let routes = FrozenRoutes::capture(&BTreeMap::new());
        let observe = || verify_installed(&fixture.display, root, package, &pin);
        fs::write(&receipt_path, InstallEvidence::new(root, &checkout, package, &pin).receipt()).unwrap();
        let plain = observe().unwrap().unwrap();
        assert!(plain.base_tree.is_none() && plain.baseline.is_none());
        let v1 = overlay_receipt(root, &checkout, package, &pin, stack, baseline, &routes, None).unwrap();
        fs::write(&receipt_path, &v1).unwrap();
        let legacy = observe().unwrap().unwrap();
        assert!(legacy.base_tree.is_none() && legacy.baseline == Some(baseline));
        let extended = overlay_receipt(root, &checkout, package, &pin, stack, baseline, &routes, Some(&tree)).unwrap();
        let extended_parent: serde_json::Value = serde_json::from_str(&extended).unwrap();
        let legacy_parent: serde_json::Value = serde_json::from_str(&v1).unwrap();
        assert_eq!(extended_parent["version"], legacy_parent["version"]);
        assert_eq!(extended_parent["seal"], legacy_parent["seal"]);
        fs::write(&receipt_path, &extended).unwrap();
        assert_eq!(observe().unwrap().unwrap().base_tree.unwrap().as_str(), tree.as_str());
        fs::write(&receipt_path, extended.replace(tree.as_str(), &"9".repeat(40))).unwrap();
        assert!(observe().unwrap().unwrap().base_tree.is_none());
    }

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
    fn failed_lock_exchange_restores_replaced_and_retired_checkouts_and_receipts() {
        let mut outer = None;
        let (temp_path, temp) = GitDirectory::project(&std::env::temp_dir()).unwrap();
        let root = get_stage(&mut outer, &temp_path, &temp).unwrap();
        let live_deps_path = root.path.join(DEPS);
        fs::create_dir_all(live_deps_path.join(PINS)).unwrap();
        fs::create_dir(live_deps_path.join("sample")).unwrap();
        fs::create_dir(live_deps_path.join("obsolete")).unwrap();
        fs::write(live_deps_path.join("sample/source.w"), b"previous live source").unwrap();
        fs::write(live_deps_path.join(PINS).join("sample"), b"previous receipt").unwrap();
        fs::write(live_deps_path.join("obsolete/source.w"), b"obsolete clean source").unwrap();
        fs::write(live_deps_path.join(PINS).join("obsolete"), b"obsolete receipt").unwrap();
        fs::write(root.path.join("wosy.lock"), b"previous lock").unwrap();
        let live_deps = root.handle.directory(Path::new(DEPS)).unwrap().unwrap();
        let live_sample = live_deps.directory(Path::new("sample")).unwrap().unwrap();
        let old_tree = tree_identity(&live_sample).unwrap().working;
        let obsolete = live_deps.directory(Path::new("obsolete")).unwrap().unwrap();
        let old_obsolete = tree_identity(&obsolete).unwrap().working;
        let git = GitProject { root: root.display.clone(), directory: root.handle.clone() };
        let mut retirements = [Retirement {
            name: "obsolete".to_owned(),
            previous: Some(PreviousCheckout { checkout: obsolete, baseline: old_obsolete }),
            moved_checkout: false,
            moved_receipt: false,
        }];

        let mut candidate = None;
        let stage = get_stage(&mut candidate, &root.display, &root.handle).unwrap();
        let prepared_path = stage.path.join(DEPS);
        fs::create_dir_all(prepared_path.join(PINS)).unwrap();
        fs::create_dir(prepared_path.join("sample")).unwrap();
        fs::write(prepared_path.join("sample/source.w"), b"prepared successor").unwrap();
        fs::write(prepared_path.join(PINS).join("sample"), b"prepared receipt").unwrap();
        let prepared = stage.handle.directory(Path::new(DEPS)).unwrap().unwrap();
        let sample = prepared.directory(Path::new("sample")).unwrap().unwrap();
        let working = tree_identity(&sample).unwrap().working;
        let mut installs = [PendingInstall {
            name: "sample".to_owned(),
            expected_checkout: sample,
            expected_receipt: "prepared receipt".to_owned(),
            expected_working: working,
            previous_checkout: Some(PreviousCheckout { checkout: live_sample, baseline: old_tree }),
            receipt_only: false,
            destination: live_deps_path.join("sample"),
            previous_receipt: false,
            receipt_published: false,
            checkout_published: false,
        }];
        let result = publish_updates(
            &root.handle, Path::new(DEPS), &live_deps, &stage.handle, &prepared, stage,
            &mut installs, GitDirectory::rename, || {
                publish_retirements(&git, stage, &mut retirements, || {
                    assert_eq!(fs::read(live_deps_path.join("sample/source.w")).unwrap(), b"prepared successor");
                    assert!(!live_deps_path.join("obsolete").exists());
                    assert!(!live_deps_path.join(PINS).join("obsolete").exists());
                    stage.handle.exchange(Path::new("missing-lock"), &root.handle, Path::new("wosy.lock"))
                        .map_err(|error| error.to_string())
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(live_deps_path.join("sample/source.w")).unwrap(), b"previous live source");
        assert_eq!(fs::read(live_deps_path.join(PINS).join("sample")).unwrap(), b"previous receipt");
        assert_eq!(fs::read(live_deps_path.join("obsolete/source.w")).unwrap(), b"obsolete clean source");
        assert_eq!(fs::read(live_deps_path.join(PINS).join("obsolete")).unwrap(), b"obsolete receipt");
        assert_eq!(fs::read(root.path.join("wosy.lock")).unwrap(), b"previous lock");
    }

    #[test]
    fn publication_restores_checkout_and_old_receipts_when_later_rename_fails() {
        test_checkout_rename_rollback(true);
        test_checkout_rename_rollback(false);
    }

    fn test_checkout_rename_rollback(had_receipts: bool) {
        let mut stage = None;
        let (root, project) = GitDirectory::project(&std::env::temp_dir()).unwrap();
        let staging = get_stage(&mut stage, &root, &project).unwrap();
        let deps_path = staging.path.join("existing");
        let receipts_path = deps_path.join(PINS);
        let existing = deps_path.join("unchanged");
        fs::create_dir_all(&existing).unwrap();
        if had_receipts {
            fs::create_dir(&receipts_path).unwrap();
            fs::write(receipts_path.join("first"), "previous receipt").unwrap();
        }
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
                expected_working: WorkingTreeId([0; 32]),
                previous_checkout: None,
                receipt_only: false,
                destination: deps_path.join(name),
                previous_receipt: false,
                receipt_published: false,
                checkout_published: false,
            }
        });
        let expected = tree_identity(&installs[0].expected_checkout).unwrap().working;
        for install in &mut installs {
            install.expected_working = expected;
        }
        let error = publish_updates(
            &stage, Path::new("existing"), &deps, &stage, &stage_deps, staging,
            &mut installs, |from, name, into, dest| {
            if from == &stage_deps && name == Path::new("second") {
                Err(std::io::Error::other("simulated checkout rename failure"))
            } else {
                from.rename(name, into, dest)
            }
        }, || Ok(())).unwrap_err();

        assert!(error.contains("simulated checkout rename failure"));
        assert!(!error.contains("rollback incomplete"));
        assert!(!deps_path.join("first").exists());
        assert!(!deps_path.join("second").exists());
        if had_receipts {
            assert_eq!(fs::read_to_string(receipts_path.join("first")).unwrap(), "previous receipt");
        } else {
            assert!(!receipts_path.exists());
        }
        assert!(!receipts_path.join("second").exists());
        assert_eq!(fs::read_to_string(existing.join("edit")).unwrap(), "keep user edit");
    }
}
