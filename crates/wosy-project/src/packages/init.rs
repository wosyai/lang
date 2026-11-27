use std::path::Path;

#[cfg(target_os = "linux")]
use super::{
    git_sync::{Stage, Staging},
    valid_package_name, GitDirectory, ProjectSnapshot,
};
#[cfg(any(target_os = "linux", test))]
use super::{snapshot::MANIFEST, PackageLock, LOCKFILE};
#[cfg(target_os = "linux")]
use std::{
    ffi::OsString,
    fs,
    io::{self, Read},
    os::unix::fs::MetadataExt,
};

/// Write the supported base-language project scaffold into an existing
/// directory, with no implicit package resolution or tooling invocation.
pub fn init_project(root: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        init_linux(root)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        Err("wosy init currently requires Linux".to_owned())
    }
}

#[cfg(target_os = "linux")]
const ROOT_ENTRIES: &[&str] = &[
    "src",
    "scripts",
    ".gitignore",
    LOCKFILE,
    "wosy.meta.json",
    MANIFEST,
];

#[cfg(target_os = "linux")]
fn init_linux(root: &Path) -> Result<(), String> {
    let snapshot = ProjectSnapshot::writer(root)?;
    let name = snapshot
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| valid_package_name(name))
        .ok_or_else(|| "init needs a snake_case project directory name starting with a letter".to_owned())?;
    let project = snapshot.git_project()?;
    let physical = project
        .directory
        .descriptor_path()
        .map_err(|error| format!("cannot access project directory: {error}"))?;

    for entry in ROOT_ENTRIES {
        match fs::symlink_metadata(physical.join(entry)) {
            Ok(_) => return Err(format!("cannot initialize project: {entry} already exists")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot inspect {entry}: {error}")),
        }
    }

    // Share the anchored, private staging directory with package commands.
    // Git is not invoked. Publish the manifest last so readers cannot
    // discover a project before all of its inputs have been put in place.
    let mut staging = Staging::default();
    let stage = staging.workspace(&project)?;
    write_scaffold(stage, name)?;
    publish_scaffold(stage, &project.directory, |_| {})
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy)]
enum ScaffoldHost {
    Posix,
    #[cfg(test)]
    Windows,
}

