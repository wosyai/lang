//! Shared source observation and namespace discovery for the ordinary CLI.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use tokio::time::Instant;
use wosy_compiler::ScalarSourceDeclarations;
use wosy_project::{
    load_reachable_source_graph, ContentRevision, GraphLoadError, NamespaceEdgeInput,
    NamespacePackages, PackageRelativePath, ProjectConfiguration, ProjectContext, SourceGraph,
    SourceIdentity, SourceNodeInput, SourceObservation, SourceSpan,
};
use wosy_protocol::Import;

/// Preserve the ordinary graph's first-discovery provenance ordering.
pub fn observations(graph: SourceGraph) -> Vec<SourceObservation> {
    graph.nodes.into_iter().map(|node| SourceObservation { source: node.source, content_revision: node.content_revision }).collect()
}

pub fn imports(declarations: &ScalarSourceDeclarations) -> Result<Vec<Import>, String> {
    declarations.namespaces().map(|ns| Import::from_tokens(
        &ns.package, &ns.path, &ns.binding, ns.span.start, ns.span.end,
    )).collect()
}

/// Each source is read and observed once. The observer supplies imports from
/// fresh CST declarations or a matching persisted observation.
pub enum Error<E = String> {
    Observe(E),
    Graph(GraphLoadError),
}

impl<E: From<String>> From<String> for Error<E> {
    fn from(message: String) -> Self {
        Self::Observe(message.into())
    }
}

pub fn load(
    root: &Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    observe: impl FnMut(&SourceIdentity, &ContentRevision, String) -> Result<Vec<Import>, String>,
) -> Result<SourceGraph, Error> {
    load_with_reader(root, project, configuration, read_source, observe)
}

pub(crate) fn read_source(
    path: &Path, opened: Option<fs::File>,
) -> Result<(ContentRevision, String), String> {
    let read_error = |error| format!("failed to read {}: {error}", path.display());
    let text = if let Some(mut file) = opened {
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(read_error)?;
        text
    } else {
        fs::read_to_string(path).map_err(read_error)?
    };
    Ok((revision(blake3::hash(text.as_bytes())), text))
}

/// Read one live regular source without admitting more than the preview's
/// source budget. Retain one overflow sentinel only to detect growth.
#[cfg(target_os = "linux")]
pub(crate) fn read_bounded_regular_source(
    path: &Path, opened: Option<fs::File>, max_bytes: usize,
) -> Result<(ContentRevision, String), String> {
    let read_error = |error| format!("failed to read {}: {error}", path.display());
    let file = match opened { Some(file) => file, None => wosy_project::open_live_source(path).map_err(read_error)? };
    let info = file.metadata().map_err(read_error)?;
    if !info.is_file() { return Err("preview source must be a regular file".into()); }
    if info.len() > max_bytes as u64 { return Err("preview source exceeds the authored byte budget".into()); }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1) as u64).read_to_end(&mut bytes).map_err(read_error)?;
    if bytes.len() > max_bytes { return Err("preview source exceeds the authored byte budget".into()); }
    let text = String::from_utf8(bytes).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok((revision(blake3::hash(text.as_bytes())), text))
}

fn revision(hash: blake3::Hash) -> ContentRevision {
    ContentRevision(hash.to_hex().to_string())
}

/// Used only to compare live bytes with a previously parsed authored root.
/// The aggregate cap is shared by the root and all reachable source files.
pub(crate) struct LiveSourceReader {
    remaining: usize,
    deadline: Instant,
}

impl LiveSourceReader {
    pub(crate) fn new(bytes: usize, deadline: Instant) -> Self { Self { remaining: bytes, deadline } }

    pub(crate) fn check_time(&self) -> io::Result<()> {
        require_live_time(self.deadline)
    }

    pub(crate) fn observe(&mut self, path: &Path, opened: Option<fs::File>) -> io::Result<ContentRevision> {
        self.check_time()?;
        let mut file = match opened { Some(file) => file, None => wosy_project::open_live_source(path)? };
        self.check_time()?;
        let info = file.metadata()?;
        self.check_time()?;
        if !info.is_file() { return Err(io::Error::other("live authored source must be a regular file")); }
        if info.len() > self.remaining as u64 { return Err(source_budget_exceeded()); }
        let deadline = self.deadline;
        read_live_revision(&mut file, &mut self.remaining, || require_live_time(deadline))
    }
}

fn require_live_time(deadline: Instant) -> io::Result<()> {
    if Instant::now() < deadline { Ok(()) }
    else { Err(io::Error::new(io::ErrorKind::TimedOut, "diagnostic session deadline elapsed")) }
}

fn source_budget_exceeded() -> io::Error {
    io::Error::other("live authored source exceeds the byte budget")
}

