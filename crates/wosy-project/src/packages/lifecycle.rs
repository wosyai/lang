use super::git_sync::{dirty_checkout, overlay_receipt, tree_identity, FrozenRoutes, PreviousCheckout, PristinePackage, Staging, PINS};
use super::patches::{GitOverlay, PatchId, PatchStack, PatchStackId, PATCHES};
use super::snapshot::MANIFEST;
use super::{graph, load_git, load_local_declarations, GitDirectory, GitProject, LockedPackage, PackageLock, ProjectSnapshot, DEPS, LOCKFILE};
use crate::{project_dependencies, scalar_string};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::Item;

fn root_graph(snapshot: &ProjectSnapshot) -> Result<(String, String, graph::Declarations), String> {
    let manifest = snapshot.manifest()?;
    let document = toml_edit::ImDocument::parse(manifest.clone()).map_err(|error| error.to_string())?;
    let package = document.get("package").and_then(Item::as_table)
        .ok_or_else(|| format!("package is missing from {MANIFEST}"))?;
    let name = scalar_string(package, "name")?;
    let declarations = project_dependencies(snapshot.root(), &name, &document)?;
    Ok((manifest, name, declarations))
}

pub fn snapshot_dependencies(root: &Path) -> Result<(), String> {
    let snapshot = ProjectSnapshot::writer(root)?;
    let (manifest, root_name, declarations) = root_graph(&snapshot)?;
    let mut git = None;
    let mut targets = BTreeMap::new();
    let mut staging = Staging::default();
    let closure = graph::resolve(snapshot.root(), &root_name, &declarations, |binding, parent, declaration| {
        match binding {
            graph::Binding::Local(_) => load_local_declarations(parent, declaration),
            graph::Binding::Git(requirement) => {
                let name = declaration.name.as_str();
                let pin = snapshot.pin_for(name, requirement)?;
                let git = git_project(&snapshot, &mut git)?;
                let installed = git.installed(name, pin)?
                    .ok_or_else(|| format!("dependency {name} is not materialized; run wosy dep sync"))?;
                let stack = PatchStack::load(&git.directory, name)?;
                installed.require_stack(name, stack.identity())?;
                // The previously resolved routing is independent of edits
                // (even edits already captured in a patch but not locked).
                let original;
                let routes = if let Some(routes) = installed.routes.clone() {
                    original = None;
                    routes
                } else {
                    // Legacy receipts proved only unpatched originals.
                    let fetched = staging.pristine(git, name, pin)?;
                    let loaded = load_git(snapshot.root(), declaration, fetched.checkout.clone(), pin, None)?;
                    original = Some(fetched);
                    FrozenRoutes::capture(&loaded.dependencies)
                };
                let root = snapshot.root().join(DEPS).join(name);
                let dependencies = routes.declarations(&root, name);
                targets.insert(name.to_owned(), (pin, installed, stack, routes, original));
                Ok(graph::LoadedPackage { root, source: None, dependencies, revision: Some(pin.revision.clone()) })
            }
        }
    })?;
    snapshot.check_reached(&closure.git_packages)?;
    let Some(git) = git else { return Ok(()); };
    let mut drafts = Vec::new();
    for (name, (pin, installed, mut stack, routes, original)) in targets {
        let frozen = staging.freeze_working(&git, &name, &installed.checkout)?;
        let baseline = installed.baseline.or_else(|| original.as_ref().map(|pristine| pristine.identity.working));
        if baseline == Some(frozen.working) {
            continue;
        }
        let pristine: PristinePackage = match original {
            Some(pristine) => pristine,
            None => staging.pristine(&git, &name, pin)?,
        };
        let repo = GitOverlay::for_pristine(&git, staging.workspace(&git)?, &pristine);
        let old_tree = repo.apply(&name, &pristine.tree, &stack)?;
        let working = repo.capture(&frozen)?;
        let Some(delta) = repo.difference(&old_tree, &working)? else { continue; };
        let old_stack = stack.identity();
        let id = stack.append(delta)?;
        let receipt = overlay_receipt(
            &git.directory, &installed.checkout, &name, pin, stack.identity(), frozen.working, &routes,
        )?;
        let stage = staging.workspace(&git)?;
        drafts.push(stage_patch(stage, name, installed.checkout, old_stack, id, stack.bytes(id), &receipt)?);
    }
    if drafts.is_empty() {
        return Ok(());
    }
    snapshot.require_unchanged(&manifest)?;
    publish_patches(&git, staging.workspace(&git)?, &mut drafts)
}

struct PatchDraft {
    name: String,
    id: PatchId,
    checkout: GitDirectory,
    old_stack: PatchStackId,
    patch_stage: GitDirectory,
    receipt_stage: GitDirectory,
    published: bool,
    receipt_changed: bool,
}

