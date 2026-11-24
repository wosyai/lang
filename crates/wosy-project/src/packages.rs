use super::{
    dependency_document, dependency_package_table, load_dependency_package, parse_dependencies, DependencyDeclaration, DependencySource, GitSource, ResolvedPackage,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use toml_edit::{Item, Table};

mod add;
mod anchored;
mod git_sync;
mod git_wire;
mod graph;
mod init;
mod lifecycle;
mod patches;
mod snapshot;

pub(crate) use snapshot::ProjectSnapshot;

pub use add::add_dependency;
pub use anchored::GitDirectory;
pub use init::init_project;
pub use lifecycle::{snapshot_dependencies, update_dependencies};
pub use patches::PatchStackId;
pub use git_sync::sync_dependencies;

const LOCKFILE: &str = "wosy.lock";
const DEPS: &str = "deps";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PackageLock {
    pub(super) packages: BTreeMap<String, LockedPackage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LockedPackage {
    source: GitSource,
    revision: git_wire::GitOid,
    source_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LockedInputId([u8; 32]);

impl LockedInputId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl LockedPackage {
    fn input_identity(&self) -> LockedInputId {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"wosy-locked-input-v1\0");
        for field in [
            self.source.origin.as_str(),
            self.revision.as_str(),
            self.source.package_path.as_str(),
            self.source_hash.as_str(),
        ] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field.as_bytes());
        }
        LockedInputId(*hasher.finalize().as_bytes())
    }
}

impl PackageLock {
    pub fn parse(content: String) -> Result<Self, String> {
        let document = toml_edit::ImDocument::parse(content).map_err(|error| error.to_string())?;
        if document.get("version").and_then(Item::as_integer) != Some(1) {
            return Err("expected lockfile version 1".to_owned());
        }
        if document.iter().any(|(key, _)| !matches!(key, "version" | "packages")) {
            return Err("unsupported lockfile field".to_owned());
        }
        let mut lock = Self::default();
        let Some(packages) = document.get("packages") else {
            return Ok(lock);
        };
        let packages = packages
            .as_table()
            .ok_or_else(|| "packages must be a table".to_owned())?;
        for (name, item) in packages {
            if !valid_package_name(name) {
                return Err(format!("invalid package name {name}"));
            }
            let table = item
                .as_table()
                .ok_or_else(|| format!("package {name} must be a table"))?;
            if table
                .iter()
                .any(|(key, _)| !matches!(key, "git" | "rev" | "path" | "source_hash"))
            {
                return Err(format!("package {name} contains an unsupported field"));
            }
            let field = |key, valid: fn(&str) -> bool| {
                table
                    .get(key)
                    .and_then(Item::as_str)
                    .filter(|value| valid(value))
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| format!("package {name} has invalid {key}"))
            };
            lock.packages.insert(
                name.to_owned(),
                LockedPackage {
                    source: GitSource {
                        origin: field("git", valid_origin)?,
                        package_path: normalize_package_path(
                            table.get("path").and_then(Item::as_str)
                                .ok_or_else(|| format!("package {name} has invalid path"))?,
                        ).map_err(|_| format!("package {name} has invalid path"))?,
                    },
                    revision: table.get("rev").and_then(Item::as_str)
                        .and_then(|text| git_wire::GitOid::parse(text).ok())
                        .ok_or_else(|| format!("package {name} has invalid rev"))?,
                    source_hash: field("source_hash", |hash| hash.len() == 64 && is_lower_hex(hash))?,
                },
            );
        }
        Ok(lock)
    }

    pub fn render(&self) -> String {
        use toml_edit::{value, DocumentMut};
        let mut document = DocumentMut::new();
        document["version"] = value(1);
        let mut packages = Table::new();
        packages.set_implicit(true);
        for (name, pin) in &self.packages {
            let mut entry = Table::new();
            entry["git"] = value(&pin.source.origin);
            entry["rev"] = value(pin.revision.as_str());
            entry["path"] = value(&pin.source.package_path);
            entry["source_hash"] = value(&pin.source_hash);
            packages.insert(name, Item::Table(entry));
        }
        if !self.packages.is_empty() {
            document["packages"] = Item::Table(packages);
        }
        document.to_string()
    }

    fn pin_for(&self, name: &str, source: &GitSource) -> Result<&LockedPackage, String> {
        let pin = self.packages.get(name)
            .ok_or_else(|| format!("dependency {name} has no entry in {LOCKFILE}"))?;
        if pin.source != *source {
            return Err(format!("dependency {name} disagrees with {LOCKFILE}"));
        }
        Ok(pin)
    }

    fn pin_for_requirement(&self, name: &str, requirement: &graph::GitRequirement) -> Result<&LockedPackage, String> {
        let pin = self.pin_for(name, &requirement.source)?;
        requirement.require_revision(name, &pin.revision)?;
        Ok(pin)
    }

    fn check_reached(&self, reached: &BTreeSet<String>) -> Result<(), String> {
        if let Some(name) = self.packages.keys().find(|name| !reached.contains(*name)) {
            return Err(format!("{LOCKFILE} contains undeclared Git dependency {name}"));
        }
        Ok(())
    }


}

