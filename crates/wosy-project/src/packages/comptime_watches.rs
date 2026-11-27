use crate::{ContentRevision, ObservedClosedComptimeProfile, PackageRelativePath, ProjectConfiguration};
use std::path::Path;
use toml_edit::ImDocument;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WatchRoot {
    OwningPackage,
    ConsumingProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchedComptimeFile {
    pub root: WatchRoot,
    pub path: PackageRelativePath,
    pub revision: ContentRevision,
}

/// Counts shared across the two selected roots, including irrelevant entries
/// scanned by a wildcard. Limits do not bound the ordinary source parser.
#[derive(Clone, Copy)]
pub struct ComptimeWatchLimits {
    pub entries: usize,
    pub files: usize,
    pub content_bytes: u64,
}

impl Default for ComptimeWatchLimits {
    fn default() -> Self {
        Self { entries: 4096, files: 256, content_bytes: 16 << 20 }
    }
}

/// The local consumer selection for read-only input observations.
pub struct LocalComptimeInputScope<'a> {
    pub target: &'a str,
    pub profile: &'a str,
    pub site: &'a str,
    pub watch_limits: ComptimeWatchLimits,
}

#[cfg(target_os = "linux")]
pub use linux::{observe_comptime_watches, observe_local_comptime_inputs};

#[cfg(not(target_os = "linux"))]
pub fn observe_local_comptime_inputs(
    _: &Path, _: LocalComptimeInputScope<'_>, _: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(ObservedClosedComptimeProfile, Vec<WatchedComptimeFile>), String> {
    Err("local comptime input observation requires Linux anchored reads".into())
}

