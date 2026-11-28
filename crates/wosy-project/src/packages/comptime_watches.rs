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

/// Exact client-minted declared inputs for the dependency-free local
/// comptime corridor. Not a statement about undeclared tool inputs.
pub struct OwnedLocalComptimeInputs {
    project: std::path::PathBuf,
    site: String,
    configuration_identity: [u8; 32],
    declarations: ObservedClosedComptimeProfile,
    watches: Vec<WatchedComptimeFile>,
}

impl OwnedLocalComptimeInputs {
    pub fn project(&self) -> &Path { &self.project }
    pub fn site(&self) -> &str { &self.site }
    pub fn configuration_identity(&self) -> [u8; 32] { self.configuration_identity }
    pub fn declarations(&self) -> &ObservedClosedComptimeProfile { &self.declarations }
    pub fn watches(&self) -> &[WatchedComptimeFile] { &self.watches }
}

pub fn compiler_source_configuration_identity(
    manifest: &str, packages: &std::collections::BTreeMap<String, crate::ResolvedPackage>,
) -> Result<[u8; 32], String> {
    let bytes = serde_json::to_vec(&(manifest, packages)).map_err(|error| error.to_string())?;
    Ok(blake3::derive_key("wosy source configuration inputs v2", &bytes))
}

/// Uses the existing anchored path/watch observer; no caller-supplied
/// command, watch list or source-input digest is accepted.
pub fn observe_owned_local_comptime_inputs(
    root: &Path, scope: LocalComptimeInputScope<'_>, expected: [u8; 32],
) -> Result<OwnedLocalComptimeInputs, String> {
    let site = scope.site.to_owned();
    let (declarations, watches) = observe_local_comptime_inputs(root, scope, |manifest| {
        if compiler_source_configuration_identity(manifest, &std::collections::BTreeMap::new())? != expected {
            return Err("local comptime configuration changed after source staging".into());
        }
        Ok(())
    })?;
    Ok(OwnedLocalComptimeInputs {
        project: root.to_owned(), site, configuration_identity: expected, declarations, watches,
    })
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
        collections::{BTreeSet, HashMap},
        ffi::OsString,
        fs::{File, Metadata},
        io::Read,
        os::unix::fs::MetadataExt,
        path::PathBuf,
    };

    const MAX_OBSERVED_METADATA_BYTES: u64 = 1 << 20;

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
            let mut budget = scope.watch_limits;
            resolve(&directory, WatchRoot::OwningPackage, &globs, &mut budget, &mut result)?;
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

    fn order_watches(result: &mut [WatchedComptimeFile]) {
        result.sort_unstable_by(|a, b| (a.root, &a.path).cmp(&(b.root, &b.path)));
    }

    /// Resolve read-only declarations for the caller's already-resolved site.
    /// Does not establish site reachability or grant the evaluator access.
    pub fn observe_comptime_watches(
        consumer_root: &Path, consumer_package: &str, configuration: &ProjectConfiguration,
        consumer_manifest: &ImDocument<String>, owner: &str, site: &str, limits: ComptimeWatchLimits,
    ) -> Result<Vec<WatchedComptimeFile>, String> {
        let mut budget = limits;
        let mut result = Vec::new();
        if owner == consumer_package {
            let globs = declarations(consumer_manifest, site, "watch")?;
            if !globs.is_empty() {
                let directory = open_directory(consumer_root)?;
                resolve(&directory, WatchRoot::OwningPackage, &globs, &mut budget, &mut result)?;
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
                resolve(directory, WatchRoot::OwningPackage, &own, &mut budget, &mut result)?;
            }
            if !add.is_empty() {
                let consumer = open_directory(consumer_root)?;
                resolve(&consumer, WatchRoot::ConsumingProject, &add, &mut budget, &mut result)?;
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
    type Cursor = usize;
    const ACCEPT: Cursor = 0;

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
            if parts.iter().any(|part| {
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
        budget: &mut ComptimeWatchLimits, result: &mut Vec<WatchedComptimeFile>,
    ) -> Result<(), String> {
        let (patterns, states) = Patterns::compile(patterns);
        Walker { root, patterns, budget, result }.walk(directory.clone(), states)
    }

    struct Patterns<'p> {
        components: Vec<Component<'p>>,
        transitions: Vec<(usize, Cursor)>,
    }

    enum Component<'p> {
        Exact(&'p str),
        Recursive,
        Wild { hidden: bool, matcher: WildComponent },
    }

    impl<'p> Patterns<'p> {
        fn compile(globs: &[Glob<'p>]) -> (Self, BTreeSet<Cursor>) {
            let mut patterns = Self { components: Vec::new(), transitions: Vec::new() };
            let mut components = HashMap::new();
            let mut suffixes = HashMap::new();
            let mut start = BTreeSet::new();
            for glob in globs {
                let mut next = ACCEPT;
                for &part in glob.iter().rev() {
                    let component = *components.entry(part).or_insert_with(|| {
                        let id = patterns.components.len();
                        patterns.components.push(if part == "**" {
                            Component::Recursive
                        } else if part.contains(['*', '?']) {
                            Component::Wild { hidden: part.starts_with('.'), matcher: WildComponent::compile(part) }
                        } else {
                            Component::Exact(part)
                        });
                        id
                    });
                    next = *suffixes.entry((component, next)).or_insert_with(|| {
                        patterns.transitions.push((component, next));
                        patterns.transitions.len()
                    });
                }
                patterns.close(next, &mut start);
            }
            (patterns, start)
        }

        fn transition(&self, state: Cursor) -> Option<(usize, Cursor)> {
            state.checked_sub(1).map(|index| self.transitions[index])
        }

        fn close(&self, mut state: Cursor, result: &mut BTreeSet<Cursor>) {
            while result.insert(state) {
                let Some((component, next)) = self.transition(state) else { break; };
                if !matches!(self.components[component], Component::Recursive) { break; }
                state = next;
            }
        }

        fn after_name(&self, states: &BTreeSet<Cursor>, name: &str) -> BTreeSet<Cursor> {
            let hidden = name.starts_with('.');
            let mut evaluated = HashMap::new();
            let mut result = BTreeSet::new();
            for &state in states {
                let Some((component, next)) = self.transition(state) else { continue; };
                match &self.components[component] {
                    Component::Recursive if !hidden => self.close(state, &mut result),
                    Component::Exact(exact) if *exact == name => self.close(next, &mut result),
                    Component::Wild { hidden: selects_hidden, matcher } if !hidden || *selects_hidden => {
                        if *evaluated.entry(component).or_insert_with(|| matcher.matches(name)) {
                            self.close(next, &mut result);
                        }
                    }
                    _ => {}
                }
            }
            result
        }
    }

    struct Walker<'a, 'p> {
        root: WatchRoot,
        patterns: Patterns<'p>,
        budget: &'a mut ComptimeWatchLimits,
        result: &'a mut Vec<WatchedComptimeFile>,
    }

    struct DirectoryFrame {
        directory: GitDirectory,
        states: BTreeSet<Cursor>,
        names: Box<dyn Iterator<Item = std::io::Result<OsString>>>,
        listed: bool,
    }

    impl Walker<'_, '_> {
        fn frame(&self, directory: GitDirectory, states: BTreeSet<Cursor>) -> Result<DirectoryFrame, String> {
            let names: Option<BTreeSet<OsString>> = states.iter().filter_map(|&state| {
                self.patterns.transition(state)
            }).map(|(component, _)| match self.patterns.components[component] {
                Component::Exact(part) => Some(OsString::from(part)),
                _ => None,
            }).collect();
            let listed = names.is_none();
            let names: Box<dyn Iterator<Item = _>> = if let Some(names) = names {
                Box::new(names.into_iter().map(Ok))
            } else {
                Box::new(directory.entry_names().map_err(|error| error.to_string())?)
            };
            Ok(DirectoryFrame { directory, states, names, listed })
        }

        fn walk(&mut self, directory: GitDirectory, states: BTreeSet<Cursor>) -> Result<(), String> {
            // A frame owns its exact anchored directory and lazily scans its
            // names. Descend before consuming the next sibling, just as DFS.
            let mut stack = vec![self.frame(directory, states)?];
            let mut path = PathBuf::new();
            while let Some(frame) = stack.last_mut() {
                let Some(next) = frame.names.next() else {
                    stack.pop();
                    path.pop();
                    continue;
                };
                let name = next.map_err(|error| error.to_string())?;
                if let Some((directory, states)) = self.entry(&frame.directory, &mut path, &frame.states, name, frame.listed)? {
                    stack.push(self.frame(directory, states)?);
                }
            }
            Ok(())
        }

        fn entry(
            &mut self, dir: &GitDirectory, path: &mut PathBuf,
            states: &BTreeSet<Cursor>, name: OsString, listed: bool,
        ) -> Result<Option<(GitDirectory, BTreeSet<Cursor>)>, String> {
            charge(&mut self.budget.entries, "comptime watch exceeds its entry limit")?;
            let name = name.to_str().ok_or("comptime watch encountered a non-UTF-8 path")?;
            let next = self.patterns.after_name(states, name);
            if next.is_empty() { return Ok(None); }
            let matches = next.contains(&ACCEPT);
            let descend = next.len() > usize::from(matches);
            let Some(file) = dir.open(Path::new(name))? else {
                return if listed { Err("comptime watch entry changed during observation".into()) } else { Ok(None) };
            };
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            if metadata.is_dir() && descend {
                let directory = GitDirectory::new(file)?;
                path.push(name);
                return Ok(Some((directory, next)));
            }
            if !metadata.is_dir() && matches {
                path.push(name);
                self.file(file, metadata, path)?;
                path.pop();
            }
            Ok(None)
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

    /// Component syntax has no neighboring stars: the only whole-component
    /// double star belongs to the path automaton. Thus one epsilon shift
    /// closes all star states. Indices count Unicode scalars, not bytes.
    struct WildComponent {
        accept: usize,
        minimum_chars: usize,
        star: Vec<u64>,
        any: Vec<u64>,
        literal: HashMap<char, Vec<usize>>,
    }

    impl WildComponent {
        fn compile(pattern: &str) -> Self {
            let accept = pattern.chars().count();
            let words = accept / 64 + 1;
            let mut component = Self { accept, minimum_chars: accept,
                star: vec![0; words], any: vec![0; words], literal: HashMap::new() };
            for (position, token) in pattern.chars().enumerate() {
                match token {
                    '*' => { component.minimum_chars -= 1; set_bit(&mut component.star, position); }
                    '?' => set_bit(&mut component.any, position),
                    literal => component.literal.entry(literal).or_default().push(position),
                }
            }
            component
        }

        fn matches(&self, name: &str) -> bool {
            if name.chars().count() < self.minimum_chars { return false; }
            let mut active = vec![0u64; self.star.len()];
            active[0] = 1;
            self.close(&mut active);
            // Only observed name characters receive dense bit masks; the
            // declaration itself keeps sparse literal positions.
            let mut masks = HashMap::new();
            for ch in name.chars() {
                let mask = masks.entry(ch).or_insert_with(|| {
                    let mut mask = self.any.clone();
                    if let Some(positions) = self.literal.get(&ch) {
                        for &position in positions { set_bit(&mut mask, position); }
                    }
                    mask
                });
                let mut carry = 0;
                for ((word, &matches), &star) in active.iter_mut().zip(mask.iter()).zip(&self.star) {
                    let advance = *word & matches;
                    *word = (*word & star) | (advance << 1) | carry;
                    carry = advance >> 63;
                }
                if active.iter().all(|&word| word == 0) { return false; }
                self.close(&mut active);
            }
            active[self.accept / 64] & (1 << (self.accept % 64)) != 0
        }

        fn close(&self, active: &mut [u64]) {
            let mut carry = 0;
            for (word, &star) in active.iter_mut().zip(&self.star) {
                let epsilon = *word & star;
                *word |= (epsilon << 1) | carry;
                carry = epsilon >> 63;
            }
        }
    }

    fn set_bit(words: &mut [u64], position: usize) {
        words[position / 64] |= 1 << (position % 64);
    }
    #[cfg(test)]
    mod matcher_tests {
        use super::*;

        #[test]
        fn component_word_matcher_agrees_with_small_exhaustive_unicode_oracle() {
            fn oracle(tokens: &[char], name: &[char]) -> bool {
                match tokens {
                    [] => name.is_empty(),
                    ['*', rest @ ..] => (0..=name.len()).any(|start| oracle(rest, &name[start..])),
                    [head, rest @ ..] => name.split_first().is_some_and(|(first, tail)| {
                        (*head == '?' || head == first) && oracle(rest, tail)
                    }),
                }
            }
            fn strings(alphabet: &[char], maximum: usize) -> Vec<String> {
                let mut strings = vec![String::new()];
                for _ in 0..maximum {
                    let next: Vec<_> = strings.iter().flat_map(|prefix| {
                        alphabet.iter().map(move |token| format!("{prefix}{token}"))
                    }).collect();
                    strings.extend(next);
                    strings.sort_unstable();
                    strings.dedup();
                }
                strings
            }
            let names = strings(&['a', 'b', 'é'], 3);
            for pattern in strings(&['a', 'b', 'é', '*', '?'], 4) {
                if pattern.contains("**") { continue; }
                let tokens: Vec<_> = pattern.chars().collect();
                let matcher = WildComponent::compile(&pattern);
                for name in &names {
                    assert_eq!(matcher.matches(name), oracle(&tokens, &name.chars().collect::<Vec<_>>()),
                        "pattern {pattern:?}, name {name:?}");
                }
            }
            for prefix in [62, 63, 64, 127] {
                let pattern = format!("{}*?é", "a".repeat(prefix));
                let positive = format!("{}xyé", "a".repeat(prefix));
                let negative = format!("{}xyb", "a".repeat(prefix));
                let matcher = WildComponent::compile(&pattern);
                assert!(matcher.matches(&positive));
                assert!(!matcher.matches(&negative));
            }
        }

        #[test]
        fn path_automaton_interns_suffixes_and_preserves_recursive_hidden_semantics() {
            let (patterns, start) = Patterns::compile(&[
                vec!["left", "**", ".té?t"],
                vec!["right", "**", ".té?t"],
                vec!["right", "**", "file*"],
            ]);
            assert_eq!(patterns.transitions.len(), 7);
            let left = patterns.after_name(&start, "left");
            let right = patterns.after_name(&start, "right");
            assert!(left.is_subset(&right));
            let child = patterns.after_name(&left, "child");
            assert_eq!(child, left);
            assert!(patterns.after_name(&left, ".hidden_dir").is_empty());
            assert_eq!(patterns.after_name(&left, ".têst"), BTreeSet::new());
            assert_eq!(patterns.after_name(&left, ".tést"), BTreeSet::from([ACCEPT]));
        }
    }

}