struct GitProject {
    root: PathBuf,
    directory: GitDirectory,
}

impl GitProject {
    fn installed(&self, name: &str, pin: &LockedPackage) -> Result<Option<git_sync::Installation>, String> {
        git_sync::verify_installed(&self.root, &self.directory, name, pin)
    }

}

pub(super) fn resolve_installed(
    snapshot: &ProjectSnapshot,
    root: &Path,
    package: &str,
    dependencies: &BTreeMap<String, DependencyDeclaration>,
) -> Result<BTreeMap<String, ResolvedPackage>, String> {
    let mut git = None;
    let resolution = graph::resolve(root, package, dependencies, |binding, parent, declaration| {
        let name = declaration.name.as_str();
        match binding {
            graph::Binding::Local(_) => load_local(parent, declaration, None),
            graph::Binding::Git(requirement) => {
                let pin = snapshot.pin_for(name, requirement)?;
                if git.is_none() {
                    git = Some(snapshot.git_project()?);
                }
                let git = git.as_ref().expect("opened Git project");
                let stack = patches::PatchStack::load(&git.directory, name)?.identity();
                let installed = git.installed(name, pin)?
                    .ok_or_else(|| format!("dependency {name} is not materialized; run wosy dep sync"))?;
                installed.require_stack(name, stack)?;
                let mut loaded = load_git(root, declaration, installed.checkout, pin, None)?;
                loaded.source.as_mut().expect("Git source binding").patch_stack_id = Some(stack);
                Ok(loaded)

            }
        }
    })?;
    snapshot.check_reached(&resolution.git_packages)?;
    Ok(resolution.packages)
}

fn load_local(
    parent: &Path,
    declaration: &DependencyDeclaration,
    inspected: Option<toml_edit::ImDocument<String>>,
) -> Result<graph::LoadedPackage, String> {
    let DependencySource::Local { path } = &declaration.source else {
        return Err("expected a local package declaration".to_owned());
    };
    load_dependency_package(parent.join(path), declaration, None, inspected)
        .map(Into::into)
        .map_err(|error| error.diagnostics[0].message.clone())
}

fn load_local_declarations(
    parent: &Path,
    declaration: &DependencyDeclaration,
) -> Result<graph::LoadedPackage, String> {
    let DependencySource::Local { path } = &declaration.source else {
        return Err("expected a local package declaration".to_owned());
    };
    let root = parent.join(path);
    let error = |error: crate::DependencyResolutionError| error.diagnostics[0].message.clone();
    let document = dependency_document(&root, declaration, None, None).map_err(error)?;
    dependency_package_table(&document, declaration).map_err(error)?;
    let dependencies = parse_dependencies(&root, &declaration.name, document.get("dependencies"))
        .map_err(error)?;
    Ok(graph::LoadedPackage { root, source: None, dependencies, revision: None })
}

fn load_git(
    consuming_project: &Path,
    declaration: &DependencyDeclaration,
    checkout: GitDirectory,
    pin: &LockedPackage,
    inspected: Option<toml_edit::ImDocument<String>>,
) -> Result<graph::LoadedPackage, String> {
    let path = consuming_project.join(DEPS).join(&declaration.name);
    let (mut package, dependencies) = load_dependency_package(path, declaration, Some(checkout), inspected)
        .map_err(|error| error.diagnostics[0].message.clone())?;
    package.git_pin_identity = Some(pin.input_identity());
    let mut loaded: graph::LoadedPackage = (package, dependencies).into();
    loaded.revision = Some(pin.revision.clone());
    Ok(loaded)
}