fn stage_patch(
    stage: &super::git_sync::Stage,
    name: String,
    checkout: GitDirectory,
    old_stack: PatchStackId,
    id: PatchId,
    delta: &[u8],
    receipt: &str,
) -> Result<PatchDraft, String> {
    let patch_dir = Path::new("new-patches").join(&name);
    let receipt_dir = Path::new("new-patch-receipts");
    for directory in [&patch_dir, receipt_dir] {
        fs::create_dir_all(stage.path.join(directory)).map_err(|error| format!("cannot stage patch outputs: {error}"))?;
    }
    fs::write(stage.path.join(&patch_dir).join(id.filename()), delta)
        .map_err(|error| format!("cannot stage patch for {name}: {error}"))?;
    fs::write(stage.path.join(receipt_dir).join(&name), receipt)
        .map_err(|error| format!("cannot stage patch provenance for {name}: {error}"))?;
    let patch_stage = stage.handle.directory(&patch_dir)?.expect("created patch stage");
    let receipt_stage = stage.handle.directory(receipt_dir)?.expect("created patch receipt stage");
    Ok(PatchDraft { name, id, checkout, old_stack, patch_stage, receipt_stage, published: false, receipt_changed: false })
}

fn publish_patches(git: &GitProject, stage: &super::git_sync::Stage, drafts: &mut [PatchDraft]) -> Result<(), String> {
    let deps = git.directory.directory(Path::new(DEPS))?
        .ok_or_else(|| "installed deps disappeared".to_owned())?;
    let receipts = deps.directory(Path::new(PINS))?
        .ok_or_else(|| "installed dependency provenance disappeared".to_owned())?;
    for draft in drafts.iter() {
        if deps.directory(Path::new(&draft.name))?.as_ref() != Some(&draft.checkout)
            || PatchStack::load(&git.directory, &draft.name)?.identity() != draft.old_stack
        {
            return Err(format!("dependency {} or its authored patch stack changed during snapshot", draft.name));
        }
    }
    let mut made = Vec::new();
    let mut destinations = BTreeMap::new();
    let publish = (|| {
        let patches = patch_directory(&git.directory, Path::new(PATCHES), &mut made)?;
        for draft in drafts.iter_mut() {
            let name = Path::new(&draft.name);
            let package = patch_directory(&patches, name, &mut made)?;
            destinations.insert(draft.name.clone(), package.clone());
            let filename = draft.id.filename();
            let filename = Path::new(&filename);
            draft.patch_stage.rename(filename, &package, filename)
                .map_err(|error| format!("cannot publish patch {}: {error}", filename.display()))?;
            draft.published = true;
            draft.receipt_stage.exchange(name, &receipts, name)
                .map_err(|error| format!("cannot publish patch provenance for {}: {error}", draft.name))?;
            draft.receipt_changed = true;
        }
        Ok(())
    })();
    let Err(cause) = publish else { return Ok(()); };
    let mut failed = Vec::new();
    for draft in drafts.iter().rev() {
        let name = Path::new(&draft.name);
        if draft.receipt_changed {
            if let Err(error) = draft.receipt_stage.exchange(name, &receipts, name) {
                failed.push(format!("{} provenance: {error}", draft.name));
                continue;
            }
        }
        if draft.published {
            let filename = draft.id.filename();
            let filename = Path::new(&filename);
            if let Err(error) = destinations[&draft.name].rename(filename, &draft.patch_stage, filename) {
                failed.push(format!("{} patch: {error}", draft.name));
            }
        }
    }
    for (parent, name) in made.into_iter().rev() {
        let _ = parent.remove_empty_directory(&name);
    }
    if failed.is_empty() {
        Err(cause)
    } else {
        stage.preserve.set(true);
        Err(format!("{cause}; rollback incomplete: {}; recovery under {}", failed.join("; "), stage.display.display()))
    }
}

fn patch_directory(
    parent: &GitDirectory,
    name: &Path,
    made: &mut Vec<(GitDirectory, PathBuf)>,
) -> Result<GitDirectory, String> {
    if let Some(directory) = parent.directory(name)? {
        return Ok(directory);
    }
    parent.mkdir(name).map_err(|error| format!("cannot create patch directory {}: {error}", name.display()))?;
    made.push((parent.clone(), name.to_path_buf()));
    parent.directory(name)?.ok_or_else(|| format!("patch directory {} disappeared", name.display()))
}

