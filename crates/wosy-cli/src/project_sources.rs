//! Shared source observation and namespace discovery for the ordinary CLI.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::Path;
use wosy_compiler::ScalarSourceDeclarations;
use wosy_project::{
    load_reachable_source_graph, ContentRevision, GraphLoadError, NamespaceEdgeInput,
    NamespacePackages, PackageRelativePath, ProjectConfiguration, ProjectContext, SourceGraph,
    SourceIdentity, SourceNodeInput, SourceSpan,
};
use wosy_protocol::Import;

pub fn imports(declarations: &ScalarSourceDeclarations) -> Result<Vec<Import>, String> {
    declarations.namespaces().map(|ns| Import::from_tokens(
        &ns.package, &ns.path, &ns.binding, ns.span.start, ns.span.end,
    )).collect()
}

/// Each source is read and observed once. The observer supplies imports from
/// fresh CST declarations or a matching persisted observation.
pub enum Error {
    Observe(String),
    Graph(GraphLoadError),
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Observe(message)
    }
}

pub fn load(
    root: &Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    include_git: bool,
    mut observe: impl FnMut(&SourceIdentity, &ContentRevision, String) -> Result<Vec<Import>, String>,
) -> Result<SourceGraph, Error> {
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
        let read_error = |error| format!("failed to read {}: {error}", source_path.display());
        let text = if let Some(mut file) = checked_git_source {
            let mut text = String::new();
            file.read_to_string(&mut text).map_err(read_error)?;
            text
        } else {
            fs::read_to_string(&source_path).map_err(read_error)?
        };
        let revision = ContentRevision(blake3::hash(text.as_bytes()).to_hex().to_string());
        let source = SourceIdentity::new(project_identity, package.to_owned(), path);
        let imports = observe(&source, &revision, text)?;
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
                if !include_git && module.dependency.is_some_and(|p| p.git_pin_identity.is_some()) {
                    return Err(Error::Observe("source preparation currently supports local dependencies only".to_owned()));
                }
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