pub(super) fn valid_origin(origin: &str) -> bool {
    !origin.is_empty() && !origin.as_bytes().contains(&0)
}

pub(super) fn valid_package_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_lower_hex(value: &str) -> bool {
    value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn normalize_package_path(path: &str) -> Result<String, String> {
    let relative = crate::PackageRelativePath::new(PathBuf::from(path))?;
    let normalized: PathBuf = relative.as_path().components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        }).collect();
    if normalized.as_os_str().is_empty() {
        Ok(".".to_owned())
    } else {
        normalized.into_os_string().into_string()
            .map_err(|_| "Git package path is not UTF-8".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dependencies;

    const REV: &str = "0123456789abcdef0123456789abcdef01234567";
    const HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    fn declarations(
        dependencies: &str,
    ) -> Result<BTreeMap<String, DependencyDeclaration>, crate::DependencyResolutionError> {
        let document = toml_edit::ImDocument::parse(dependencies).unwrap();
        parse_dependencies(Path::new("."), "app", document.get("dependencies"))
    }

    #[test]
    fn parses_local_and_git_source_declarations_without_a_pin() {
        let graph = declarations(
            "[dependencies.local_lib]\npath = \"../local_lib\"\n\n\
             [dependencies.git_lib]\ngit = \"../upstream\"\npath = \"packages/git_lib\"\n",
        )
        .unwrap();
        assert_eq!(
            graph["local_lib"].source,
            DependencySource::Local {
                path: "../local_lib".into(),
            },
        );
        assert_eq!(
            graph["git_lib"].source,
            DependencySource::Git(GitSource {
                origin: "../upstream".to_owned(),
                package_path: "packages/git_lib".to_owned(),
            }),
        );

        let error = declarations(&format!(
            "[dependencies.git_lib]\ngit = \"../upstream\"\nrev = \"{REV}\"\n"
        ))
        .unwrap_err();
        assert!(error.diagnostics[0].message.contains("rev belongs in wosy.lock"));
    }

    #[test]
    fn rejects_unsafe_git_checkout_paths() {
        for (name, path) in [
            ("git_lib", "packages/../git_lib"),
            ("git_lib", "/packages/git_lib"),
            ("../git_lib", "."),
            ("app", "."),
        ] {
            let error = declarations(&format!(
                "[dependencies.\"{name}\"]\ngit = \"../upstream\"\npath = \"{path}\"\n"
            ))
            .unwrap_err();
            assert_eq!(error.diagnostics[0].code, "P0001", "{name}: {path}");
        }
    }

    #[test]
    fn lock_is_canonical_and_refuses_stale_declarations() {
        let record = |name| format!(
            "[packages.{name}]\ngit = \"../upstream\"\nrev = \"{REV}\"\npath = \".\"\nsource_hash = \"{HASH}\"\n"
        );
        let unsorted = format!("version = 1\n\n{}\n{}", record("zulu"), record("alpha"));
        let sorted = format!("version = 1\n\n{}\n{}", record("alpha"), record("zulu"));
        let lock = PackageLock::parse(unsorted).unwrap();
        assert_eq!(lock.render(), sorted);
        assert_eq!(PackageLock::parse(lock.render()).unwrap(), lock);
        let source = |origin: &str| GitSource { origin: origin.to_owned(), package_path: ".".to_owned() };
        assert!(lock.pin_for("alpha", &source("../upstream")).is_ok());
        assert_eq!(
            lock.pin_for("alpha", &source("../another")).unwrap_err(),
            "dependency alpha disagrees with wosy.lock"
        );
        assert_eq!(
            lock.check_reached(&BTreeSet::new()).unwrap_err(),
            "wosy.lock contains undeclared Git dependency alpha"
        );
        for bad_rev in ["main", "0123456"] {
            let error = PackageLock::parse(sorted.replace(REV, bad_rev)).unwrap_err();
            assert!(error.contains("invalid rev"));
        }
        let invalid = sorted.replace(HASH, "short");
        assert!(PackageLock::parse(invalid).unwrap_err().contains("invalid source_hash"));
    }
}