pub fn update_dependencies(root: &Path) -> Result<(), String> {
    let snapshot = ProjectSnapshot::writer(root)?;
    let (manifest, root_name, declarations) = root_graph(&snapshot)?;
    let mut git = None;
    let mut original = Staging::default();
    let mut new = Staging::default();
    let mut pins = PackageLock::default();

    // The current lock is the old generation's exact Git set. Only one
    // walk plans the successor, from freshly staged authored overlays and
    // the current local declarations. Live Git manifests are never a solver
    // input, and local manifests aren't read in a second "old" traversal.
    let new_closure = graph::resolve(snapshot.root(), &root_name, &declarations, |binding, parent, declaration| {
        match binding {
            graph::Binding::Local(_) => load_local_declarations(parent, declaration),
            graph::Binding::Git(requirement) => {
                let name = declaration.name.as_str();
                let source = &requirement.source;
                let old_lock = snapshot.parsed_lock()?
                    .ok_or_else(|| format!("Git dependency updates require {LOCKFILE}"))?;
                let old = old_lock.packages.get(name);
                if old.is_some_and(|record| record.source != *source) {
                    return Err(format!("dependency {name} changes its declared Git source during update"));
                }
                let git = git_project(&snapshot, &mut git)?;
                let (pristine, revision) = new.fetch_requirement(git, name, requirement, true)?;
                let pin = match old {
                    Some(old) if old.revision == revision => {
                        pristine.require_pin_hash(name, old)?;
                        old.clone()
                    }
                    previous => {
                        if let Some(previous) = previous {
                            new.require_forward(git, &pristine, source, &previous.revision, &revision)?;
                        }
                        LockedPackage {
                            source: source.clone(), revision, source_hash: pristine.identity.original_v1.clone(),
                        }
                    }
                };
                let stack = PatchStack::load(&git.directory, name)?;
                let (candidate, working) = new.replay(git, name, &pristine, &stack)?;
                let loaded = load_git(snapshot.root(), declaration, candidate.clone(), &pin, None)?;
                let routes = FrozenRoutes::capture(&loaded.dependencies);
                let installed = old.map(|pin| git.installed(name, pin)).transpose()?.flatten();
                if let Some(current) = installed.as_ref().filter(|current| {
                    current.stack == stack.identity() && old.is_some_and(|record| *record == pin)
                }) {
                    // Advance the recorded, resolved routing (for example
                    // after a snapshot introduced a dependency) without
                    // replacing working bytes already at this pin and stack.
                    if current.routes.as_ref() != Some(&routes) {
                        new.refresh_receipt(
                            git, name, &pin, &stack, current.checkout.clone(), working, &routes,
                        )?;
                    }
                } else {
                    let previous = if let Some(old) = old {
                        installed.map(|current| -> Result<_, String> {
                            let baseline = match current.baseline {
                                Some(baseline) => baseline,
                                None => original.pristine(git, name, old)?.identity.working,
                            };
                            if tree_identity(&current.checkout)?.working != baseline {
                                return Err(dirty_checkout(git, name));
                            }
                            Ok(PreviousCheckout { checkout: current.checkout, baseline })
                        }).transpose()?
                    } else {
                        None
                    };
                    new.prepare_install(git, name, &pin, &stack, candidate, working, previous, &routes)?;
                }
                pins.packages.insert(name.to_owned(), pin);
                Ok(loaded)
            }
        }
    })?;
    pins.check_reached(&new_closure.git_packages)?;
    // A local-only project with an unrelated legacy lock remains read-only.
    let current_lock = match snapshot.parsed_lock() {
        Ok(Some(lock)) if !lock.packages.is_empty() || !pins.packages.is_empty() => lock,
        Ok(_) | Err(_) if pins.packages.is_empty() => return Ok(()),
        Err(error) => return Err(error),
        _ => return Err(format!("Git dependency updates require {LOCKFILE}")),
    };
    let git = git_project(&snapshot, &mut git)?;
    for (name, pin) in &current_lock.packages {
        if !pins.packages.contains_key(name) {
            let previous = git.installed(name, pin)?.map(|installed| -> Result<_, String> {
                let baseline = match installed.baseline {
                    Some(baseline) => baseline,
                    None => original.pristine(git, name, pin)?.identity.working,
                };
                Ok(PreviousCheckout { checkout: installed.checkout, baseline })
            }).transpose()?;
            new.retire(git, name, previous)?;
        }
    }
    let lock_changed = &pins != current_lock;
    if lock_changed {
        let stage = new.workspace(git)?;
        fs::write(stage.path.join("updated-lock"), pins.render())
            .map_err(|error| format!("cannot stage {LOCKFILE}: {error}"))?;
    }
    new.publish_with(git, |stage| {
        snapshot.require_unchanged(&manifest)?;
        if lock_changed {
            let stage = stage.expect("prepared lock stage");
            stage.handle.exchange(Path::new("updated-lock"), &git.directory, Path::new(LOCKFILE))
                .map_err(|error| format!("cannot publish {LOCKFILE}: {error}"))?;
        }
        Ok(())
    })
}

fn git_project<'a>(snapshot: &ProjectSnapshot, project: &'a mut Option<GitProject>) -> Result<&'a GitProject, String> {
    if project.is_none() {
        *project = Some(snapshot.git_project()?);
    }
    Ok(project.as_ref().expect("opened Git project"))
}