#[cfg(any(target_os = "linux", test))]
fn scaffold_files(name: &str, host: ScaffoldHost) -> Vec<(&'static str, String)> {
    let (command, suffix) = match host {
        ScaffoldHost::Posix => (r#""sh""#, ""),
        #[cfg(test)]
        ScaffoldHost::Windows => (
            r#""powershell.exe", "-NoProfile", "-NonInteractive", "-File""#,
            ".ps1",
        ),
    };
    let manifest = include_str!("init_templates/project.toml")
        .replace("@package@", name)
        .replace("@script_command@", command)
        .replace("@script_suffix@", suffix);
    let mut files = vec![
        (MANIFEST, manifest),
        (LOCKFILE, PackageLock::default().render()),
        ("wosy.meta.json", "{}\n".into()),
        (".gitignore", "/deps/\n/.wosy/\n/references/wosy/\n".into()),
        ("src/main.w", include_str!("init_templates/main.w").into()),
        ("scripts/start.ll", include_str!("init_templates/start.ll").into()),
    ];
    let scripts: &[(&str, &str)] = match host {
        ScaffoldHost::Posix => &[
            ("scripts/build-native", include_str!("init_templates/build-native")),
            ("scripts/run-native", include_str!("init_templates/run-native")),
            ("scripts/build-wasm", include_str!("init_templates/build-wasm")),
            ("scripts/run-wasmtime", include_str!("init_templates/run-wasmtime")),
        ],
        #[cfg(test)]
        ScaffoldHost::Windows => &[
            ("scripts/build-native.ps1", include_str!("init_templates/build-native.ps1")),
            ("scripts/run-native.ps1", include_str!("init_templates/run-native.ps1")),
            ("scripts/build-wasm.ps1", include_str!("init_templates/build-wasm.ps1")),
            ("scripts/run-wasmtime.ps1", include_str!("init_templates/run-wasmtime.ps1")),
            ("scripts/run-program.ps1", include_str!("init_templates/run-program.ps1")),
        ],
    };
    files.extend(scripts.iter().copied().map(|(path, text)| (path, text.into())));
    files
}

#[cfg(target_os = "linux")]
fn write_scaffold(stage: &Stage, name: &str) -> Result<(), String> {
    for directory in ["src", "scripts"] {
        fs::create_dir(stage.path.join(directory))
            .map_err(|error| format!("cannot stage {directory}: {error}"))?;
    }
    for (path, text) in scaffold_files(name, ScaffoldHost::Posix) {
        fs::write(stage.path.join(path), text)
            .map_err(|error| format!("cannot stage {path}: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod scaffold_tests {
    use super::*;

    #[test]
    fn stock_scripts_match_their_host_manifest() {
        for (host, prefix, suffix) in [
            (ScaffoldHost::Posix, &["sh"][..], ""),
            (ScaffoldHost::Windows, &["powershell.exe", "-NoProfile", "-NonInteractive", "-File"][..], ".ps1"),
        ] {
            let files = scaffold_files("test_app", host);
            let content = |path: &str| {
                files.iter().find(|(name, _)| *name == path).unwrap().1.as_str()
            };
            let manifest = content(MANIFEST).parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(manifest["package"]["name"].as_str(), Some("test_app"));
            assert_eq!(content(LOCKFILE), PackageLock::default().render());
            assert_eq!(content("wosy.meta.json"), "{}\n");
            assert_eq!(manifest["targets"]["app"]["artifacts"]["main"]["profiles"]["dev"]["output"].as_str(), Some("native"));
            assert_eq!(manifest["targets"]["app"]["artifacts"]["main"]["profiles"]["wasm"]["output"].as_str(), Some("wasm-wasip1"));
            for (section, profile, name) in [
                ("builders", "native", "build-native"),
                ("builders", "llvm_wasm", "build-wasm"),
                ("runners", "native", "run-native"),
                ("runners", "wasmtime", "run-wasmtime"),
            ] {
                let path = format!("scripts/{name}{suffix}");
                let command = manifest[section][profile]["command"].as_array().unwrap();
                let actual = command.iter().map(|value| value.as_str().unwrap()).collect::<Vec<_>>();
                let expected = prefix.iter().copied().chain([path.as_str()]).collect::<Vec<_>>();
                assert_eq!(actual, expected);
                assert!(!content(&path).is_empty());
            }
        }
        for (path, text) in scaffold_files("test_app", ScaffoldHost::Windows) {
            if path.starts_with("scripts/build-") {
                assert!(text.contains("$Artifact.executable"));
            } else if path == "scripts/run-native.ps1" || path == "scripts/run-wasmtime.ps1" {
                assert!(text.contains("$env:WOSY_ARTIFACT_EXECUTABLE"));
            }
        }
    }
}

// A rename does not change file identity, mode or contents. Remember the
// exact staged entries so rollback can distinguish outside work from ours.
#[cfg(target_os = "linux")]
struct EntryReceipt {
    // Keep the original inode alive even if another writer removes its name.
    _original: fs::File,
    metadata: fs::Metadata,
    payload: PayloadReceipt,
}

#[cfg(target_os = "linux")]
enum PayloadReceipt {
    File(Vec<u8>),
    Directory(Vec<(OsString, EntryReceipt)>),
}

#[cfg(target_os = "linux")]
impl EntryReceipt {
    fn capture(parent: &GitDirectory, name: &Path, allow_directory: bool) -> Result<Self, String> {
        let mut file = parent
            .open(name)?
            .ok_or_else(|| format!("staged entry {} disappeared", name.display()))?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        let original = file.try_clone().map_err(|error| error.to_string())?;
        let payload = if metadata.is_dir() && allow_directory {
            let directory = GitDirectory::new(file)?;
            let mut names = directory.entries().map_err(|error| error.to_string())?;
            names.sort_unstable();
            let files = names
                .into_iter()
                .map(|name| {
                    let receipt = Self::capture(&directory, Path::new(&name), false)?;
                    Ok((name, receipt))
                })
                .collect::<Result<Vec<_>, String>>()?;
            PayloadReceipt::Directory(files)
        } else if metadata.is_file() {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            PayloadReceipt::File(bytes)
        } else {
            return Err(format!("unexpected staged entry {}", name.display()));
        };
        Ok(Self {
            _original: original,
            metadata,
            payload,
        })
    }

    fn matches(&self, parent: &GitDirectory, name: &Path) -> Result<bool, String> {
        let Some(file) = parent.open(name)? else {
            return Ok(false);
        };
        let now = file.metadata().map_err(|error| error.to_string())?;
        let identity = |m: &fs::Metadata| {
            (m.dev(), m.ino(), m.mode(), m.uid(), m.gid(), m.mtime(), m.mtime_nsec())
        };
        if identity(&now) != identity(&self.metadata) {
            return Ok(false);
        }
        match &self.payload {
            PayloadReceipt::File(contents) => {
                if now.len() != contents.len() as u64 {
                    return Ok(false);
                }
                let mut bytes = Vec::new();
                file.take(now.len() + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                Ok(bytes == *contents)
            }
            PayloadReceipt::Directory(receipts) => {
                let directory = GitDirectory::new(file)?;
                // The private receipt names are sorted. Walk live names
                // lazily; there is no need to list or sort a user directory.
                let mut seen = vec![false; receipts.len()];
                for name in directory.entry_names().map_err(|error| error.to_string())? {
                    let name = name.map_err(|error| error.to_string())?;
                    let Ok(index) = receipts.binary_search_by(|(expected, _)| expected.cmp(&name))
                    else {
                        return Ok(false);
                    };
                    if std::mem::replace(&mut seen[index], true) {
                        return Ok(false);
                    }
                }
                if seen.contains(&false) {
                    return Ok(false);
                }
                for (name, receipt) in receipts {
                    if !receipt.matches(&directory, Path::new(name))? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn publish_scaffold(
    stage: &Stage,
    project: &GitDirectory,
    before_publish: impl Fn(&str),
) -> Result<(), String> {
    let receipts = ROOT_ENTRIES
        .iter()
        .map(|entry| EntryReceipt::capture(&stage.handle, Path::new(entry), true))
        .collect::<Result<Vec<_>, _>>()?;
    for (index, entry) in ROOT_ENTRIES.iter().enumerate() {
        before_publish(entry);
        let name = Path::new(entry);
        if let Err(cause) = stage.handle.rename(name, project, name) {
            let mut notes = Vec::new();
            for (published, receipt) in ROOT_ENTRIES[..index].iter().zip(&receipts).rev() {
                let path = Path::new(published);
                if !receipt.matches(project, path).unwrap_or(false) {
                    notes.push(format!(
                        "{published} no longer matches scaffold; left in place"
                    ));
                    continue;
                }
                if let Err(error) = project.rename(path, &stage.handle, path) {
                    notes.push(format!("{published} was not moved: {error}"));
                    continue;
                }
                // A non-cooperating writer could swap or change the entry
                // between the comparison and rename. Restore a mismatch
                // without replacing anything newly present at the live path.
                // Never recursively delete a formerly live entry: an editor
                // may even have an open descriptor to it after both checks.
                stage.preserve.set(true);
                if !receipt.matches(&stage.handle, path).unwrap_or(false) {
                    match stage.handle.rename(path, project, path) {
                        Ok(()) => notes.push(format!("{published} changed; restored in place")),
                        Err(error) => notes.push(format!("{published} could not be restored: {error}")),
                    }
                }
            }
            if stage.preserve.get() {
                notes.push(format!("recovery under {}", stage.display.display()));
            }
            let detail = notes
                .into_iter()
                .map(|note| format!("; {note}"))
                .collect::<String>();
            return Err(format!("cannot publish {entry}: {cause}{detail}"));
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        env,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn with_scaffold(action: impl FnOnce(&Path, &Stage, &GitDirectory)) {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Temp(env::temp_dir().join(format!(
            "wosy-init-rollback-{}-{time}",
            std::process::id()
        )));
        fs::create_dir(&root.0).unwrap();
        let snapshot = ProjectSnapshot::writer(&root.0).unwrap();
        let project = snapshot.git_project().unwrap();
        let mut staging = Staging::default();
        let stage = staging.workspace(&project).unwrap();
        write_scaffold(stage, "test_app").unwrap();
        let display = stage.display.clone();
        action(&root.0, stage, &project.directory);
        drop(staging);
        assert!(display.is_dir(), "named recovery must outlive staging");
    }

    #[test]
    fn late_manifest_collision_recovers_every_unmodified_published_entry() {
        with_scaffold(|root, stage, project| {
            let error = publish_scaffold(stage, project, |entry| {
                if entry == MANIFEST {
                    fs::write(root.join(MANIFEST), b"existing project").unwrap();
                }
            })
            .unwrap_err();
            assert!(error.contains("cannot publish wosy.toml"), "{error}");
            assert!(error.contains(stage.display.to_string_lossy().as_ref()), "{error}");
            assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), b"existing project");
            assert!(ROOT_ENTRIES[..ROOT_ENTRIES.len() - 1]
                .iter()
                .all(|entry| !root.join(entry).exists()));
            assert!(ROOT_ENTRIES.iter().all(|entry| stage.path.join(entry).exists()));
            assert_eq!(fs::read_dir(root).unwrap().count(), 2);
        });
    }

    #[test]
    fn rollback_leaves_replaced_entry_and_added_directory_child_in_place() {
        with_scaffold(|root, stage, project| {
            let error = publish_scaffold(stage, project, |entry| {
                if entry == MANIFEST {
                    fs::rename(root.join("src"), root.join("saved_src")).unwrap();
                    fs::create_dir(root.join("src")).unwrap();
                    fs::write(root.join("src/user.w"), b"user replacement").unwrap();
                    fs::write(root.join("scripts/user.txt"), b"user addition").unwrap();
                    fs::write(root.join(MANIFEST), b"existing project").unwrap();
                }
            })
            .unwrap_err();
            assert!(error.contains("scripts no longer matches scaffold; left in place"), "{error}");
            assert!(error.contains("src no longer matches scaffold; left in place"), "{error}");
            assert_eq!(fs::read(root.join("src/user.w")).unwrap(), b"user replacement");
            assert_eq!(fs::read(root.join("scripts/user.txt")).unwrap(), b"user addition");
            assert_eq!(
                fs::read(root.join("saved_src/main.w")).unwrap(),
                include_bytes!("init_templates/main.w"),
            );
            assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), b"existing project");
            assert!(!root.join(LOCKFILE).exists());
            assert!(stage.path.join(LOCKFILE).is_file());
            assert!(!stage.path.join("src").exists());
            assert!(!stage.path.join("scripts").exists());
        });
    }
}
