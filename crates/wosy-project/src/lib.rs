use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

pub mod packages;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub project: String,
    pub package: String,
    pub path: PackageRelativePath,
}

impl SourceIdentity {
    pub fn new(project: String, package: String, path: PackageRelativePath) -> Self {
        Self {
            project,
            package,
            path,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ContentRevision(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceObservation {
    pub source: SourceIdentity,
    pub content_revision: ContentRevision,
}

pub fn source_identity(observations: &[SourceObservation]) -> String {
    let mut identity = String::new();
    for (index, observation) in observations.iter().enumerate() {
        if index != 0 {
            identity.push(',');
        }
        write!(
            &mut identity,
            "{}:{}:{}",
            observation.source.package,
            observation.source.path.as_path().display(),
            observation.content_revision.0
        )
        .expect("formatting source identity into a String");
    }
    identity
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PackageRelativePath(PathBuf);

impl PackageRelativePath {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "source path {} must be relative to the package root",
                path.display()
            ));
        }
        if path.as_os_str().is_empty() {
            return Err("source path must not be empty".to_owned());
        }
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ByteSpan {
    pub start: u32,
    pub end: u32,
}

impl ByteSpan {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub source: SourceIdentity,
    pub range: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyDeclaration {
    pub name: String,
    pub source: DependencySource,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DependencySource {
    Local { path: PathBuf },
    Git(GitSource),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitSource {
    pub origin: String,
    pub package_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPackage {
    pub name: String,
    pub project_root: PathBuf,
    pub source_root: PathBuf,
    pub identity: SourceIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyDiagnostic {
    pub code: String,
    pub message: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyResolutionError {
    pub diagnostics: Vec<DependencyDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NamespaceEdgeInput {
    pub origin: String,
    pub path: PackageRelativePath,
    pub binding: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNodeInput {
    pub source: SourceIdentity,
    pub content_revision: ContentRevision,
    pub namespace_edges: Vec<NamespaceEdgeInput>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceNodeId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DfsState {
    Unvisited,
    Visiting,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NamespaceEdge {
    pub origin: String,
    pub path: PackageRelativePath,
    pub binding: String,
    pub span: SourceSpan,
    pub target: SourceNodeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReachableSourceNode {
    pub id: SourceNodeId,
    pub source: SourceIdentity,
    pub content_revision: ContentRevision,
    pub namespace_edges: Vec<NamespaceEdge>,
    pub dfs_state: DfsState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceGraph {
    pub nodes: Vec<ReachableSourceNode>,
    pub initialization_order: Vec<SourceNodeId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissingModuleDiagnostic {
    pub code: String,
    pub requested_package: String,
    pub requested_path: PackageRelativePath,
    pub edge: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CycleDiagnostic {
    pub code: String,
    pub edge: SourceSpan,
    pub cycle: Vec<SourceSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GraphDiagnostic {
    MissingModule(MissingModuleDiagnostic),
    Cycle(CycleDiagnostic),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphLoadError {
    pub diagnostics: Vec<GraphDiagnostic>,
}

impl GraphLoadError {
    fn missing(edge: &NamespaceEdgeInput, requested_path: PackageRelativePath) -> Self {
        Self {
            diagnostics: vec![GraphDiagnostic::MissingModule(MissingModuleDiagnostic {
                code: "M0001".to_owned(),
                requested_package: edge.origin.clone(),
                requested_path,
                edge: edge.span.clone(),
            })],
        }
    }

    fn cycle(edge: &NamespaceEdgeInput, cycle: Vec<SourceSpan>) -> Self {
        Self {
            diagnostics: vec![GraphDiagnostic::Cycle(CycleDiagnostic {
                code: "M0003".to_owned(),
                edge: edge.span.clone(),
                cycle,
            })],
        }
    }
}

pub fn load_reachable_source_graph(
    package: &str,
    root: SourceNodeInput,
    available: Vec<SourceNodeInput>,
    resolved_packages: &BTreeMap<String, ResolvedPackage>,
) -> Result<SourceGraph, GraphLoadError> {
    let root_key = source_key(&root.source);
    let root_span = SourceSpan {
        source: root.source.clone(),
        range: ByteSpan::new(0, 0),
    };
    let mut sources = BTreeMap::new();
    sources.insert(root_key.clone(), root);
    for source in available {
        sources.insert(source_key(&source.source), source);
    }

    let mut graph = SourceGraph {
        nodes: Vec::new(),
        initialization_order: Vec::new(),
    };
    let mut node_by_path = HashMap::new();
    let mut stack = Vec::new();
    visit_source(
        package,
        &root_key,
        &sources,
        resolved_packages,
        &mut graph,
        &mut node_by_path,
        &mut stack,
        &root_span,
        &root_key.2,
    )?;
    Ok(graph)
}

fn visit_source(
    package: &str,
    key: &(String, String, PackageRelativePath),
    sources: &BTreeMap<(String, String, PackageRelativePath), SourceNodeInput>,
    resolved_packages: &BTreeMap<String, ResolvedPackage>,
    graph: &mut SourceGraph,
    node_by_path: &mut HashMap<(String, String, PackageRelativePath), SourceNodeId>,
    stack: &mut Vec<SourceNodeId>,
    incoming: &SourceSpan,
    requested_path: &PackageRelativePath,
) -> Result<SourceNodeId, GraphLoadError> {
    if let Some(id) = node_by_path.get(key) {
        if graph.nodes[id.0].dfs_state == DfsState::Visiting {
            let cycle = stack
                .iter()
                .map(|item| SourceSpan {
                    source: graph.nodes[item.0].source.clone(),
                    range: ByteSpan::new(0, 0),
                })
                .chain(std::iter::once(incoming.clone()))
                .collect();
            return Err(GraphLoadError::cycle(
                &NamespaceEdgeInput {
                    origin: incoming.source.package.clone(),
                    path: key.2.clone(),
                    binding: String::new(),
                    span: incoming.clone(),
                },
                cycle,
            ));
        }
        return Ok(*id);
    }

    let source = sources.get(key).ok_or_else(|| {
        GraphLoadError::missing(
            &NamespaceEdgeInput {
                origin: key.1.clone(),
                path: key.2.clone(),
                binding: String::new(),
                span: incoming.clone(),
            },
            requested_path.clone(),
        )
    })?;
    let id = SourceNodeId(graph.nodes.len());
    node_by_path.insert(key.clone(), id);
    graph.nodes.push(ReachableSourceNode {
        id,
        source: source.source.clone(),
        content_revision: source.content_revision.clone(),
        namespace_edges: Vec::new(),
        dfs_state: DfsState::Visiting,
    });
    graph.initialization_order.push(id);
    stack.push(id);

    for edge in &source.namespace_edges {
        let (target_project, target_path) = if edge.origin == package {
            (source.source.project.as_str(), edge.path.clone())
        } else {
            let resolved = resolved_packages
                .get(&edge.origin)
                .ok_or_else(|| GraphLoadError::missing(edge, edge.path.clone()))?;
            let target_path = PackageRelativePath::new(
                resolved.identity.path.as_path().join(edge.path.as_path()),
            )
            .map_err(|_| GraphLoadError::missing(edge, edge.path.clone()))?;
            (resolved.identity.project.as_str(), target_path)
        };
        let target_key = source_key_parts(target_project, &edge.origin, target_path);
        let target = visit_source(
            package,
            &target_key,
            sources,
            resolved_packages,
            graph,
            node_by_path,
            stack,
            &edge.span,
            &edge.path,
        )?;
        graph.nodes[id.0].namespace_edges.push(NamespaceEdge {
            origin: edge.origin.clone(),
            path: target_key.2,
            binding: edge.binding.clone(),
            span: edge.span.clone(),
            target,
        });
    }
    stack.pop();
    graph.nodes[id.0].dfs_state = DfsState::Complete;
    Ok(id)
}

fn source_key(source: &SourceIdentity) -> (String, String, PackageRelativePath) {
    source_key_parts(&source.project, &source.package, source.path.clone())
}

fn source_key_parts(
    project: &str,
    package: &str,
    path: PackageRelativePath,
) -> (String, String, PackageRelativePath) {
    (project.to_owned(), package.to_owned(), path)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub package: String,
    pub source_root: PathBuf,
    pub comment_categories: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuilderConfig {
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub toolchain_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub command: Vec<String>,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactProfile {
    pub backend: String,
    pub output: String,
    pub builder: String,
    pub run_runner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exports: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetLayout {
    pub pointer_size: u64,
    pub pointer_alignment: u64,
}

impl ArtifactProfile {
    pub fn target_layout(&self) -> Result<TargetLayout, String> {
        match (self.backend.as_str(), self.output.as_str()) {
            ("llvm", "wasm-wasip1" | "wasm-wasip2" | "wasm-web") => Ok(TargetLayout {
                pointer_size: 4,
                pointer_alignment: 4,
            }),
            _ if self.exports.is_some() => {
                Err("root exports require a selected LLVM Wasm artifact profile".to_owned())
            }
            ("llvm", "native") => Ok(TargetLayout {
                pointer_size: 8,
                pointer_alignment: 8,
            }),
            (backend, output) => Err(format!(
                "unsupported artifact profile backend {backend} with output {output}"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactConfig {
    pub artifact: String,
    pub profiles: std::collections::BTreeMap<String, ArtifactProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetProfile {
    pub main_artifact: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetConfig {
    pub root: PathBuf,
    pub artifacts: std::collections::BTreeMap<String, ArtifactConfig>,
    pub profiles: std::collections::BTreeMap<String, TargetProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfiguration {
    pub default_target: String,
    pub builders: std::collections::BTreeMap<String, BuilderConfig>,
    pub runners: std::collections::BTreeMap<String, RunnerConfig>,
    pub targets: std::collections::BTreeMap<String, TargetConfig>,
    pub dependencies: BTreeMap<String, DependencyDeclaration>,
    pub resolved_packages: BTreeMap<String, ResolvedPackage>,
}

impl ProjectConfiguration {
    pub fn load(root: &Path) -> Result<Self, String> {
        Self::load_with_manifest(root).map(|(configuration, _)| configuration)
    }

    pub fn load_with_manifest(root: &Path) -> Result<(Self, toml_edit::ImDocument<String>), String> {
        let path = root.join("wosy.toml");
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let document =
            toml_edit::ImDocument::parse(content).map_err(|error| error.to_string())?;
        let package = document
            .get("package")
            .and_then(Item::as_table)
            .ok_or_else(|| "package is missing".to_owned())?;
        let package_name = scalar_string(package, "name")?;
        let dependencies = project_dependencies(root, &package_name, &document)?;
        let resolved_packages = resolve_local_packages(root, &dependencies)
            .map_err(|error| error.diagnostics[0].message.clone())?;
        let default_target = scalar_string(package, "default_target")?;
        let builders = named_commands(document.get("builders"), "builder")?;
        let runners = named_runners(document.get("runners"))?;
        let targets_table = document
            .get("targets")
            .and_then(Item::as_table)
            .ok_or_else(|| "targets is missing".to_owned())?;
        let mut targets = std::collections::BTreeMap::new();
        for (name, item) in targets_table {
            let table = item
                .as_table()
                .ok_or_else(|| format!("target {name} is not a table"))?;
            let root = table
                .get("root")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| format!("target {name} has no root"))?;
            let artifacts = parse_artifacts(table.get("artifacts"))?;
            let profiles = parse_target_profiles(table.get("profiles"))?;
            targets.insert(
                name.to_owned(),
                TargetConfig {
                    root: PathBuf::from(root),
                    artifacts,
                    profiles,
                },
            );
        }
        Ok((
            Self {
                default_target,
                builders,
                runners,
                targets,
                dependencies,
                resolved_packages,
            },
            document,
        ))
    }

    pub fn resolve_target(&self, target: Option<&str>) -> Result<String, String> {
        let target = target.unwrap_or(self.default_target.as_str());
        self.target_config(target)?;
        Ok(target.to_owned())
    }

    fn target_config(&self, target: &str) -> Result<&TargetConfig, String> {
        self.targets
            .get(target)
            .ok_or_else(|| format!("target {target} is missing from wosy.toml"))
    }

    pub fn artifact_profile(
        &self,
        target: &str,
        profile: &str,
    ) -> Result<
        (
            &TargetConfig,
            &ArtifactConfig,
            &ArtifactProfile,
            &TargetProfile,
        ),
        String,
    > {
        let target_config = self.target_config(target)?;
        let target_profile = target_config
            .profiles
            .get(profile)
            .ok_or_else(|| format!("target {target} profile {profile} is missing"))?;
        let artifact = target_config
            .artifacts
            .get(&target_profile.main_artifact)
            .ok_or_else(|| format!("main artifact {} is missing", target_profile.main_artifact))?;
        let artifact_profile = artifact
            .profiles
            .get(profile)
            .ok_or_else(|| format!("artifact profile {profile} is missing"))?;
        Ok((target_config, artifact, artifact_profile, target_profile))
    }
}

fn project_dependencies(
    root: &Path,
    package: &str,
    document: &toml_edit::ImDocument<String>,
) -> Result<BTreeMap<String, DependencyDeclaration>, String> {
    parse_dependencies(root, package, document.get("dependencies")).map_err(|error| {
        let diagnostic = &error.diagnostics[0];
        format!(
            "{} at {}..{}",
            diagnostic.message, diagnostic.span.range.start, diagnostic.span.range.end
        )
    })
}

pub fn parse_dependencies(
    root: &Path,
    package: &str,
    item: Option<&Item>,
) -> Result<BTreeMap<String, DependencyDeclaration>, DependencyResolutionError> {
    let Some(item) = item else {
        return Ok(BTreeMap::new());
    };
    let error = |message, span: Option<std::ops::Range<usize>>| DependencyResolutionError {
        diagnostics: vec![dependency_diagnostic(
            root,
            package,
            "P0001",
            message,
            span.unwrap_or(0..0),
        )],
    };
    let table = item
        .as_table()
        .ok_or_else(|| error("dependencies must be a table".to_owned(), item.span()))?;
    let mut dependencies = BTreeMap::new();
    for (name, item) in table {
        let invalid = |message| error(message, item.span());
        let dependency = item
            .as_table()
            .ok_or_else(|| invalid(format!("dependency {name} must be a table")))?;
        let source = if let Some(git) = dependency.get("git") {
            if name == package {
                return Err(invalid(format!(
                    "dependency {name} has the project package name"
                )));
            }
            if dependency.get("rev").is_some() {
                return Err(invalid(format!(
                    "dependency {name} rev belongs in wosy.lock, not wosy.toml"
                )));
            }
            if dependency
                .iter()
                .any(|(key, _)| !matches!(key, "git" | "path"))
            {
                return Err(invalid(format!(
                    "dependency {name} contains an unsupported field"
                )));
            }
            let origin = git
                .as_str()
                .filter(|origin| packages::valid_origin(origin))
                .ok_or_else(|| invalid(format!("dependency {name} git must be a non-empty origin")))?;
            if !packages::valid_package_name(name) {
                return Err(invalid(format!(
                    "Git dependency {name} must use a snake_case package name"
                )));
            }
            let package_path = match dependency.get("path") {
                None => ".",
                Some(path) => path
                    .as_str()
                    .filter(|path| packages::valid_package_path(path))
                    .ok_or_else(|| invalid(format!("dependency {name} path must be a repository-relative package directory")))?,
            };
            DependencySource::Git(GitSource {
                origin: origin.to_owned(),
                package_path: package_path.to_owned(),
            })
        } else {
            if dependency.iter().any(|(key, _)| key != "path") {
                return Err(invalid(format!(
                    "dependency {name} contains an unsupported field"
                )));
            }
            let path = dependency
                .get("path")
                .ok_or_else(|| invalid(format!("dependency {name} must declare path")))?
                .as_str()
                .ok_or_else(|| invalid(format!("dependency {name} path must be a string")))?;
            if path.is_empty() {
                return Err(invalid(format!("dependency {name} path must be non-empty")));
            }
            DependencySource::Local { path: path.into() }
        };
        dependencies.insert(
            name.to_owned(),
            DependencyDeclaration {
                name: name.to_owned(),
                source,
                span: dependency_diagnostic(
                    root,
                    package,
                    "P0001",
                    String::new(),
                    item.span().unwrap_or(0..0),
                )
                .span,
            },
        );
    }
    Ok(dependencies)
}

fn resolve_package(
    package_root: PathBuf,
    declaration: &DependencyDeclaration,
) -> Result<ResolvedPackage, DependencyResolutionError> {
    let name = &declaration.name;
    let error = |message| DependencyResolutionError {
        diagnostics: vec![DependencyDiagnostic {
            code: "P0002".to_owned(),
            message,
            span: declaration.span.clone(),
        }],
    };
    let manifest_path = package_root.join("wosy.toml");
    let content = fs::read_to_string(&manifest_path)
        .map_err(|_| error(format!("dependency {name} package manifest is missing")))?;
    let document = content
        .parse::<toml_edit::ImDocument<String>>()
        .map_err(|_| error(format!("dependency {name} package manifest is invalid")))?;
    let package_table = document
        .get("package")
        .and_then(Item::as_table)
        .ok_or_else(|| error(format!("dependency {name} package manifest has no package table")))?;
    let resolved_name = scalar_string(package_table, "name")
        .map_err(|_| error(format!("dependency {name} package manifest has no package name")))?;
    if resolved_name != *name {
        return Err(error(format!(
            "dependency {name} package name is {resolved_name}"
        )));
    }
    let source_root = package_table
        .get("source_root")
        .and_then(Item::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| error(format!("dependency {name} package manifest has no source root")))?;
    let source_path = PackageRelativePath::new(source_root.clone()).map_err(|_| {
        error(format!("dependency {name} source root must be package-relative"))
    })?;
    if !package_root.join(&source_root).is_dir() {
        return Err(error(format!("dependency {name} source root is missing")));
    }
    Ok(ResolvedPackage {
        name: resolved_name.clone(),
        source_root: package_root.join(&source_root),
        identity: SourceIdentity::new(
            package_root.to_string_lossy().into_owned(),
            resolved_name,
            source_path,
        ),
        project_root: package_root,
    })
}

fn resolve_local_packages(
    root: &Path,
    dependencies: &BTreeMap<String, DependencyDeclaration>,
) -> Result<BTreeMap<String, ResolvedPackage>, DependencyResolutionError> {
    dependencies
        .iter()
        .map(|(name, declaration)| {
            let DependencySource::Local { path } = &declaration.source else {
                return Err(DependencyResolutionError {
                    diagnostics: vec![DependencyDiagnostic {
                        code: "P0002".to_owned(),
                        message: format!("Git dependency {name} cannot yet be materialized"),
                        span: declaration.span.clone(),
                    }],
                });
            };
            resolve_package(root.join(path), declaration).map(|resolved| (name.clone(), resolved))
        })
        .collect()
}

fn dependency_diagnostic(
    root: &Path,
    package: &str,
    code: &str,
    message: impl Into<String>,
    span: std::ops::Range<usize>,
) -> DependencyDiagnostic {
    DependencyDiagnostic {
        code: code.to_owned(),
        message: message.into(),
        span: SourceSpan {
            source: SourceIdentity::new(
                root.to_string_lossy().into_owned(),
                package.to_owned(),
                PackageRelativePath::new(PathBuf::from("wosy.toml")).expect("manifest path"),
            ),
            range: ByteSpan::new(span.start as u32, span.end as u32),
        },
    }
}

fn named_commands(
    item: Option<&Item>,
    kind: &str,
) -> Result<std::collections::BTreeMap<String, BuilderConfig>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| format!("{kind}s are missing"))?;
    let mut commands = std::collections::BTreeMap::new();
    for (name, item) in table {
        let table = item
            .as_table()
            .ok_or_else(|| format!("{kind} {name} is not a table"))?;
        commands.insert(
            name.to_owned(),
            BuilderConfig {
                command: string_array(table, "command")?,
                args: string_array_optional(table, "args")?,
                toolchain_identity: scalar_string(table, "toolchain_identity")?,
            },
        );
    }
    Ok(commands)
}

fn named_runners(
    item: Option<&Item>,
) -> Result<std::collections::BTreeMap<String, RunnerConfig>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| "runners are missing".to_owned())?;
    let mut commands = std::collections::BTreeMap::new();
    for (name, item) in table {
        let table = item
            .as_table()
            .ok_or_else(|| format!("runner {name} is not a table"))?;
        commands.insert(
            name.to_owned(),
            RunnerConfig {
                command: string_array(table, "command")?,
                args: string_array_optional(table, "args")?,
            },
        );
    }
    Ok(commands)
}

fn string_array(table: &toml_edit::Table, key: &str) -> Result<Vec<String>, String> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
        .ok_or_else(|| format!("{key} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect()
}

fn string_array_optional(table: &toml_edit::Table, key: &str) -> Result<Vec<String>, String> {
    table
        .get(key)
        .map_or(Ok(Vec::new()), |_item| string_array(table, key))
}

fn parse_artifacts(
    item: Option<&Item>,
) -> Result<std::collections::BTreeMap<String, ArtifactConfig>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| "artifacts are missing".to_owned())?;
    let mut artifacts = std::collections::BTreeMap::new();
    for (name, item) in table {
        let table = item
            .as_table()
            .ok_or_else(|| format!("artifact {name} is not a table"))?;
        let artifact = table
            .get("artifact")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("artifact {name} has no identity"))?;
        let profiles_table = table
            .get("profiles")
            .and_then(Item::as_table)
            .ok_or_else(|| format!("artifact {name} has no profiles"))?;
        let mut profiles = std::collections::BTreeMap::new();
        for (profile_name, profile_item) in profiles_table {
            let profile = profile_item
                .as_table()
                .ok_or_else(|| format!("artifact profile {profile_name} is not a table"))?;
            profiles.insert(
                profile_name.to_owned(),
                ArtifactProfile {
                    backend: scalar_string(profile, "backend")?,
                    output: scalar_string(profile, "output")?,
                    builder: scalar_string(profile, "builder")?,
                    run_runner: scalar_string(profile, "run_runner")?,
                    exports: profile.get("exports").map(parse_root_exports).transpose()?,
                },
            );
        }
        artifacts.insert(
            name.to_owned(),
            ArtifactConfig {
                artifact: artifact.to_owned(),
                profiles,
            },
        );
    }
    Ok(artifacts)
}

fn parse_root_exports(item: &Item) -> Result<std::collections::BTreeMap<String, String>, String> {
    let table = item
        .as_table()
        .ok_or_else(|| "artifact profile exports must be a table".to_owned())?;
    if table.iter().next().is_none() {
        return Err("artifact profile exports must not be empty".to_owned());
    }
    table
        .iter()
        .map(|(label, source)| {
            let source = source
                .as_str()
                .ok_or_else(|| format!("artifact profile export {label:?} must name a root function"))?;
            Ok((label.to_owned(), source.to_owned()))
        })
        .collect()
}

fn parse_target_profiles(
    item: Option<&Item>,
) -> Result<std::collections::BTreeMap<String, TargetProfile>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| "target profiles are missing".to_owned())?;
    let mut profiles = std::collections::BTreeMap::new();
    for (name, item) in table {
        let profile = item
            .as_table()
            .ok_or_else(|| format!("target profile {name} is not a table"))?;
        profiles.insert(
            name.to_owned(),
            TargetProfile {
                main_artifact: scalar_string(profile, "main_artifact")?,
            },
        );
    }
    Ok(profiles)
}

fn scalar_string(table: &toml_edit::Table, key: &str) -> Result<String, String> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} must be a string"))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactIdentity {
    pub source_identity: String,
    pub configuration_identity: String,
    pub artifact_identity: String,
    pub backend: String,
    pub output: String,
    pub builder_identity: String,
    pub runner_identity: String,
    pub llvm_input_identity: String,
    pub compiler_identity: String,
    pub llvm_identity: String,
    pub builder_toolchain_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub identity: ArtifactIdentity,
    pub executable: PathBuf,
    pub source_observations: Vec<SourceObservation>,
}

pub fn load_artifact_manifest(path: &Path) -> Result<ArtifactManifest, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

pub fn artifact_manifest_path(root: &Path, target: &str, profile: &str, artifact: &str) -> PathBuf {
    root.join(".wosy/artifacts")
        .join(target)
        .join(profile)
        .join(artifact)
        .join("artifact-manifest.json")
}

pub fn publish_artifact(
    output_dir: &Path,
    executable: PathBuf,
    identity: ArtifactIdentity,
    source_observations: Vec<SourceObservation>,
) -> Result<ArtifactManifest, String> {
    if !executable.is_file() {
        return Err(format!("builder did not produce {}", executable.display()));
    }
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let manifest = ArtifactManifest {
        identity,
        executable,
        source_observations,
    };
    let manifest_path = output_dir.join("artifact-manifest.json");
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    fs::write(manifest_path, bytes).map_err(|error| error.to_string())?;
    Ok(manifest)
}

impl ProjectContext {
    pub fn new(
        root: PathBuf,
        package: String,
        source_root: PathBuf,
        comment_categories: Vec<String>,
    ) -> Self {
        Self {
            root,
            package,
            source_root,
            comment_categories,
        }
    }

    pub fn load(root: &Path, target: &str) -> Result<Self, String> {
        let config_path = root.join("wosy.toml");
        let content = fs::read_to_string(&config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
        let document = content
            .parse::<DocumentMut>()
            .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
        Self::from_manifest(root, target, document.as_table())
    }

    pub fn from_manifest(
        root: &Path,
        target: &str,
        document: &toml_edit::Table,
    ) -> Result<Self, String> {
        let target_table = document
            .get("targets")
            .and_then(Item::as_table)
            .and_then(|targets| targets.get(target))
            .and_then(Item::as_table)
            .ok_or_else(|| format!("target {target} is missing from wosy.toml"))?;
        let source = target_table
            .get("root")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("target {target} has no root source"))?;
        let package = document
            .get("package")
            .and_then(Item::as_table)
            .and_then(|table| table.get("name"))
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| "package.name is missing from wosy.toml".to_owned())?
            .to_owned();
        let comment_categories = document
            .get("comments")
            .and_then(Item::as_table)
            .and_then(|table| table.get("categories"))
            .and_then(Item::as_value)
            .and_then(|value| value.as_array())
            .map(|array| {
                array
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .ok_or_else(|| "comment categories must be strings".to_owned())
                            .map(str::to_owned)
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .ok_or_else(|| "comments.categories is missing from wosy.toml".to_owned())?;
        Ok(Self::new(
            root.to_path_buf(),
            package,
            PathBuf::from(source),
            comment_categories,
        ))
    }

    pub fn source_path(&self) -> PathBuf {
        self.root.join(&self.source_root)
    }
}

pub fn discover_root(start: &Path) -> Result<PathBuf, String> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("wosy.toml").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            return Err("could not find wosy.toml".to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(value: &str) -> PackageRelativePath {
        PackageRelativePath::new(PathBuf::from(value)).expect("relative source path")
    }

    fn source(path_value: &str, edges: Vec<NamespaceEdgeInput>) -> SourceNodeInput {
        let path = path(path_value);
        SourceNodeInput {
            source: SourceIdentity::new("project".to_owned(), "app".to_owned(), path),
            content_revision: ContentRevision(format!("revision-{path_value}")),
            namespace_edges: edges,
        }
    }

    fn edge(source_path: &str, target_path: &str, binding: &str) -> NamespaceEdgeInput {
        NamespaceEdgeInput {
            origin: "app".to_owned(),
            path: path(target_path),
            binding: binding.to_owned(),
            span: SourceSpan {
                source: SourceIdentity::new(
                    "project".to_owned(),
                    "app".to_owned(),
                    path(source_path),
                ),
                range: ByteSpan::new(4, 28),
            },
        }
    }

    #[test]
    fn project_context_serializes_structured_configuration() {
        let context = ProjectContext::new(
            PathBuf::from("/workspace"),
            "app".to_owned(),
            PathBuf::from("src"),
            vec!["INTENT".to_owned(), "TYPE".to_owned()],
        );

        let value = serde_json::to_value(&context).expect("project context serialization");

        assert_eq!(value["package"], "app");
        assert_eq!(value["comment_categories"][0], "INTENT");
    }

    #[cfg(unix)]
    #[test]
    fn source_identity_preserves_lossy_path_display() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(b"src/\xff.w".to_vec()));
        let observation = SourceObservation {
            source: SourceIdentity::new(
                "project".into(),
                "app".into(),
                PackageRelativePath::new(path).expect("relative non-UTF path"),
            ),
            content_revision: ContentRevision("revision".into()),
        };
        assert_eq!(source_identity(&[observation]), "app:src/\u{fffd}.w:revision");
    }

    #[test]
    fn source_identity_preserves_observation_order_and_revisions() {
        let observations = vec![
            SourceObservation {
                source: SourceIdentity::new(
                    "project".to_owned(),
                    "app".to_owned(),
                    path("src/main.w"),
                ),
                content_revision: ContentRevision("main-revision".to_owned()),
            },
            SourceObservation {
                source: SourceIdentity::new(
                    "project".to_owned(),
                    "app".to_owned(),
                    path("src/math.w"),
                ),
                content_revision: ContentRevision("math-revision".to_owned()),
            },
        ];

        assert_eq!(source_identity(&[]), "");
        assert_eq!(
            source_identity(&observations[..1]),
            "app:src/main.w:main-revision"
        );
        assert_eq!(
            source_identity(&observations),
            "app:src/main.w:main-revision,app:src/math.w:math-revision"
        );
        let serialized = serde_json::to_value(&ArtifactManifest {
            identity: ArtifactIdentity {
                source_identity: source_identity(&observations),
                configuration_identity: "configuration".to_owned(),
                artifact_identity: "artifact".to_owned(),
                backend: "backend".to_owned(),
                output: "output".to_owned(),
                builder_identity: "builder".to_owned(),
                runner_identity: "runner".to_owned(),
                llvm_input_identity: "llvm".to_owned(),
                compiler_identity: "compiler".to_owned(),
                llvm_identity: "llvm".to_owned(),
                builder_toolchain_identity: "builder-toolchain".to_owned(),
            },
            executable: PathBuf::from("main"),
            source_observations: observations,
        })
        .expect("artifact manifest serialization");
        assert_eq!(
            serialized["source_observations"][0]["source"]["path"],
            "src/main.w"
        );
        assert_eq!(
            serialized["source_observations"][1]["content_revision"],
            "math-revision"
        );
    }

    #[test]
    fn configuration_resolves_required_default_target() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/build/scalar_base/project");
        let configuration = ProjectConfiguration::load(&root).expect("project configuration");

        assert_eq!(configuration.default_target, "app");
        assert_eq!(configuration.resolve_target(None), Ok("app".to_owned()));
        assert_eq!(
            configuration.resolve_target(Some("app")),
            Ok("app".to_owned())
        );
        assert_eq!(
            configuration.resolve_target(Some("missing")),
            Err("target missing is missing from wosy.toml".to_owned())
        );
        let invalid = ProjectConfiguration {
            default_target: "missing".to_owned(),
            builders: BTreeMap::new(),
            runners: BTreeMap::new(),
            targets: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            resolved_packages: BTreeMap::new(),
        };
        assert_eq!(
            invalid.resolve_target(None),
            Err("target missing is missing from wosy.toml".to_owned())
        );
    }

    #[test]
    fn artifact_profile_loads_wasm_wasip1_with_builder_and_runner_identities() {
        let document = r#"
            [artifacts.main]
            artifact = "runtime"

            [artifacts.main.profiles.dev]
            backend = "llvm"
            output = "wasm-wasip1"
            builder = "llvm-wasm"
            run_runner = "wasmtime-wasip1"
        "#
        .parse::<DocumentMut>()
        .expect("artifact configuration");

        let artifacts = parse_artifacts(document.as_table().get("artifacts"));
        let artifact = artifacts
            .expect("Wasm artifact configuration")
            .remove("main")
            .expect("main artifact");
        let profile = artifact.profiles.get("dev").expect("dev profile");

        assert_eq!(profile.output, "wasm-wasip1");
        assert_eq!(profile.builder, "llvm-wasm");
        assert_eq!(profile.run_runner, "wasmtime-wasip1");
    }

    #[test]
    fn artifact_root_exports_parse_optional_nonempty_tables() {
        let document = r#"
            [artifacts.main]
            artifact = "runtime"
            [artifacts.main.profiles.dev]
            backend = "llvm"
            output = "wasm-wasip1"
            builder = "wasm"
            run_runner = "wasm"
            [artifacts.main.profiles.dev.exports]
            demo_score = "score"
            tick = "step"
        "#;
        let parse = |source: &str| {
            let doc = source.parse::<DocumentMut>().expect("fixture TOML");
            parse_artifacts(doc.get("artifacts"))
        };
        for output in ["wasm-wasip1", "wasm-wasip2", "wasm-web"] {
            let artifacts = parse(&document.replace("wasm-wasip1", output)).expect("root exports");
            let profile = &artifacts["main"].profiles["dev"];
            assert_eq!(profile.exports.as_ref().unwrap()["demo_score"], "score");
            assert_eq!(profile.target_layout(), Ok(TargetLayout { pointer_size: 4, pointer_alignment: 4 }));
        }
        let native = parse(&document.replace("wasm-wasip1", "native")).expect("parse native profile");
        assert!(native["main"].profiles["dev"].target_layout().is_err());
        let empty = document.replace("demo_score = \"score\"", "").replace("tick = \"step\"", "");
        assert!(
            parse(&empty)
                .expect_err("empty export table")
                .contains("must not be empty")
        );
        assert!(parse(&document.replace("tick = \"step\"", "tick = 1")).is_err());
    }

    #[test]
    fn artifact_profile_derives_documented_llvm_target_layouts() {
        for output in ["wasm-wasip1", "wasm-wasip2", "wasm-web"] {
            let profile = ArtifactProfile {
                backend: "llvm".to_owned(),
                output: output.to_owned(),
                builder: "builder".to_owned(),
                run_runner: "runner".to_owned(),
                exports: None,
            };
            assert_eq!(
                profile.target_layout(),
                Ok(TargetLayout {
                    pointer_size: 4,
                    pointer_alignment: 4,
                })
            );
        }
        let native = ArtifactProfile {
            backend: "llvm".to_owned(),
            output: "native".to_owned(),
            builder: "builder".to_owned(),
            run_runner: "runner".to_owned(),
            exports: None,
        };
        assert_eq!(
            native.target_layout(),
            Ok(TargetLayout {
                pointer_size: 8,
                pointer_alignment: 8,
            })
        );
        let unsupported = ArtifactProfile {
            backend: "other".to_owned(),
            output: "custom".to_owned(),
            builder: "builder".to_owned(),
            run_runner: "runner".to_owned(),
            exports: None,
        };
        assert_eq!(
            unsupported.target_layout(),
            Err("unsupported artifact profile backend other with output custom".to_owned())
        );
    }

    #[test]
    fn builder_requires_string_toolchain_identity() {
        for declaration in ["", "toolchain_identity = 181"] {
            let document = format!(
                "[builders.native]\ncommand = [\"build-native\"]\nargs = []\n{declaration}"
            )
            .parse::<DocumentMut>()
            .expect("builder configuration");

            assert_eq!(
                named_commands(document.as_table().get("builders"), "builder"),
                Err("toolchain_identity must be a string".to_owned())
            );
        }
    }

    #[test]
    fn reachable_graph_visits_edges_in_source_order_once() {
        let root = source(
            "src/main.w",
            vec![
                edge("src/main.w", "src/math.w", "math"),
                edge("src/main.w", "src/math.w", "again"),
            ],
        );
        let graph = load_reachable_source_graph(
            "app",
            root,
            vec![source("src/math.w", Vec::new())],
            &BTreeMap::new(),
        )
        .expect("graph loads");

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(
            graph.initialization_order,
            vec![SourceNodeId(0), SourceNodeId(1)]
        );
        assert_eq!(graph.nodes[0].namespace_edges[0].target, SourceNodeId(1));
        assert_eq!(graph.nodes[0].namespace_edges[1].target, SourceNodeId(1));
        assert!(graph
            .nodes
            .iter()
            .all(|node| node.dfs_state == DfsState::Complete));
    }

    #[test]
    fn later_available_root_replaces_supplied_root() {
        let root = source("src/main.w", vec![edge("src/main.w", "src/missing.w", "old")]);
        let mut first = source(
            "src/main.w",
            vec![edge("src/main.w", "src/missing.w", "first")],
        );
        first.content_revision = ContentRevision("first".to_owned());
        let mut last = source("src/main.w", vec![edge("src/main.w", "src/math.w", "last")]);
        last.content_revision = ContentRevision("last".to_owned());
        let graph = load_reachable_source_graph(
            "app",
            root,
            vec![first, source("src/math.w", Vec::new()), last],
            &BTreeMap::new(),
        )
        .expect("later root supplies reachable edges");

        assert_eq!(
            graph.nodes[0].content_revision,
            ContentRevision("last".to_owned())
        );
        assert_eq!(graph.nodes[0].namespace_edges[0].binding, "last");
        assert_eq!(graph.nodes[0].namespace_edges[0].target, SourceNodeId(1));
    }

    #[test]
    fn missing_module_has_typed_m0001_requesting_span() {
        let root = source("src/main.w", vec![edge("src/main.w", "src/math.w", "math")]);
        let error = load_reachable_source_graph("app", root, Vec::new(), &BTreeMap::new())
            .expect_err("missing");

        assert_eq!(error.diagnostics.len(), 1);
        match &error.diagnostics[0] {
            GraphDiagnostic::MissingModule(diagnostic) => {
                assert_eq!(diagnostic.code, "M0001");
                assert_eq!(diagnostic.requested_path, path("src/math.w"));
                assert_eq!(diagnostic.requested_package, "app");
                assert_eq!(diagnostic.edge.source.path, path("src/main.w"));
                assert_eq!(diagnostic.edge.range, ByteSpan::new(4, 28));
            }
            GraphDiagnostic::Cycle(_) => panic!("expected missing module"),
        }
    }

    #[test]
    fn graph_resolves_namespace_against_declared_package_source_root() {
        let root = source(
            "src/main.w",
            vec![NamespaceEdgeInput {
                origin: "std".to_owned(),
                path: path("math.w"),
                binding: "std_math".to_owned(),
                span: SourceSpan {
                    source: SourceIdentity::new(
                        "app-project".to_owned(),
                        "app".to_owned(),
                        path("src/main.w"),
                    ),
                    range: ByteSpan::new(3, 30),
                },
            }],
        );
        let dependency = SourceNodeInput {
            source: SourceIdentity::new(
                "stdlib-project".to_owned(),
                "std".to_owned(),
                path("src/math.w"),
            ),
            content_revision: ContentRevision("std-math".to_owned()),
            namespace_edges: Vec::new(),
        };
        let mut resolved = BTreeMap::new();
        resolved.insert(
            "std".to_owned(),
            ResolvedPackage {
                name: "std".to_owned(),
                project_root: PathBuf::from("/stdlib"),
                source_root: PathBuf::from("/stdlib/src"),
                identity: SourceIdentity::new(
                    "stdlib-project".to_owned(),
                    "std".to_owned(),
                    path("src"),
                ),
            },
        );

        let graph = load_reachable_source_graph("app", root, vec![dependency], &resolved)
            .expect("package-qualified graph");

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].namespace_edges[0].target, SourceNodeId(1));
        assert_eq!(graph.nodes[1].source.package, "std");
        assert_eq!(graph.nodes[1].source.path, path("src/math.w"));
    }

    #[test]
    fn cycle_has_typed_m0003_participating_sources() {
        let root = source("src/main.w", vec![edge("src/main.w", "src/math.w", "math")]);
        let math = source("src/math.w", vec![edge("src/math.w", "src/main.w", "main")]);
        let error = load_reachable_source_graph("app", root, vec![math], &BTreeMap::new())
            .expect_err("cycle");

        match &error.diagnostics[0] {
            GraphDiagnostic::Cycle(diagnostic) => {
                assert_eq!(diagnostic.code, "M0003");
                assert_eq!(diagnostic.edge.source.path, path("src/math.w"));
                assert_eq!(diagnostic.cycle.len(), 3);
                assert_eq!(diagnostic.cycle[0].source.path, path("src/main.w"));
                assert_eq!(diagnostic.cycle[1].source.path, path("src/math.w"));
            }
            GraphDiagnostic::MissingModule(_) => panic!("expected cycle"),
        }
    }

    fn dependency_document(value: &str) -> toml_edit::ImDocument<String> {
        value.parse().expect("dependency configuration")
    }

    fn dependency_test_root() -> PathBuf {
        let current_thread = std::thread::current();
        let thread_name = current_thread.name().unwrap_or("test");
        let safe_thread_name: String = thread_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        std::env::temp_dir().join(format!(
            "wosy-project-dependency-{}-{}",
            std::process::id(),
            safe_thread_name
        ))
    }

    #[test]
    fn local_dependency_preserves_declaration_and_package_identity() {
        let root = dependency_test_root().join("valid");
        let package_root = root.join("../stdlib");
        fs::create_dir_all(package_root.join("src")).expect("package source root");
        fs::write(
            package_root.join("wosy.toml"),
            "[package]\nname = \"std\"\nsource_root = \"src\"\n",
        )
        .expect("package manifest");
        let document = dependency_document("[dependencies.std]\npath = \"../stdlib\"\n");
        let dependencies = parse_dependencies(&root, "app", document.get("dependencies"))
            .expect("local dependency");
        let declaration = dependencies.get("std").expect("std declaration");
        assert_eq!(declaration.name, "std");
        assert_eq!(
            declaration.source,
            DependencySource::Local {
                path: PathBuf::from("../stdlib"),
            }
        );
        assert!(declaration.span.range.end > declaration.span.range.start);
        let resolved =
            resolve_local_packages(&root, &dependencies).expect("resolved package");
        let package = resolved.get("std").expect("resolved std package");
        assert_eq!(package.name, "std");
        assert_eq!(package.project_root, package_root);
        assert_eq!(package.source_root, package_root.join("src"));
        assert_eq!(package.identity.package, "std");
        assert_eq!(package.identity.path, path("src"));
    }

    #[test]
    fn malformed_local_dependency_has_structured_diagnostic() {
        let root = dependency_test_root().join("malformed");
        let document = dependency_document("[dependencies]\nstd = \"../stdlib\"\n");
        let error = parse_dependencies(&root, "app", document.get("dependencies"))
            .expect_err("malformed dependency");
        assert_eq!(error.diagnostics[0].code, "P0001");
        assert_eq!(error.diagnostics[0].span.source.path, path("wosy.toml"));
        assert!(error.diagnostics[0].span.range.end > error.diagnostics[0].span.range.start);
    }

    #[test]
    fn project_configuration_load_reports_malformed_dependency_declaration_span() {
        let root = dependency_test_root().join("load-malformed");
        fs::create_dir_all(&root).expect("project root");
        let content = "[package]\nname = \"app\"\n\n[dependencies]\nstd = 42\n";
        fs::write(root.join("wosy.toml"), content).expect("project manifest");

        let error = ProjectConfiguration::load(&root).expect_err("malformed dependency");

        assert_eq!(error, "dependency std must be a table at 45..47");
    }

    fn dependency_project_manifest(dependencies: &str) -> String {
        format!(
            "[package]\nname = \"app\"\ndefault_target = \"app\"\n\n{dependencies}\n\n\
             [builders]\n[runners]\n[targets.app]\nroot = \"src/main.w\"\n\
             [targets.app.artifacts]\n[targets.app.profiles]\n"
        )
    }

    #[test]
    fn local_project_loading_preserves_nested_sibling_root_without_reading_the_lock() {
        let fixture = dependency_test_root();
        let project = fixture.join("project");
        let sibling_root = fixture.join("sibling");
        fs::create_dir_all(&project).expect("project directory");
        fs::create_dir_all(sibling_root.join("src/nested")).expect("sibling source directory");
        fs::write(
            sibling_root.join("wosy.toml"),
            "[package]\nname = \"sibling\"\nsource_root = \"src/nested\"\n",
        )
        .expect("sibling manifest");
        fs::write(
            project.join("wosy.toml"),
            dependency_project_manifest("[dependencies.sibling]\npath = \"../sibling\""),
        )
        .expect("project manifest");
        fs::write(project.join("wosy.lock"), "intentionally invalid lock")
            .expect("legacy lock contents");

        let configuration = ProjectConfiguration::load(&project).expect("local package loads");
        let sibling = &configuration.resolved_packages["sibling"];
        assert_eq!(sibling.project_root, project.join("../sibling"));
        assert_eq!(sibling.source_root, project.join("../sibling/src/nested"));
    }

    #[test]
    fn git_project_loading_never_uses_a_matching_lock_to_trust_an_unverified_tree() {
        let project = dependency_test_root();
        let revision = "0123456789abcdef0123456789abcdef01234567";
        fs::create_dir_all(&project).expect("project directory");
        fs::write(
            project.join("wosy.toml"),
            dependency_project_manifest("[dependencies.vendored]\ngit = \"../origin\""),
        )
        .expect("project manifest");
        let lock = format!(
            "version = 1\n[packages.vendored]\ngit = \"../origin\"\nrev = \"{revision}\"\n\
             path = \".\"\nsource_hash = \"{}\"\n",
            "0".repeat(64)
        );
        fs::write(project.join("wosy.lock"), lock).expect("valid lockfile");
        assert!(packages::PackageLock::read(&project).unwrap().is_some());

        for seeded in [false, true] {
            if seeded {
                let installed = project.join("deps/vendored");
                fs::create_dir_all(installed.join("src")).expect("seeded sources");
                fs::write(
                    installed.join("wosy.toml"),
                    "[package]\nname = \"vendored\"\nsource_root = \"src\"\n",
                )
                .expect("matching but unverified installed manifest");
            }
            assert_eq!(
                ProjectConfiguration::load(&project).unwrap_err(),
                "Git dependency vendored cannot yet be materialized",
                "seeded = {seeded}"
            );
        }
    }

    #[test]
    fn missing_local_dependency_manifest_has_structured_diagnostic() {
        let root = dependency_test_root().join("missing");
        fs::create_dir_all(&root).expect("project root");
        let document = dependency_document("[dependencies.std]\npath = \"../stdlib\"\n");
        let dependencies = parse_dependencies(&root, "app", document.get("dependencies"))
            .expect("local dependency");
        let error = resolve_local_packages(&root, &dependencies)
            .expect_err("missing package manifest");
        assert_eq!(error.diagnostics[0].code, "P0002");
        assert!(error.diagnostics[0].message.contains("manifest is missing"));
        assert_eq!(error.diagnostics[0].span.source.package, "app");
    }

    #[test]
    fn invalid_local_dependency_manifest_has_structured_diagnostic() {
        let root = dependency_test_root().join("invalid");
        let package_root = root.join("../stdlib");
        fs::create_dir_all(&package_root).expect("package root");
        fs::write(package_root.join("wosy.toml"), "[package").expect("invalid manifest");
        let document = dependency_document("[dependencies.std]\npath = \"../stdlib\"\n");
        let dependencies = parse_dependencies(&root, "app", document.get("dependencies"))
            .expect("local dependency");

        let error =
            resolve_local_packages(&root, &dependencies).expect_err("invalid manifest");

        assert_eq!(error.diagnostics[0].code, "P0002");
        assert!(error.diagnostics[0].message.contains("manifest is invalid"));
        assert!(error.diagnostics[0].span.range.end > error.diagnostics[0].span.range.start);
    }

    #[test]
    fn local_dependency_manifest_identity_must_match_declaration() {
        let root = dependency_test_root().join("identity");
        let package_root = root.join("../stdlib");
        fs::create_dir_all(package_root.join("src")).expect("package source root");
        fs::write(
            package_root.join("wosy.toml"),
            "[package]\nname = \"different\"\nsource_root = \"src\"\n",
        )
        .expect("package manifest");
        let document = dependency_document("[dependencies.std]\npath = \"../stdlib\"\n");
        let dependencies = parse_dependencies(&root, "app", document.get("dependencies"))
            .expect("local dependency");

        let error =
            resolve_local_packages(&root, &dependencies).expect_err("identity mismatch");

        assert_eq!(error.diagnostics[0].code, "P0002");
        assert!(error.diagnostics[0]
            .message
            .contains("package name is different"));
        assert_eq!(error.diagnostics[0].span.source.path, path("wosy.toml"));
    }
}