#[cfg(not(target_os = "linux"))]
pub fn observe_comptime_watches(
    _: &Path, _: &str, _: &ProjectConfiguration, _: &ImDocument<String>,
    _: &str, _: &str, _: ComptimeWatchLimits,
) -> Result<Vec<WatchedComptimeFile>, String> {
    Err("comptime watch observation requires Linux anchored reads".into())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::packages::GitDirectory;
    use std::{
        collections::BTreeSet,
        ffi::OsString,
        fs::{File, Metadata},
        io::Read,
        os::unix::fs::MetadataExt,
        path::PathBuf,
    };

    const MAX_DEPTH: usize = 32;
    const MAX_OBSERVED_METADATA_BYTES: u64 = 1 << 20;
    const MAX_MATCH_STEPS: usize = 4 << 20;

    /// Authenticate the exact raw manifest before ordinary configuration
    /// resolution or watches. Both fixed metadata files and all owning
    /// watch paths are opened through one anchored directory. This lane
    /// accepts only the dependency-free consumer; no launch authority.
    pub fn observe_local_comptime_inputs(
        root: &Path, scope: LocalComptimeInputScope<'_>, authorize_manifest: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<(ObservedClosedComptimeProfile, Vec<WatchedComptimeFile>), String> {
        use super::super::{snapshot::MANIFEST, ProjectSnapshot, LOCKFILE};
        let (mut snapshot, directory) = ProjectSnapshot::anchored_compiler_reader(root)?;
        let bytes = observed_metadata(&directory, MANIFEST)?.ok_or("watched-project manifest is missing")?;
        let text = String::from_utf8(bytes).map_err(|_| "watched-project manifest is not UTF-8")?;
        authorize_manifest(&text)?;
        let document = ImDocument::parse(text).map_err(|_| "invalid watched-project manifest")?;
        if document.get("dependencies").is_some_and(|item| item.as_table_like().is_none_or(|table| !table.is_empty())) {
            return Err("local comptime input observation requires a dependency-free consumer".into());
        }
        snapshot.set_observed_lock(observed_metadata(&directory, LOCKFILE)?);
        let (configuration, document) = ProjectConfiguration::from_snapshot_manifest(root, &snapshot, document)?;
        let selected = ObservedClosedComptimeProfile::observe_input_declarations(
            &configuration, &document, scope.target, scope.profile,
        )?;
        let globs = declarations(&document, scope.site, "watch")?;
        let mut result = Vec::new();
        if !globs.is_empty() {
            let mut budget = watch_budget(scope.watch_limits);
            let mut steps = MAX_MATCH_STEPS;
            resolve(&directory, WatchRoot::OwningPackage, &globs, &mut budget, &mut steps, &mut result)?;
            order_watches(&mut result);
        }
        Ok((selected, result))
    }

    fn observed_metadata(directory: &GitDirectory, name: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(file) = directory.source(Path::new(name))? else { return Ok(None); };
        let mut bytes = Vec::new();
        file.take(MAX_OBSERVED_METADATA_BYTES + 1).read_to_end(&mut bytes)
            .map_err(|_| format!("cannot read local comptime metadata {name}"))?;
        if bytes.len() as u64 > MAX_OBSERVED_METADATA_BYTES {
            return Err(format!("local comptime metadata {name} exceeds its byte limit"));
        }
        Ok(Some(bytes))
    }

    fn watch_budget(limits: ComptimeWatchLimits) -> ComptimeWatchLimits {
        let maximum = ComptimeWatchLimits::default();
        ComptimeWatchLimits {
            entries: limits.entries.min(maximum.entries),
            files: limits.files.min(maximum.files),
            content_bytes: limits.content_bytes.min(maximum.content_bytes),
        }
    }

    fn order_watches(result: &mut [WatchedComptimeFile]) {
        result.sort_unstable_by(|a, b| (a.root, &a.path).cmp(&(b.root, &b.path)));
    }

    /// Resolve read-only declarations for the caller's already-resolved site.
    /// Does not establish site reachability or grant the evaluator access.
    pub fn observe_comptime_watches(
        consumer_root: &Path, consumer_package: &str, configuration: &ProjectConfiguration,
        consumer_manifest: &ImDocument<String>, owner: &str, site: &str, limits: ComptimeWatchLimits,
    ) -> Result<Vec<WatchedComptimeFile>, String> {
        let mut budget = watch_budget(limits);
        let mut result = Vec::new();
        let mut match_steps = MAX_MATCH_STEPS;
        if owner == consumer_package {
            let globs = declarations(consumer_manifest, site, "watch")?;
            if !globs.is_empty() {
                let directory = open_directory(consumer_root)?;
                resolve(&directory, WatchRoot::OwningPackage, &globs, &mut budget, &mut match_steps, &mut result)?;
            }
        } else {
            let package = configuration.resolved_packages.get(owner)
                .ok_or("watch owner must be a resolved dependency")?;
            let local;
            let directory = if let Some((_, resolved)) = configuration.git_package(owner)? {
                resolved
            } else {
                let path = package.local_root_identity.as_ref()
                    .ok_or("watch owner has no resolved local identity")?;
                local = open_directory(path)?;
                &local
            };
            let document = owner_manifest(directory, owner)?;
            let own = declarations(&document, site, "watch")?;
            let add = declarations(consumer_manifest, &format!("{owner}:{site}"), "watch_add")?;
            if !own.is_empty() {
                resolve(directory, WatchRoot::OwningPackage, &own, &mut budget, &mut match_steps, &mut result)?;
            }
            if !add.is_empty() {
                let consumer = open_directory(consumer_root)?;
                resolve(&consumer, WatchRoot::ConsumingProject, &add, &mut budget, &mut match_steps, &mut result)?;
            }
        }
        order_watches(&mut result);
        Ok(result)
    }

    fn open_directory(path: &Path) -> Result<GitDirectory, String> {
        GitDirectory::new(GitDirectory::open_absolute(path).map_err(|error| error.to_string())?)
    }

    fn owner_manifest(directory: &GitDirectory, owner: &str) -> Result<ImDocument<String>, String> {
        let file = directory.source(Path::new("wosy.toml"))?.ok_or("watch owner manifest is missing")?;
        let mut text = String::new();
        file.take(MAX_OBSERVED_METADATA_BYTES + 1).read_to_string(&mut text)
            .map_err(|_| "watch owner manifest cannot be read as UTF-8")?;
        if text.len() as u64 > MAX_OBSERVED_METADATA_BYTES { return Err("watch owner manifest exceeds its byte limit".into()); }
        let document = ImDocument::parse(text).map_err(|_| "invalid watch owner manifest")?;
        if document.get("package").and_then(|package| package.get("name")).and_then(toml_edit::Item::as_str) != Some(owner) {
            return Err("watch owner manifest no longer names the resolved package".into());
        }
        Ok(document)
    }

    type Glob<'a> = Vec<&'a str>;
    type Cursor = (usize, usize);

    fn declarations<'a>(document: &'a ImDocument<String>, site: &str, field: &str) -> Result<Vec<Glob<'a>>, String> {
        let Some(table) = document.get("comptime") else { return Ok(Vec::new()) };
        let table = table.as_table_like().ok_or("comptime watches must be a table")?;
        let Some(site) = table.get(site) else { return Ok(Vec::new()) };
        let site = site.as_table_like().ok_or("selected comptime site must be a table")?;
        let Some(value) = site.get(field) else { return Ok(Vec::new()) };
        let globs = value.as_array().ok_or("selected comptime watch paths must be an array")?;
        let mut seen = BTreeSet::new();
        let mut parsed = Vec::new();
        for value in globs {
            let pattern = value.as_str().ok_or("a comptime watch glob must be a string")?;
            if !seen.insert(pattern) { continue; }
            let parts: Vec<_> = pattern.split('/').collect();
            if parts.len() > MAX_DEPTH || parts.iter().any(|part| {
                matches!(*part, "" | "." | "..")
                    || part.contains(['[', ']', '{', '}', '\\', '\0'])
                    || (*part != "**" && part.contains("**"))
            }) {
                return Err("unsupported comptime watch glob; use relative components, *, ? or whole-component **".into());
            }
            parsed.push(parts);
        }
        Ok(parsed)
    }

    fn resolve(
        directory: &GitDirectory, root: WatchRoot, patterns: &[Glob<'_>],
        budget: &mut ComptimeWatchLimits, match_steps: &mut usize, result: &mut Vec<WatchedComptimeFile>,
    ) -> Result<(), String> {
        let mut states = BTreeSet::new();
        for index in 0..patterns.len() { epsilon_closure(patterns, (index, 0), &mut states); }
        Walker { root, patterns, budget, match_steps, result }.directory(directory, &mut PathBuf::new(), 0, &states)
    }

    struct Walker<'a, 'p> {
        root: WatchRoot,
        patterns: &'a [Glob<'p>],
        budget: &'a mut ComptimeWatchLimits,
        match_steps: &'a mut usize,
        result: &'a mut Vec<WatchedComptimeFile>,
    }

    impl Walker<'_, '_> {
        fn directory(
            &mut self, dir: &GitDirectory, path: &mut PathBuf, depth: usize, states: &BTreeSet<Cursor>,
        ) -> Result<(), String> {
            if depth >= MAX_DEPTH { return Err("comptime watch exceeds its directory depth limit".into()); }
            let names: Option<BTreeSet<OsString>> = states.iter().filter_map(|(p, c)| {
                self.patterns[*p].get(*c)
            }).map(|segment| (!segment.contains(['*', '?'])).then(|| OsString::from(segment)))
                .collect();
            if let Some(names) = names {
                for name in names { self.entry(dir, path, depth, states, name, false)?; }
            } else {
                for name in dir.entry_names().map_err(|error| error.to_string())? {
                    self.entry(dir, path, depth, states, name.map_err(|error| error.to_string())?, true)?;
                }
            }
            Ok(())
        }

        fn entry(
            &mut self, dir: &GitDirectory, path: &mut PathBuf, depth: usize,
            states: &BTreeSet<Cursor>, name: OsString, listed: bool,
        ) -> Result<(), String> {
            charge(&mut self.budget.entries, "comptime watch exceeds its entry limit")?;
            let name = name.to_str().ok_or("comptime watch encountered a non-UTF-8 path")?;
            let mut next = BTreeSet::new();
            for &(p, c) in states {
                charge(self.match_steps, "comptime watch exceeds its pattern work limit")?;
                let Some(segment) = self.patterns[p].get(c) else { continue };
                if name.starts_with('.') && !segment.starts_with('.') { continue; }
                if *segment == "**" {
                    epsilon_closure(self.patterns, (p, c), &mut next);
                } else if match_component(segment, name, self.match_steps)? {
                    epsilon_closure(self.patterns, (p, c + 1), &mut next);
                }
            }
            if next.is_empty() { return Ok(()); }
            let matches = next.iter().any(|&(p, c)| c == self.patterns[p].len());
            let descend = next.iter().any(|&(p, c)| c < self.patterns[p].len());
            let Some(file) = dir.open(Path::new(name))? else {
                return if listed { Err("comptime watch entry changed during observation".into()) } else { Ok(()) };
            };
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            path.push(name);
            let result = if metadata.is_dir() && descend {
                self.directory(&GitDirectory::new(file)?, path, depth + 1, &next)
            } else if !metadata.is_dir() && matches {
                self.file(file, metadata, path)
            } else { Ok(()) };
            path.pop();
            result
        }

        fn file(&mut self, mut file: File, metadata: Metadata, path: &Path) -> Result<(), String> {
            if !metadata.is_file() { return Err("comptime watch matched a non-regular file".into()); }
            charge(&mut self.budget.files, "comptime watch exceeds its matched file limit")?;
            let bytes = metadata.len();
            self.budget.content_bytes = self.budget.content_bytes.checked_sub(bytes)
                .ok_or("comptime watch exceeds its file byte limit")?;
            let mut input = (&mut file).take(bytes + 1);
            let mut hash = blake3::Hasher::new();
            let mut actual = 0u64;
            let mut buffer = [0; 8192];
            loop {
                let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 { break; }
                hash.update(&buffer[..count]);
                actual += count as u64;
            }
            let end = file.metadata().map_err(|error| error.to_string())?;
            let version = |m: &Metadata| (m.len(), m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec());
            if actual != bytes || version(&metadata) != version(&end) {
                return Err("comptime watched file changed during observation".into());
            }
            self.result.push(WatchedComptimeFile {
                root: self.root, path: PackageRelativePath::new(path.to_path_buf())?,
                revision: ContentRevision(hash.finalize().to_hex().to_string()),
            });
            Ok(())
        }
    }

    fn charge(remaining: &mut usize, reason: &'static str) -> Result<(), String> {
        *remaining = remaining.checked_sub(1).ok_or(reason)?;
        Ok(())
    }

    fn epsilon_closure(patterns: &[Glob<'_>], (p, mut c): Cursor, set: &mut BTreeSet<Cursor>) {
        while set.insert((p, c)) && patterns[p].get(c) == Some(&"**") { c += 1; }
    }

    /// Match one UTF-8 component with only the last wildcard retry.
    fn match_component(mut pattern: &str, mut name: &str, steps: &mut usize) -> Result<bool, String> {
        if !pattern.contains(['*', '?']) { return Ok(pattern == name); }
        let mut retry: Option<(&str, &str)> = None;
        while let Some(value) = name.chars().next() {
            charge(steps, "comptime watch exceeds its pattern work limit")?;
            match pattern.chars().next() {
                Some('*') => {
                    pattern = &pattern[1..];
                    retry = Some((pattern, name));
                }
                Some(token) if token == '?' || token == value => {
                    pattern = &pattern[token.len_utf8()..];
                    name = &name[value.len_utf8()..];
                }
                _ => {
                    let Some((suffix, remainder)) = retry else { return Ok(false) };
                    let Some(previous) = remainder.chars().next() else { return Ok(false) };
                    name = &remainder[previous.len_utf8()..];
                    pattern = suffix;
                    retry = Some((suffix, name));
                }
            }
        }
        Ok(pattern.bytes().all(|byte| byte == b'*'))
    }
}
