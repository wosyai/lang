use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

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
    fn missing(edge: &NamespaceEdgeInput) -> Self {
        Self {
            diagnostics: vec![GraphDiagnostic::MissingModule(MissingModuleDiagnostic {
                code: "M0001".to_owned(),
                requested_path: edge.path.clone(),
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
) -> Result<SourceGraph, GraphLoadError> {
    let mut sources = BTreeMap::new();
    sources.insert(root.source.path.clone(), root.clone());
    for source in available {
        sources.insert(source.source.path.clone(), source);
    }

    let mut graph = SourceGraph {
        nodes: Vec::new(),
        initialization_order: Vec::new(),
    };
    let mut node_by_path = HashMap::new();
    let mut stack = Vec::new();
    visit_source(
        package,
        &root.source.path,
        &sources,
        &mut graph,
        &mut node_by_path,
        &mut stack,
        &SourceSpan {
            source: root.source.clone(),
            range: ByteSpan::new(0, 0),
        },
    )?;
    Ok(graph)
}

fn visit_source(
    package: &str,
    path: &PackageRelativePath,
    sources: &BTreeMap<PackageRelativePath, SourceNodeInput>,
    graph: &mut SourceGraph,
    node_by_path: &mut HashMap<PackageRelativePath, SourceNodeId>,
    stack: &mut Vec<SourceNodeId>,
    incoming: &SourceSpan,
) -> Result<SourceNodeId, GraphLoadError> {
    if let Some(id) = node_by_path.get(path) {
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
                    path: path.clone(),
                    binding: String::new(),
                    span: incoming.clone(),
                },
                cycle,
            ));
        }
        return Ok(*id);
    }

    let source = sources.get(path).ok_or_else(|| {
        GraphLoadError::missing(&NamespaceEdgeInput {
            origin: package.to_owned(),
            path: path.clone(),
            binding: String::new(),
            span: incoming.clone(),
        })
    })?;
    let id = SourceNodeId(graph.nodes.len());
    node_by_path.insert(path.clone(), id);
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
        if edge.origin != package {
            return Err(GraphLoadError::missing(edge));
        }
        let target = visit_source(
            package,
            &edge.path,
            sources,
            graph,
            node_by_path,
            stack,
            &edge.span,
        )?;
        graph.nodes[id.0].namespace_edges.push(NamespaceEdge {
            origin: edge.origin.clone(),
            path: edge.path.clone(),
            binding: edge.binding.clone(),
            span: edge.span.clone(),
            target,
        });
    }
    stack.pop();
    graph.nodes[id.0].dfs_state = DfsState::Complete;
    Ok(id)
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
}

impl ProjectConfiguration {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("wosy.toml");
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let document = content
            .parse::<DocumentMut>()
            .map_err(|error| error.to_string())?;
        let package = document
            .get("package")
            .and_then(Item::as_table)
            .ok_or_else(|| "package is missing".to_owned())?;
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
        Ok(Self {
            default_target,
            builders,
            runners,
            targets,
        })
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub identity: ArtifactIdentity,
    pub executable: PathBuf,
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
) -> Result<ArtifactManifest, String> {
    if !executable.is_file() {
        return Err(format!("builder did not produce {}", executable.display()));
    }
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let manifest = ArtifactManifest {
        identity,
        executable,
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
    fn reachable_graph_visits_edges_in_source_order_once() {
        let root = source(
            "src/main.w",
            vec![
                edge("src/main.w", "src/math.w", "math"),
                edge("src/main.w", "src/math.w", "again"),
            ],
        );
        let graph =
            load_reachable_source_graph("app", root, vec![source("src/math.w", Vec::new())])
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
    fn missing_module_has_typed_m0001_requesting_span() {
        let root = source("src/main.w", vec![edge("src/main.w", "src/math.w", "math")]);
        let error = load_reachable_source_graph("app", root, Vec::new()).expect_err("missing");

        assert_eq!(error.diagnostics.len(), 1);
        match &error.diagnostics[0] {
            GraphDiagnostic::MissingModule(diagnostic) => {
                assert_eq!(diagnostic.code, "M0001");
                assert_eq!(diagnostic.requested_path, path("src/math.w"));
                assert_eq!(diagnostic.edge.source.path, path("src/main.w"));
                assert_eq!(diagnostic.edge.range, ByteSpan::new(4, 28));
            }
            GraphDiagnostic::Cycle(_) => panic!("expected missing module"),
        }
    }

    #[test]
    fn cycle_has_typed_m0003_participating_sources() {
        let root = source("src/main.w", vec![edge("src/main.w", "src/math.w", "math")]);
        let math = source("src/math.w", vec![edge("src/math.w", "src/main.w", "main")]);
        let error = load_reachable_source_graph("app", root, vec![math]).expect_err("cycle");

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
}