fn read_live_revision(
    file: &mut impl Read, remaining: &mut usize, mut check_time: impl FnMut() -> io::Result<()>,
) -> io::Result<ContentRevision> {
    let mut hash = blake3::Hasher::new();
    let mut scratch = [0u8; 8192];
    loop {
        check_time()?;
        // A single overflow sentinel detects growth after the metadata
        // observation. Its byte is never admitted or retained.
        let end = scratch.len().min(remaining.saturating_add(1));
        let read = match file.read(&mut scratch[..end]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        check_time()?;
        if read == 0 { return Ok(revision(hash.finalize())); }
        *remaining = remaining.checked_sub(read).ok_or_else(source_budget_exceeded)?;
        hash.update(&scratch[..read]);
    }
}

/// Source traversal and namespace resolution are shared by parsing and live
/// digest probes. The latter need not retain the file text.
pub(crate) fn load_with_reader<E: From<String>, S>(
    root: &Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    mut read: impl FnMut(&Path, Option<fs::File>) -> Result<(ContentRevision, S), E>,
    mut observe: impl FnMut(&SourceIdentity, &ContentRevision, S) -> Result<Vec<Import>, E>,
) -> Result<SourceGraph, Error<E>> {
    let root_path = PackageRelativePath::new(project.source_root.clone())?;
    let root_project = root.to_string_lossy().into_owned();
    let packages = NamespacePackages {
        root_name: &project.package,
        root_source: configuration.module_source_root.as_ref(),
        dependencies: &configuration.resolved_packages,
    };
    let mut discovered = HashMap::from([(
        project.package.as_str(),
        HashSet::from([root_path.clone()]),
    )]);
    let mut pending = VecDeque::from([(project.package.as_str(), root_path, None::<fs::File>)]);
    let mut nodes = Vec::new();

    while let Some((package, path, checked_git_source)) = pending.pop_front() {
        let (package_root, project_identity) = if package == project.package {
            (root, root_project.clone())
        } else {
            let resolved = configuration
                .resolved_packages
                .get(package)
                .ok_or_else(|| format!("dependency package {package} is missing"))?;
            (resolved.project_root.as_path(), resolved.identity.project.clone())
        };
        let source_path = package_root.join(path.as_path());
        let (revision, text) = read(&source_path, checked_git_source).map_err(Error::Observe)?;
        let source = SourceIdentity::new(project_identity, package.to_owned(), path);
        let imports = observe(&source, &revision, text).map_err(Error::Observe)?;
        let mut namespace_edges = Vec::with_capacity(imports.len());
        for Import { origin, path, binding, span } in imports {
            let module = packages
                .resolve(&origin, &path)
                .ok_or_else(|| format!("dependency package {origin} is missing"))?;
            let target = module.package;
            let already_discovered = discovered
                .get(target)
                .is_some_and(|paths| paths.contains(module.path.as_ref()));
            if !already_discovered {
                let available = if let Some((_, checkout)) = configuration.git_package(target)? {
                    checkout.source(module.path.as_path())?.map(Some)
                } else {
                    let dir = module.dependency.map_or(root, |p| p.project_root.as_path());
                    dir.join(module.path.as_path()).is_file().then_some(None)
                };
                if let Some(checked_git_source) = available {
                    let target_path = module.path.into_owned();
                    discovered.entry(target).or_default().insert(target_path.clone());
                    pending.push_back((target, target_path, checked_git_source));
                }
            }
            namespace_edges.push(NamespaceEdgeInput {
                origin,
                path,
                binding,
                span: SourceSpan { source: source.clone(), range: span },
            });
        }
        nodes.push(SourceNodeInput {
            source,
            content_revision: revision,
            namespace_edges,
        });
    }
    let mut nodes = nodes.into_iter();
    let entry = nodes.next().ok_or_else(|| "project root source is missing".to_owned())?;
    load_reachable_source_graph(&packages, entry, nodes.collect()).map_err(Error::Graph)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a disposable regular file larger than one read chunk"]
    fn live_source_deadline_stops_after_one_completed_file_read() {
        use std::io::Seek;
        use std::time::Duration;

        let path = std::path::PathBuf::from(std::env::var_os("WOSY_LIVE_DEADLINE_SOURCE").unwrap());
        let mut file = wosy_project::open_live_source(&path).unwrap();
        let metadata = file.metadata().unwrap();
        let mut remaining = metadata.len() as usize;
        assert!(metadata.is_file() && remaining > 8192);
        let mut polls = 0;
        let mut expiry = Instant::now() + Duration::from_secs(60);
        let error = read_live_revision(&mut file, &mut remaining, || {
            polls += 1;
            if polls == 2 { expiry = Instant::now(); }
            require_live_time(expiry)
        }).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(polls, 2);
        assert_eq!(file.stream_position().unwrap(), 8192);
        // The chunk returned at the expiry cut was never hashed or charged.
        assert_eq!(remaining as u64, file.metadata().unwrap().len());
    }
}
