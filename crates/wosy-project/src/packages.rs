use super::{DependencyDeclaration, DependencySource, GitSource};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use toml_edit::{Item, Table};

const LOCKFILE: &str = "wosy.lock";

#[derive(Debug, Default, Eq, PartialEq)]
pub struct PackageLock {
    pub(super) packages: BTreeMap<String, LockedPackage>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct LockedPackage {
    pub(super) source: GitSource,
    pub(super) revision: String,
    pub(super) source_hash: String,
}

impl PackageLock {
    pub fn read(root: &Path) -> Result<Option<Self>, String> {
        let path = root.join(LOCKFILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(format!("{LOCKFILE} must be a regular file")),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot inspect {LOCKFILE}: {error}")),
        }
        let content = fs::read_to_string(path)
            .map_err(|error| format!("cannot read {LOCKFILE}: {error}"))?;
        Self::parse(content)
            .map(Some)
            .map_err(|error| format!("invalid {LOCKFILE}: {error}"))
    }

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
                        package_path: field("path", valid_package_path)?,
                    },
                    revision: field("rev", valid_git_revision)?,
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
            entry["rev"] = value(&pin.revision);
            entry["path"] = value(&pin.source.package_path);
            entry["source_hash"] = value(&pin.source_hash);
            packages.insert(name, Item::Table(entry));
        }
        if !self.packages.is_empty() {
            document["packages"] = Item::Table(packages);
        }
        document.to_string()
    }

    pub fn check_declarations(
        &self,
        dependencies: &BTreeMap<String, DependencyDeclaration>,
    ) -> Result<(), String> {
        for (name, declaration) in dependencies {
            let DependencySource::Git(source) = &declaration.source else {
                continue;
            };
            let pin = self
                .packages
                .get(name)
                .ok_or_else(|| format!("dependency {name} has no entry in {LOCKFILE}"))?;
            if pin.source != *source {
                return Err(format!("dependency {name} disagrees with {LOCKFILE}"));
            }
        }
        for name in self.packages.keys() {
            if !dependencies
                .get(name)
                .is_some_and(|declaration| matches!(declaration.source, DependencySource::Git(_)))
            {
                return Err(format!("{LOCKFILE} contains undeclared Git dependency {name}"));
            }
        }
        Ok(())
    }
}

pub(super) fn valid_origin(origin: &str) -> bool {
    !origin.is_empty() && origin == origin.trim() && !origin.chars().any(char::is_control)
}

pub(super) fn valid_package_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && valid_path_component(name)
}

pub(super) fn valid_git_revision(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && is_lower_hex(revision)
}

fn is_lower_hex(value: &str) -> bool {
    value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn valid_package_path(path: &str) -> bool {
    path == "." || path.split('/').all(valid_path_component)
}

fn valid_path_component(component: &str) -> bool {
    // Treat manifest paths the same on Unix and Windows. Git-internal names and
    // DOS device names must not become package checkout paths.
    let stem = component.split('.').next().unwrap_or_default().to_ascii_lowercase();
    let dos_device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (["com", "lpt"].iter().any(|prefix| stem.starts_with(prefix))
            && stem[3..].chars().count() == 1
            && stem[3..].chars().all(|c| matches!(c, '0'..='9' | '¹' | '²' | '³')));
    !component.is_empty()
        && !component.ends_with(['.', ' '])
        && !component.eq_ignore_ascii_case(".git")
        && !dos_device
        && !component
            .chars()
            .any(|c| c.is_control() || "\\:\"<>|?*".contains(c))
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
            ("git_lib", "packages/.Git/git_lib"),
            ("git_lib", "packages/CON/git_lib"),
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
        let graph = |origin| declarations(&format!(
            "[dependencies.zulu]\ngit = \"../upstream\"\n[dependencies.alpha]\ngit = \"{origin}\"\n[dependencies.local_lib]\npath = \"../local_lib\"\n"
        )).unwrap();
        assert!(lock.check_declarations(&graph("../upstream")).is_ok());
        assert_eq!(
            lock.check_declarations(&graph("../another")).unwrap_err(),
            "dependency alpha disagrees with wosy.lock"
        );
        let empty = declarations("").unwrap();
        assert_eq!(
            lock.check_declarations(&empty).unwrap_err(),
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
