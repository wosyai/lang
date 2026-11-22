use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic as Report, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
#[cfg(test)]
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
#[cfg(target_os = "linux")]
use std::io::{BufWriter, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::process::ExitCode;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, emit_scalar_build_project_llvm_text,
    emit_scalar_build_single_module_llvm_text, parse_source, prepare_scalar_project, publication,
    serialize_publication, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_project::{
    artifact_manifest_path, discover_root, load_artifact_manifest, load_reachable_source_graph,
    publish_artifact, ArtifactIdentity, ContentRevision, NamespaceEdgeInput, PackageRelativePath,
    ProjectConfiguration, ProjectContext, SourceIdentity as ProjectSourceIdentity, SourceNodeInput,
    SourceObservation, SourceSpan as ProjectSourceSpan,
};
use wosy_syntax::SourceIdentity;

#[derive(Parser)]
#[command(name = "wosy")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Parse {
        target: Option<String>,
    },
    Build {
        target: Option<String>,
        #[arg(long, default_value = "dev")]
        profile: String,
    },
    Run {
        target: Option<String>,
        #[arg(long, default_value = "dev")]
        profile: String,
        #[arg(trailing_var_arg = true)]
        arguments: Vec<String>,
    },
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Parse { target } => match parse_command(target.as_deref()) {
            Ok(valid) => {
                if valid {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(error) => {
                eprintln!("wosy: {error}");
                ExitCode::from(1)
            }
        },
        Command::Build { target, profile } => match build_command(target.as_deref(), &profile) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("wosy: {error}");
                ExitCode::from(1)
            }
        },
        Command::Run {
            target,
            profile,
            arguments,
        } => match run_command(target.as_deref(), &profile, &arguments) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("wosy: {error}");
                ExitCode::from(1)
            }
        },
    }
}

fn parse_command(target: Option<&str>) -> Result<bool, String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let (configuration, document) = ProjectConfiguration::load_with_manifest(&root)?;
    let target_name = configuration.resolve_target(target)?;
    let project = ProjectContext::from_manifest(&root, &target_name, document.as_table())?;
    drop(document);
    let source_path = project.source_path();
    let text = fs::read_to_string(&source_path)
        .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
    let relative = source_path
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let revision = blake3::hash(text.as_bytes()).to_hex().to_string();
    let source = SourceIdentity::new(
        root.to_string_lossy().into_owned(),
        project.package,
        relative,
        revision,
    );
    let output = parse_source(source, text, &project.comment_categories);
    persist_cst(&root, &output)?;
    let view = publication(&output);
    let json = serialize_publication(&view).map_err(|error| error.to_string())?;
    let output_path = root.join(".wosy/cst/current.json");
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&output_path, format!("{json}\n")).map_err(|error| error.to_string())?;
    render_diagnostics(
        &output.result.text,
        &output.diagnostics,
        &output.result.source,
    );
    Ok(output.diagnostics.is_empty())
}

fn build_command(target: Option<&str>, profile: &str) -> Result<(), String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let (configuration, document) = ProjectConfiguration::load_with_manifest(&root)?;
    let target_name = configuration.resolve_target(target)?;
    let project = ProjectContext::from_manifest(&root, &target_name, document.as_table())?;
    let (_target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(&target_name, profile)?;
    let target_layout = artifact_profile.target_layout().map(scalar_target_layout)?;
    let configuration_identity = blake3::hash(document.raw().as_bytes()).to_hex().to_string();
    drop(document);
    let (llvm, source_observations) =
        compile_project(&root, &project, &configuration, target_layout)?;
    let source_identity = wosy_project::source_identity(&source_observations);
    let artifact_name = &target_profile.main_artifact;
    let output_dir = root
        .join(".wosy/artifacts")
        .join(&target_name)
        .join(profile)
        .join(artifact_name);
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let partition_path = output_dir.join("partition.ll");
    fs::write(&partition_path, &llvm).map_err(|error| error.to_string())?;
    let runtime_path = output_dir.join("runtime.ll");
    let runtime = wosy_compiler::core_runtime(&artifact_profile.backend, &artifact_profile.output)?;
    fs::write(&runtime_path, runtime).map_err(|error| error.to_string())?;
    let identity = artifact_identity(
        configuration_identity,
        &configuration,
        &target_name,
        profile,
        artifact,
        artifact_profile,
        &source_identity,
        &llvm,
    )?;
    drop(llvm);
    let executable = output_dir.join("main");
    let pending_manifest = output_dir.join("artifact-manifest.pending.json");
    let pending = wosy_project::ArtifactManifest {
        identity: identity.clone(),
        executable: executable.clone(),
        source_observations: source_observations.clone(),
    };
    let pending_bytes = serde_json::to_vec_pretty(&pending).map_err(|error| error.to_string())?;
    fs::write(&pending_manifest, pending_bytes).map_err(|error| error.to_string())?;
    let builder = configuration
        .builders
        .get(&artifact_profile.builder)
        .ok_or_else(|| format!("builder {} is missing", artifact_profile.builder))?;
    invoke_direct(
        &root,
        &builder.command,
        &builder.args,
        &[
            partition_path.to_string_lossy().as_ref(),
            output_dir.to_string_lossy().as_ref(),
            pending_manifest.to_string_lossy().as_ref(),
            runtime_path.to_string_lossy().as_ref(),
        ],
    )?;
    publish_artifact(&output_dir, executable, identity, source_observations)?;
    fs::remove_file(pending_manifest).map_err(|error| error.to_string())?;
    Ok(())
}

fn run_command(target: Option<&str>, profile: &str, arguments: &[String]) -> Result<u8, String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let (configuration, document) = ProjectConfiguration::load_with_manifest(&root)?;
    let target_name = configuration.resolve_target(target)?;
    let (_target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(&target_name, profile)?;
    let configuration_identity = blake3::hash(document.raw().as_bytes()).to_hex().to_string();
    drop(document);
    let manifest_path =
        artifact_manifest_path(&root, &target_name, profile, &target_profile.main_artifact);
    let manifest = load_artifact_manifest(&manifest_path)
        .map_err(|error| format!("artifact manifest error: {error}"))?;
    let current_observations = manifest
        .source_observations
        .iter()
        .map(|observation| {
            let path = source_observation_path(observation);
            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            Ok(SourceObservation {
                source: observation.source.clone(),
                content_revision: ContentRevision(blake3::hash(&bytes).to_hex().to_string()),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let builder = configuration
        .builders
        .get(&artifact_profile.builder)
        .ok_or_else(|| format!("builder {} is missing", artifact_profile.builder))?;
    let runner = configuration
        .runners
        .get(&artifact_profile.run_runner)
        .ok_or_else(|| format!("runner {} is missing", artifact_profile.run_runner))?;
    let identity = &manifest.identity;
    if identity.source_identity != wosy_project::source_identity(&current_observations)
        || identity.configuration_identity != configuration_identity
        || identity.artifact_identity != format!("{target_name}:{profile}:{}", artifact.artifact)
        || identity.backend != artifact_profile.backend
        || identity.output != artifact_profile.output
        || identity.builder_identity
            != serde_json::to_string(builder).map_err(|error| error.to_string())?
        || identity.runner_identity
            != serde_json::to_string(runner).map_err(|error| error.to_string())?
        || identity.compiler_identity != compiler_identity()
        || identity.llvm_identity != llvm_identity()
        || identity.builder_toolchain_identity != builder.toolchain_identity
    {
        return Err("artifact manifest is stale".to_owned());
    }
    if !manifest.executable.is_file() {
        return Err(format!(
            "artifact is missing: {}",
            manifest.executable.display()
        ));
    }
    invoke_direct_with_arguments(
        &root,
        &runner.command,
        &runner.args,
        &manifest_path,
        arguments,
    )
}

fn source_observation_path(observation: &SourceObservation) -> std::path::PathBuf {
    std::path::Path::new(&observation.source.project).join(observation.source.path.as_path())
}

struct CanonicalProjectInputs {
    declarations: Vec<(wosy_compiler::ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    graph: wosy_project::SourceGraph,
    texts: BTreeMap<ProjectSourceIdentity, String>,
    #[cfg(feature = "internal-checker")]
    proofs: BTreeMap<PackageRelativePath, (wosy_compiler::ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
}

enum CanonicalLoadPurpose {
    Build,
    #[cfg(feature = "internal-checker")]
    Check,
    #[cfg(feature = "internal-checker")]
    Proof(HashSet<PackageRelativePath>),
}

enum CanonicalSourceDeclarations {
    Ordinary(wosy_compiler::ScalarSourceDeclarations),
    #[cfg(feature = "internal-checker")]
    Proof(wosy_compiler::ScalarProofDeclarations),
}

impl CanonicalSourceDeclarations {
    fn declarations(&self) -> &wosy_compiler::ScalarSourceDeclarations {
        match self {
            Self::Ordinary(declarations) => declarations,
            #[cfg(feature = "internal-checker")]
            Self::Proof(proof) => proof.declarations(),
        }
    }
}

#[derive(Debug)]
enum CanonicalLoadFailure {
    Input(String),
    Syntax {
        source: SourceIdentity,
        text: String,
        diagnostics: Vec<wosy_compiler::Diagnostic>,
    },
    Derivation {
        diagnostics: Vec<wosy_compiler::Diagnostic>,
        texts: BTreeMap<ProjectSourceIdentity, String>,
    },
    Graph {
        error: wosy_project::GraphLoadError,
        texts: BTreeMap<ProjectSourceIdentity, String>,
    },
}

impl From<String> for CanonicalLoadFailure {
    fn from(error: String) -> Self {
        Self::Input(error)
    }
}

impl CanonicalLoadFailure {
    fn report_build(self) -> String {
        match self {
            Self::Input(error) => error,
            Self::Syntax { source, text, diagnostics } => {
                render_diagnostics(&text, &diagnostics, &source);
                "build rejected by syntax diagnostics".to_owned()
            }
            Self::Derivation { diagnostics, texts } => {
                render_project_diagnostics(&diagnostics, &texts);
                "build rejected by CST derivation diagnostics".to_owned()
            }
            Self::Graph { error, texts } => {
                render_graph_diagnostics(&error, &texts);
                format_graph_error(error)
            }
        }
    }
}

fn canonical_syntax_failure(output: wosy_compiler::ParseOutput) -> CanonicalLoadFailure {
    CanonicalLoadFailure::Syntax {
        source: output.result.source,
        text: output.result.text,
        diagnostics: output.diagnostics,
    }
}

fn canonical_source_identity(
    project_identity: String,
    package: &str,
    path: &PackageRelativePath,
    text: &str,
) -> SourceIdentity {
    SourceIdentity::new(
        project_identity,
        package.to_owned(),
        path.as_path().to_string_lossy().replace('\\', "/"),
        blake3::hash(text.as_bytes()).to_hex().to_string(),
    )
}

fn parse_canonical_ordinary_source(
    source: SourceIdentity,
    text: String,
    comment_categories: &[String],
    persist_in: Option<&std::path::Path>,
) -> Result<(String, wosy_compiler::ScalarSourceDeclarations), CanonicalLoadFailure> {
    let output = parse_source(source, text, comment_categories);
    if !output.diagnostics.is_empty() {
        return Err(canonical_syntax_failure(output));
    }
    if let Some(root) = persist_in {
        persist_cst(root, &output)?;
    }
    let declarations = derive_scalar_declarations_from_cst(output.result.canonical_cst());
    Ok((output.result.text, declarations))
}

fn load_canonical_project_inputs(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    purpose: CanonicalLoadPurpose,
) -> Result<CanonicalProjectInputs, CanonicalLoadFailure> {
    let root_path = PackageRelativePath::new(project.source_root.clone())?;
    let root_project = root.to_string_lossy().into_owned();
    let mut discovered = HashMap::from([(
        project.package.as_str(),
        HashSet::from([root_path.clone()]),
    )]);
    let mut pending = VecDeque::from([(project.package.clone(), root_path)]);
    let mut nodes = Vec::new();
    let mut declarations_by_path = BTreeMap::new();
    let mut texts = BTreeMap::new();

    while let Some((package, path)) = pending.pop_front() {
        let (package_root, project_identity) = if package == project.package {
            (root.to_path_buf(), root_project.clone())
        } else {
            let resolved = configuration
                .resolved_packages
                .get(&package)
                .ok_or_else(|| format!("dependency package {package} is missing"))?;
            (
                resolved.project_root.clone(),
                resolved.identity.project.clone(),
            )
        };
        let source_path = package_root.join(path.as_path());
        let text = fs::read_to_string(&source_path)
            .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
        let source = canonical_source_identity(project_identity, &package, &path, &text);
        let (text, declarations) = match &purpose {
            #[cfg(feature = "internal-checker")]
            CanonicalLoadPurpose::Proof(selected) if package == project.package && selected.contains(&path) => {
                let (text, proof) = wosy_compiler::parse_proof_source(source, text, &project.comment_categories)
                    .map_err(canonical_syntax_failure)?;
                (text, CanonicalSourceDeclarations::Proof(proof))
            }
            _ => {
                let (text, declarations) = parse_canonical_ordinary_source(
                    source, text, &project.comment_categories,
                    matches!(purpose, CanonicalLoadPurpose::Build).then_some(root),
                )?;
                (text, CanonicalSourceDeclarations::Ordinary(declarations))
            }
        };
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
        let source = declarations.declarations().source();
        let mut namespace_edges = Vec::new();
        for namespace in declarations.declarations().namespaces() {
            let module_path = serde_json::from_str::<String>(&namespace.path)
                .map_err(|error| format!("invalid namespace path: {error}"))?;
            let module_path = PackageRelativePath::new(module_path.into())?;
            let (target_package, target_root, target_path) = if namespace.package == project.package {
                (project.package.as_str(), root, Cow::Borrowed(&module_path))
            } else {
                let (package, resolved) = configuration
                    .resolved_packages
                    .get_key_value(&namespace.package)
                    .ok_or_else(|| format!("dependency package {} is missing", namespace.package))?;
                let target_path = PackageRelativePath::new(
                    resolved.identity.path.as_path().join(module_path.as_path()),
                )?;
                (
                    package.as_str(),
                    resolved.project_root.as_path(),
                    Cow::Owned(target_path),
                )
            };
            let already_discovered = discovered
                .get(target_package)
                .is_some_and(|paths| paths.contains(target_path.as_ref()));
            if !already_discovered && target_root.join(target_path.as_path()).is_file() {
                let target_path = target_path.into_owned();
                discovered
                    .entry(target_package)
                    .or_default()
                    .insert(target_path.clone());
                pending.push_back((target_package.to_owned(), target_path));
            }
            namespace_edges.push(NamespaceEdgeInput {
                origin: namespace.package.clone(),
                path: module_path,
                binding: namespace.binding.clone(),
                span: ProjectSourceSpan {
                    source: ProjectSourceIdentity::new(
                        source.project.clone(),
                        source.package.clone(),
                        path.clone(),
                    ),
                    range: wosy_project::ByteSpan::new(namespace.span.start, namespace.span.end),
                },
            });
        }
        let graph_source = ProjectSourceIdentity::new(
            source.project.clone(),
            source.package.clone(),
            path.clone(),
        );
        nodes.push(SourceNodeInput {
            source: graph_source.clone(),
            content_revision: ContentRevision(source.revision.clone()),
            namespace_edges,
        });
        declarations_by_path.insert((package, path), declarations);
        texts.insert(graph_source, text);
    }
    drop((discovered, pending));

    let root_node = nodes
        .first()
        .cloned()
        .ok_or_else(|| "project root source is missing".to_owned())?;
    let graph = match load_reachable_source_graph(
        &project.package,
        root_node,
        nodes,
        &configuration.resolved_packages,
    ) {
        Ok(graph) => graph,
        Err(error) => return Err(CanonicalLoadFailure::Graph { error, texts }),
    };
    let mut project_declarations = Vec::new();
    #[cfg(feature = "internal-checker")]
    let mut proofs = BTreeMap::new();
    let mut source_by_path = BTreeMap::new();
    let mut derivation_diagnostics = Vec::new();
    for node in &graph.nodes {
        let source = compiler_source_identity(&node.source, &node.content_revision);
        source_by_path.insert(
            (node.source.package.clone(), node.source.path.clone()),
            source,
        );
    }
    for node in &graph.nodes {
        let declarations = declarations_by_path
            .remove(&(node.source.package.clone(), node.source.path.clone()))
            .ok_or_else(|| {
                format!(
                    "missing canonical CST for {}",
                    node.source.path.as_path().display()
                )
            })?;
        derivation_diagnostics.extend_from_slice(declarations.declarations().diagnostics());
        let namespace_bindings = node
            .namespace_edges
            .iter()
            .map(|edge| {
                Ok(ScalarNamespaceBinding {
                    binding: edge.binding.clone(),
                    target: source_by_path
                        .get(&(edge.origin.clone(), edge.path.clone()))
                        .cloned()
                        .ok_or_else(|| {
                            format!("missing module {}", edge.path.as_path().display())
                        })?,
                    span: wosy_syntax::ByteSpan::new(edge.span.range.start, edge.span.range.end),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        match declarations {
            CanonicalSourceDeclarations::Ordinary(declarations) => project_declarations.push((declarations, namespace_bindings)),
            #[cfg(feature = "internal-checker")]
            CanonicalSourceDeclarations::Proof(declarations) => {
                proofs.insert(node.source.path.clone(), (declarations, namespace_bindings));
            }
        }
    }
    if !derivation_diagnostics.is_empty() {
        return Err(CanonicalLoadFailure::Derivation { diagnostics: derivation_diagnostics, texts });
    }
    let initialization_order = graph
        .initialization_order
        .iter()
        .map(|id| {
            let node = graph
                .nodes
                .get(id.0)
                .ok_or_else(|| format!("missing initialization node {}", id.0))?;
            source_by_path
                .get(&(node.source.package.clone(), node.source.path.clone()))
                .cloned()
                .ok_or_else(|| format!("missing module {}", node.source.path.as_path().display()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CanonicalProjectInputs {
        declarations: project_declarations,
        initialization_order,
        graph,
        texts,
        #[cfg(feature = "internal-checker")]
        proofs,
    })
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
struct InternalCheckReceipt<Outcome = wosy_compiler::ScalarCheckerOutcome> {
    outcome: Outcome,
    texts: BTreeMap<ProjectSourceIdentity, String>,
}

#[cfg(feature = "internal-checker")]
type InternalOrderedCheckReceipt = InternalCheckReceipt<wosy_compiler::ScalarOrderedCheckerOutcome>;

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, PartialEq)]
enum InternalCheckStatus<'a> {
    Zero,
    FirstNonzero(&'a wosy_compiler::ScalarOrderedCheckerFailure),
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, PartialEq)]
enum InternalCheckEffect<'a> {
    Stdout(&'a [u8]),
    Stderr(&'a [u8]),
    DeferredFile(&'a wosy_compiler::ScalarCheckerTextFile),
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
impl InternalCheckReceipt<wosy_compiler::ScalarOrderedCheckerOutcome> {
    fn selected_status(&self) -> Option<InternalCheckStatus<'_>> {
        use wosy_compiler::ScalarOrderedCheckerOutcome;
        match &self.outcome {
            ScalarOrderedCheckerOutcome::Passed | ScalarOrderedCheckerOutcome::NoChecks =>
                Some(InternalCheckStatus::Zero),
            ScalarOrderedCheckerOutcome::Failed(reports) => Some(reports.iter().find(|report| report.code().bits() != 0)
                .map_or(InternalCheckStatus::Zero, InternalCheckStatus::FirstNonzero)),
            ScalarOrderedCheckerOutcome::Unsupported | ScalarOrderedCheckerOutcome::Invalid(_) => None,
        }
    }

    fn visit_authored_effects<E>(
        &self,
        mut sink: impl FnMut(InternalCheckEffect<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        let wosy_compiler::ScalarOrderedCheckerOutcome::Failed(reports) = &self.outcome else {
            return Ok(());
        };
        for report in reports {
            if !report.stdout().is_empty() {
                sink(InternalCheckEffect::Stdout(report.stdout()))?;
            }
            if !report.stderr().is_empty() {
                sink(InternalCheckEffect::Stderr(report.stderr()))?;
            }
            for file in report.files() {
                sink(InternalCheckEffect::DeferredFile(file))?;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "internal-checker")]
struct DormantSourceInput {
    identity: ProjectSourceIdentity,
    text: String,
}

#[cfg(feature = "internal-checker")]
fn prepare_explicit_dormant_sources(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    sources: Vec<DormantSourceInput>,
    texts: &mut BTreeMap<ProjectSourceIdentity, String>,
) -> Result<Vec<wosy_compiler::ScalarSourceDeclarations>, CanonicalLoadFailure> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let project_identity = root.to_string_lossy();
    let dependency_prefixes = configuration.resolved_packages.values()
        .filter_map(|dependency| dependency.project_root.strip_prefix(root).ok())
        .collect::<Vec<_>>();
    let mut selected = HashSet::with_capacity(sources.len());
    for DormantSourceInput { identity, .. } in &sources {
        use std::path::Component;
        let relative = identity.path.as_path();
        let spelling = relative.to_str().ok_or_else(|| "dormant source path must be UTF-8".to_owned())?;
        if spelling.is_empty() || spelling.contains('\\')
            || !relative.components().all(|component| matches!(component, Component::Normal(_)))
            || relative.components().collect::<std::path::PathBuf>().as_os_str() != relative.as_os_str()
        {
            return Err(format!("dormant source path is not canonical: {spelling}").into());
        }
        if identity.project != project_identity || identity.package != project.package
            || relative.starts_with("deps")
            || dependency_prefixes.iter().any(|prefix| relative.starts_with(prefix))
        {
            return Err(format!("dormant source is outside the current package: {spelling}").into());
        }
        if texts.contains_key(identity) || !selected.insert(identity) {
            return Err(format!("dormant source was already selected: {spelling}").into());
        }
    }

    sources.into_iter().map(|DormantSourceInput { identity, text }| {
        let source = canonical_source_identity(identity.project.clone(), &identity.package, &identity.path, &text);
        let (text, declarations) = parse_canonical_ordinary_source(
            source, text, &project.comment_categories, None,
        )?;
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
        texts.insert(identity, text);
        Ok(declarations)
    }).collect()
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    scope: wosy_compiler::ScalarCheckerScope,
    dormant_sources: Vec<DormantSourceInput>,
) -> Result<InternalCheckReceipt, CanonicalLoadFailure> {
    check_loaded_project_with(
        root, project, configuration, target_layout, scope, dormant_sources,
        wosy_compiler::check_scalar_package_with_dormant_sources,
    )
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_ordered_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    scope: wosy_compiler::ScalarCheckerScope,
    dormant_sources: Vec<DormantSourceInput>,
) -> Result<InternalOrderedCheckReceipt, CanonicalLoadFailure> {
    check_loaded_project_with(
        root, project, configuration, target_layout, scope, dormant_sources,
        wosy_compiler::check_scalar_package_ordered_with_dormant_sources,
    )
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_ordered_dependency(
    selection: &wosy_project::DirectDependencyCheckTarget,
) -> Result<InternalOrderedCheckReceipt, CanonicalLoadFailure> {
    check_loaded_ordered_project(
        &selection.project.root, &selection.project, &selection.configuration,
        scalar_target_layout(selection.layout), wosy_compiler::ScalarCheckerScope::CurrentPackage, Vec::new(),
    )
}

#[cfg(feature = "internal-checker")]
fn check_loaded_project_with<Outcome>(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    scope: wosy_compiler::ScalarCheckerScope,
    dormant_sources: Vec<DormantSourceInput>,
    check: impl FnOnce(
        Vec<(wosy_compiler::ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        Vec<SourceIdentity>,
        Vec<wosy_compiler::ScalarSourceDeclarations>,
        ScalarPreparationInputs,
        wosy_compiler::ScalarCheckerScope,
    ) -> Outcome,
) -> Result<InternalCheckReceipt<Outcome>, CanonicalLoadFailure> {
    let CanonicalProjectInputs { declarations, initialization_order, mut texts, .. } =
        load_canonical_project_inputs(root, project, configuration, CanonicalLoadPurpose::Check)?;
    let dormant = if scope == wosy_compiler::ScalarCheckerScope::CurrentPackage {
        prepare_explicit_dormant_sources(root, project, configuration, dormant_sources, &mut texts)?
    } else {
        Vec::new()
    };
    #[cfg(test)]
    routing_observations::record(|counts| counts.checks += 1);
    let outcome = check(
        declarations, initialization_order, dormant, ScalarPreparationInputs { target_layout }, scope,
    );
    Ok(InternalCheckReceipt { outcome, texts })
}

#[cfg(feature = "internal-checker")]
type InternalProofResults = Result<
    (Result<Vec<wosy_compiler::ScalarModuleProofOutcome>, wosy_compiler::ScalarModuleProofOutcome>, BTreeMap<ProjectSourceIdentity, String>),
    CanonicalLoadFailure,
>;

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_proofs(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    proof_file: PackageRelativePath,
    proof_names: &[&str],
) -> InternalProofResults {
    let selections = proof_names.iter().map(|&name| (&proof_file, name)).collect::<Vec<_>>();
    check_loaded_proof_selections(root, project, configuration, target_layout, &selections)
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_proof_selections(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    selections: &[(&PackageRelativePath, &str)],
) -> InternalProofResults {
    if selections.is_empty() {
        return Ok((Ok(Vec::new()), BTreeMap::new()));
    }
    let selected = selections.iter().map(|(path, _)| (*path).clone()).collect();
    let loaded = load_canonical_project_inputs(root, project, configuration, CanonicalLoadPurpose::Proof(selected))?;
    let mut authorities = HashMap::new();
    for &(path, _) in selections {
        if let std::collections::hash_map::Entry::Vacant(entry) = authorities.entry(path) {
            let Some((proof, _)) = loaded.proofs.get(path) else {
                return Err(selected_source_outside_graph("proof", path));
            };
            entry.insert(proof.declarations().source().clone());
        }
    }
    let proofs = selections.iter().map(|&(path, name)| (&authorities[path], name)).collect::<Vec<_>>();
    #[cfg(test)]
    routing_observations::record(|counts| counts.checks += 1);
    let outcomes = wosy_compiler::check_loaded_scalar_module_proof_selections(
        &proofs, loaded.declarations, loaded.proofs.into_values().collect(), loaded.initialization_order,
        ScalarPreparationInputs { target_layout },
    );
    Ok((outcomes, loaded.texts))
}

#[cfg(feature = "internal-checker")]
fn selected_source_outside_graph(role: &str, path: &PackageRelativePath) -> CanonicalLoadFailure {
    CanonicalLoadFailure::Input(format!(
        "selected {role} source {} is outside the canonical target graph",
        path.as_path().display(),
    ))
}

#[cfg(feature = "internal-checker")]
type InternalProofCallResult = Result<
    (wosy_compiler::ScalarProofCallOutcome, BTreeMap<ProjectSourceIdentity, String>),
    CanonicalLoadFailure,
>;

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_proof_call(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    (proof_file, proof_name): (&PackageRelativePath, &str),
    (caller_file, caller_name): (&PackageRelativePath, &str),
) -> InternalProofCallResult {
    let mut loaded = load_canonical_project_inputs(
        root, project, configuration, CanonicalLoadPurpose::Proof(HashSet::from([proof_file.clone()])),
    )?;
    let catalog = loaded.proofs.remove(proof_file).ok_or_else(|| selected_source_outside_graph("proof", proof_file))?;
    let caller_node = loaded.graph.nodes.iter().find(|node| {
        node.source.package == project.package && &node.source.path == caller_file
    }).ok_or_else(|| selected_source_outside_graph("caller", caller_file))?;
    let caller_source = compiler_source_identity(&caller_node.source, &caller_node.content_revision);
    let proof_source = catalog.0.declarations().source().clone();
    #[cfg(test)]
    routing_observations::record(|counts| counts.checks += 1);
    let outcome = wosy_compiler::check_loaded_scalar_module_proof_call(
        (&proof_source, proof_name), (&caller_source, caller_name), loaded.declarations, catalog,
        loaded.initialization_order, ScalarPreparationInputs { target_layout },
    );
    Ok((outcome, loaded.texts))
}

fn compile_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
) -> Result<(String, Vec<SourceObservation>), String> {
    let CanonicalProjectInputs {
        declarations,
        initialization_order,
        graph,
        texts,
        ..
    } = load_canonical_project_inputs(root, project, configuration, CanonicalLoadPurpose::Build)
        .map_err(CanonicalLoadFailure::report_build)?;
    #[cfg(test)]
    routing_observations::record(|counts| counts.preparations += 1);
    let prepared = prepare_scalar_project(
        declarations,
        initialization_order,
        ScalarPreparationInputs { target_layout },
    )
    .map_err(|rejected| {
        render_project_diagnostics(&rejected.into_ordered_diagnostics(), &texts);
        "build rejected by semantic diagnostics".to_owned()
    })?;
    #[cfg(test)]
    routing_observations::record(|counts| counts.builds += 1);
    let validation = analyze_scalar_build(&prepared);
    render_project_diagnostics(validation.diagnostics(), &texts);
    if !validation.diagnostics().is_empty() {
        return Err("build rejected by semantic diagnostics".to_owned());
    }
    drop(texts);
    let llvm = if graph.nodes.len() == 1 {
        #[cfg(test)]
        routing_observations::record(|counts| counts.single_module_emissions += 1);
        emit_scalar_build_single_module_llvm_text(&validation)?
    } else {
        #[cfg(test)]
        routing_observations::record(|counts| counts.project_emissions += 1);
        emit_scalar_build_project_llvm_text(&validation)?
    };
    let source_observations = graph
        .nodes
        .into_iter()
        .map(|node| SourceObservation {
            source: node.source,
            content_revision: node.content_revision,
        })
        .collect();
    Ok((llvm, source_observations))
}

fn scalar_target_layout(layout: wosy_project::TargetLayout) -> ScalarTargetLayout {
    match (layout.pointer_size, layout.pointer_alignment) {
        (4, 4) => ScalarTargetLayout::WASM32,
        (8, 8) => ScalarTargetLayout::NATIVE64,
        _ => unreachable!("project target layouts are supported"),
    }
}

fn persist_cst(root: &std::path::Path, output: &wosy_compiler::ParseOutput) -> Result<(), String> {
    let path = root
        .join(".wosy/cst")
        .join(format!("{}.json", output.result.source.revision));
    let create_parent = || {
        path.parent().map_or(Ok(()), |parent| {
            fs::create_dir_all(parent).map_err(|error| error.to_string())
        })
    };
    let snapshot = output.result.canonical_cst().snapshot_json_serializable();
    #[cfg(target_os = "linux")]
    {
        create_parent()?;
        publish_cst_snapshot(&path, &snapshot)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let bytes = serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?;
        create_parent()?;
        fs::write(&path, bytes).map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "linux")]
fn publish_cst_snapshot(
    path: &std::path::Path,
    snapshot: &impl serde::Serialize,
) -> Result<(), String> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let (temporary, file) = loop {
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_file_name(format!(".cst-{pid}-{nonce}.tmp"));
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    };
    let result = (|| {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, snapshot)
            .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        match fs::symlink_metadata(path) {
            Ok(existing) if existing.is_file() => {
                if existing.mode() & 0o200 == 0 || existing.nlink() != 1 {
                    return Err("canonical CST snapshot destination is not a writable single-link file".to_owned());
                }
                let created = writer.get_ref().metadata().map_err(|error| error.to_string())?;
                if existing.uid() != created.uid() {
                    return Err("canonical CST snapshot destination has a different owner".to_owned());
                }
            }
            Ok(_) => return Err("canonical CST snapshot destination is not a regular file".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
        drop(writer);
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
fn load_canonical_cst(path: &std::path::Path) -> Result<wosy_syntax::CanonicalCstRoot, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes.as_slice());
    deserializer.disable_recursion_limit();
    let snapshot = wosy_syntax::CanonicalCstSnapshot::deserialize(&mut deserializer)
        .map_err(|error| error.to_string())?;
    deserializer.end().map_err(|error| error.to_string())?;
    Ok(wosy_syntax::CanonicalCstRoot::from_snapshot(snapshot))
}

fn compiler_source_identity(
    source: &ProjectSourceIdentity,
    revision: &ContentRevision,
) -> SourceIdentity {
    SourceIdentity::new(
        source.project.clone(),
        source.package.clone(),
        source.path.as_path().to_string_lossy().replace('\\', "/"),
        revision.0.clone(),
    )
}

fn format_graph_error(error: wosy_project::GraphLoadError) -> String {
    error
        .diagnostics
        .into_iter()
        .map(|diagnostic| match diagnostic {
            wosy_project::GraphDiagnostic::MissingModule(diagnostic) => format!(
                "{}: missing module {}::{}",
                diagnostic.code,
                diagnostic.requested_package,
                diagnostic.requested_path.as_path().display()
            ),
            wosy_project::GraphDiagnostic::Cycle(diagnostic) => {
                format!("{}: import cycle", diagnostic.code)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_graph_diagnostics(
    error: &wosy_project::GraphLoadError,
    texts: &BTreeMap<ProjectSourceIdentity, String>,
) {
    for diagnostic in &error.diagnostics {
        match diagnostic {
            wosy_project::GraphDiagnostic::MissingModule(diagnostic) => {
                let mut files = SimpleFiles::new();
                let Some(text) = texts.get(&diagnostic.edge.source) else {
                    continue;
                };
                let file_id = files.add(
                    diagnostic.edge.source.path.as_path().display().to_string(),
                    text.as_str(),
                );
                let report = Report::error()
                    .with_code(diagnostic.code.clone())
                    .with_message(format!(
                        "missing module {}::{}",
                        diagnostic.requested_package,
                        diagnostic.requested_path.as_path().display()
                    ))
                    .with_labels(vec![Label::primary(
                        file_id,
                        diagnostic.edge.range.start as usize..diagnostic.edge.range.end as usize,
                    )]);
                let config = term::Config::default();
                let stream = StandardStream::stderr(ColorChoice::Auto);
                let mut stream_lock = stream.lock();
                let _ = term::emit(&mut stream_lock, &config, &files, &report);
            }
            wosy_project::GraphDiagnostic::Cycle(diagnostic) => {
                eprintln!(
                    "{}: {}:{}..{} is part of an import cycle",
                    diagnostic.code,
                    diagnostic.edge.source.path.as_path().display(),
                    diagnostic.edge.range.start,
                    diagnostic.edge.range.end
                );
                for source in &diagnostic.cycle {
                    eprintln!(
                        "{}: {}:{}..{} participates in the cycle",
                        diagnostic.code,
                        source.source.path.as_path().display(),
                        source.range.start,
                        source.range.end
                    );
                }
            }
        }
    }
}

fn render_project_diagnostics(
    diagnostics: &[wosy_compiler::Diagnostic],
    texts: &BTreeMap<ProjectSourceIdentity, String>,
) {
    for diagnostic in diagnostics {
        let mut files = SimpleFiles::new();
        let labels = diagnostic
            .labels
            .iter()
            .filter_map(|label| {
                let source = ProjectSourceIdentity::new(
                    label.span.source.project.clone(),
                    label.span.source.package.clone(),
                    PackageRelativePath::new(label.span.source.path.clone().into()).ok()?,
                );
                let text = texts.get(&source)?;
                let file_id = files.add(&label.span.source.path, text.as_str());
                let range = label.span.range.start as usize..label.span.range.end as usize;
                Some(
                    match label.kind {
                        wosy_compiler::DiagnosticLabelKind::Primary => {
                            Label::primary(file_id, range)
                        }
                        wosy_compiler::DiagnosticLabelKind::Secondary => {
                            Label::secondary(file_id, range)
                        }
                    }
                    .with_message(label.message.clone()),
                )
            })
            .collect();
        let report = Report::error()
            .with_code(diagnostic.code.clone())
            .with_message(diagnostic.message.clone())
            .with_labels(labels)
            .with_notes(diagnostic.notes.clone());
        let config = term::Config::default();
        let stream = StandardStream::stderr(ColorChoice::Auto);
        let mut stream_lock = stream.lock();
        let _ = term::emit(&mut stream_lock, &config, &files, &report);
    }
}

fn artifact_identity(
    configuration_identity: String,
    configuration: &ProjectConfiguration,
    target: &str,
    profile: &str,
    artifact: &wosy_project::ArtifactConfig,
    artifact_profile: &wosy_project::ArtifactProfile,
    source_identity: &str,
    llvm: &str,
) -> Result<ArtifactIdentity, String> {
    let builder = configuration
        .builders
        .get(&artifact_profile.builder)
        .ok_or_else(|| format!("builder {} is missing", artifact_profile.builder))?;
    let runner = configuration
        .runners
        .get(&artifact_profile.run_runner)
        .ok_or_else(|| format!("runner {} is missing", artifact_profile.run_runner))?;
    Ok(ArtifactIdentity {
        source_identity: source_identity.to_owned(),
        configuration_identity,
        artifact_identity: format!("{target}:{profile}:{}", artifact.artifact),
        backend: artifact_profile.backend.clone(),
        output: artifact_profile.output.clone(),
        builder_identity: serde_json::to_string(builder).map_err(|error| error.to_string())?,
        runner_identity: serde_json::to_string(runner).map_err(|error| error.to_string())?,
        llvm_input_identity: blake3::hash(llvm.as_bytes()).to_hex().to_string(),
        compiler_identity: compiler_identity(),
        llvm_identity: llvm_identity(),
        builder_toolchain_identity: builder.toolchain_identity.clone(),
    })
}

fn compiler_identity() -> String {
    wosy_compiler::COMPILER_IDENTITY.to_owned()
}

fn llvm_identity() -> String {
    "inkwell-0.10.0+llvm-sys-181.3.0".to_owned()
}

fn invoke_direct(
    root: &std::path::Path,
    command: &[String],
    configured_args: &[String],
    args: &[&str],
) -> Result<(), String> {
    let executable = command
        .first()
        .ok_or_else(|| "configured command is empty".to_owned())?;
    let mut process = std::process::Command::new(executable);
    process
        .current_dir(root)
        .args(command.iter().skip(1))
        .args(configured_args)
        .args(args);
    let status = process
        .status()
        .map_err(|error| format!("failed to invoke {executable}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{executable} exited with {status}"))
    }
}

fn invoke_direct_with_arguments(
    root: &std::path::Path,
    command: &[String],
    configured_args: &[String],
    manifest: &std::path::Path,
    arguments: &[String],
) -> Result<u8, String> {
    let executable = command
        .first()
        .ok_or_else(|| "configured command is empty".to_owned())?;
    let status = std::process::Command::new(executable)
        .current_dir(root)
        .args(command.iter().skip(1))
        .args(configured_args)
        .arg(manifest)
        .arg("--")
        .args(arguments)
        .status()
        .map_err(|error| format!("failed to invoke {executable}: {error}"))?;
    status
        .code()
        .map(|code| code as u8)
        .ok_or_else(|| "runner was terminated".to_owned())
}

fn render_diagnostics(
    source_text: &str,
    diagnostics: &[wosy_compiler::Diagnostic],
    source: &SourceIdentity,
) {
    if diagnostics.is_empty() {
        return;
    }
    let mut files = SimpleFiles::new();
    let file_id = files.add(&source.path, source_text);
    let config = term::Config::default();
    let stream = StandardStream::stderr(ColorChoice::Auto);
    let mut stream_lock = stream.lock();
    for diagnostic in diagnostics {
        let labels = diagnostic
            .labels
            .iter()
            .map(|label| {
                Label::primary(
                    file_id,
                    label.span.range.start as usize..label.span.range.end as usize,
                )
                .with_message(label.message.clone())
            })
            .collect();
        let report = Report::error()
            .with_code(diagnostic.code.clone())
            .with_message(diagnostic.message.clone())
            .with_labels(labels)
            .with_notes(diagnostic.notes.clone());
        let _ = term::emit(&mut stream_lock, &config, &files, &report);
    }
}

#[cfg(test)]
mod routing_observations {
    use super::*;
    use std::cell::Cell;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(super) struct Counts {
        pub(super) extractions: usize,
        pub(super) preparations: usize,
        pub(super) builds: usize,
        pub(super) checks: usize,
        pub(super) single_module_emissions: usize,
        pub(super) project_emissions: usize,
    }

    thread_local! {
        static COUNTS: Cell<Option<Counts>> = const { Cell::new(None) };
    }

    pub(super) fn record(update: impl FnOnce(&mut Counts)) {
        COUNTS.with(|slot| {
            if let Some(mut counts) = slot.get() {
                update(&mut counts);
                slot.set(Some(counts));
            }
        });
    }

    pub(super) fn observe<T>(run: impl FnOnce() -> T) -> (T, Counts) {
        struct Restore(Option<Counts>);

        impl Drop for Restore {
            fn drop(&mut self) {
                COUNTS.set(self.0);
            }
        }

        let _restore = Restore(COUNTS.replace(Some(Counts::default())));
        let result = run();
        (result, COUNTS.get().expect("active routing observations"))
    }

    pub(super) fn observe_compile(
        project: &ProjectContext,
        configuration: &ProjectConfiguration,
    ) -> (Result<(String, Vec<SourceObservation>), String>, Counts) {
        observe(|| compile_project(&project.root, project, configuration, ScalarTargetLayout::NATIVE64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_test_path(name: &str) -> std::path::PathBuf {
        let current_thread = std::thread::current();
        let thread_name = current_thread.name().expect("named test thread");
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
            "wosy-cli-canonical-cst-{}-{}-{name}.json",
            std::process::id(),
            safe_thread_name
        ))
    }

    fn routing_test_project(
        name: &str,
        sources: &[(&str, &str)],
    ) -> (ProjectContext, ProjectConfiguration) {
        let root = snapshot_test_path(name).with_extension("");
        fs::create_dir_all(&root).expect("create routing test project");
        for (path, text) in sources {
            fs::write(root.join(path), text).expect("write routing test source");
        }
        let project = ProjectContext {
            root,
            package: "app".to_owned(),
            source_root: "main.w".into(),
            comment_categories: Vec::new(),
        };
        let configuration = ProjectConfiguration {
            default_target: "app".to_owned(),
            builders: BTreeMap::new(),
            runners: BTreeMap::new(),
            targets: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            resolved_packages: BTreeMap::new(),
        };
        (project, configuration)
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_proof_adapter_loads_explicit_reachable_module_once_without_outputs() {
        use wosy_compiler::ScalarModuleProofOutcome;
        let sources = [
            ("main.w", "%%start\nlib = namespace app \"lib.w\"; u8 value = lib.project(7);\n%%end"),
            ("lib.w", r#"%%start
                u8(u8) project = fn(input) { input };
                proof established for project(input) -> (out) ensures (out <= 7) {}
                proof contradicted for project(input) -> (out) ensures (out <= 6) {}
            %%end"#),
        ];
        let (project, configuration) = routing_test_project("internal-proof-graph", &sources);
        let (result, counts) = routing_observations::observe(|| {
            check_loaded_proofs(
                &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
                PackageRelativePath::new("lib.w".into()).unwrap(),
                &["contradicted", "established", "contradicted"],
            )
        });
        let (outcomes, texts) = result.expect("canonical proof inputs");
        assert_eq!(outcomes.unwrap(), [
            ScalarModuleProofOutcome::Refuted,
            ScalarModuleProofOutcome::Proved,
            ScalarModuleProofOutcome::Refuted,
        ]);
        assert_eq!(texts.len(), sources.len());
        assert!(texts.values().any(|value| value == sources[1].1));
        assert_eq!(counts, routing_observations::Counts { extractions: 2, checks: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_proof_adapter_batches_multiple_exact_sources_in_request_order() {
        use wosy_compiler::ScalarModuleProofOutcome;
        let sources = [
            ("main.w", r#"%%start
                first = namespace app "first.w";
                second = namespace app "second.w";
                ignored = namespace app "ignored.w";
                u8 a = first.project(7);
                u8 b = second.project(12);
            %%end"#),
            ("first.w", r#"%%start
                u8(u8) project = fn(input) { input };
                proof same for project(input) -> (out) ensures (out <= 7) {}
            %%end"#),
            ("second.w", r#"%%start
                u8(u8) project = fn(input) { input };
                proof same for project(input) -> (out) ensures (out <= 10) {}
            %%end"#),
            ("ignored.w", r#"%%start
                u8(u8) project = fn(input) { input };
                proof inert for project(input) -> (out) ensures (out <= 255) { core.show(out == 1__0); }
            %%end"#),
        ];
        let (project, configuration) = routing_test_project("internal-proof-multi-sources", &sources);
        let first = PackageRelativePath::new("first.w".into()).unwrap();
        let second = PackageRelativePath::new("second.w".into()).unwrap();
        let (result, counts) = routing_observations::observe(|| check_loaded_proof_selections(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
            &[(&second, "same"), (&first, "same"), (&second, "same")],
        ));
        let (outcomes, texts) = result.expect("all selected proof sources are in the graph");
        assert_eq!(outcomes.unwrap(), [
            ScalarModuleProofOutcome::Refuted, ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Refuted,
        ]);
        assert_eq!(texts.len(), sources.len());
        assert_eq!(counts, routing_observations::Counts { extractions: 4, checks: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());
        let missing_one = PackageRelativePath::new("absent-z.w".into()).unwrap();
        let missing_two = PackageRelativePath::new("absent-a.w".into()).unwrap();
        let missing = check_loaded_proof_selections(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
            &[(&second, "same"), (&missing_one, "same"), (&missing_two, "same")],
        );
        assert!(matches!(missing, Err(CanonicalLoadFailure::Input(error)) if error.contains("absent-z.w")));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_proof_adapter_verifies_two_local_citations_without_expanding_source_scope() {
        use wosy_compiler::ScalarModuleProofOutcome::{Proved, Unsupported};
        let sources = [
            ("main.w", r#"%%start
                lib = namespace app "lib.w";
                other = namespace app "other.w";
            %%end"#),
            ("lib.w", r#"%%start
                i8(i8) echo = fn(value) { value };
                proof nonnegative for echo(entry) -> (out) where (entry >= 0 && entry != 0) ensures (out >= 0) {}
                proof nonzero for echo(entry) -> (out) where (entry >= 0 && entry != 0) ensures (out != 0) {}
                proof joint for echo(entry) -> (out) where (entry >= 0 && entry != 0) ensures (out > 0) {
                    core.use(nonnegative(entry));
                    core.use(nonzero(entry));
                }
                proof wrong_source for echo(entry) -> (out) where (entry >= 0 && entry != 0) ensures (out > 0) {
                    core.use(nonnegative(entry));
                    core.use(foreign(entry));
                }
            %%end"#),
            ("other.w", r#"%%start
                i8(i8) echo = fn(value) { value };
                proof foreign for echo(entry) -> (out) where (entry >= 0 && entry != 0) ensures (out != 0) {}
            %%end"#),
            ("unreachable.w", "this file is not in the target graph"),
        ];
        let (project, configuration) = routing_test_project("internal-proof-two-local-citations", &sources);
        let lib = PackageRelativePath::new("lib.w".into()).unwrap();
        let outside = PackageRelativePath::new("unreachable.w".into()).unwrap();
        let check = |selections: &[(&PackageRelativePath, &str)]| check_loaded_proof_selections(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64, selections,
        );
        let (result, counts) = routing_observations::observe(|| check(
            &[(&lib, "joint"), (&lib, "wrong_source"), (&lib, "joint")],
        ));
        let (outcomes, texts) = result.expect("canonical selected proof input");
        assert_eq!(outcomes.unwrap(), [Proved, Unsupported, Proved]);
        assert_eq!(texts.len(), 3);
        assert!(texts.values().any(|text| text == sources[2].1));
        assert!(!texts.values().any(|text| text == sources[3].1));
        assert_eq!(counts, routing_observations::Counts { extractions: 3, checks: 1, ..Default::default() });
        let (result, counts) = routing_observations::observe(|| check(&[(&lib, "joint"), (&outside, "foreign")]));
        assert!(matches!(result, Err(CanonicalLoadFailure::Input(error)) if error.contains("unreachable.w")));
        assert_eq!(counts, routing_observations::Counts { extractions: 3, ..Default::default() });
        let mut remaining = fs::read_dir(&project.root).unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        remaining.sort_unstable();
        assert_eq!(remaining, ["lib.w", "main.w", "other.w", "unreachable.w"]);
        for (name, text) in sources {
            assert_eq!(fs::read_to_string(project.root.join(name)).unwrap(), text);
        }
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_proof_adapter_reuses_only_the_selected_producer_and_caller_from_one_graph() {
        use wosy_compiler::ScalarProofCallOutcome;
        let sources = [
            ("main.w", r#"%%start
                lib = namespace app "lib.w";
                u8(u8) alternate = fn(input) { input };
                unit() consumer = fn {
                    u8 result = lib.producer(7);
                    core.require(result <= 10, { .stdout = ""; .stderr = "bad"; .code = 23; .files = []; });
                };
                unit() wrong = fn {
                    u8 result = alternate(7);
                    core.require(result <= 10, { .stdout = ""; .stderr = "bad"; .code = 23; .files = []; });
                };
            %%end"#),
            ("lib.w", r#"%%start
                u8(u8) producer = fn(input) { input };
                proof cap for producer(input) -> (out) where (input > 4 && input <= 10) ensures (out <= 10) {}
            %%end"#),
            ("unreachable.w", "this file is not in the target graph"),
        ];
        let (project, configuration) = routing_test_project("internal-proof-one-caller", &sources);
        let main = PackageRelativePath::new("main.w".into()).unwrap();
        let lib = PackageRelativePath::new("lib.w".into()).unwrap();
        let unreachable = PackageRelativePath::new("unreachable.w".into()).unwrap();
        let check = |path, name| check_loaded_proof_call(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
            (&lib, "cap"), (path, name),
        );
        let (result, counts) = routing_observations::observe(|| check(&main, "consumer"));
        let (result, texts) = result.expect("canonical selected proof and caller");
        assert_eq!(result, ScalarProofCallOutcome::Established);
        assert_eq!(texts.len(), 2);
        assert_eq!(counts, routing_observations::Counts { extractions: 2, checks: 1, ..Default::default() });
        assert_eq!(check(&main, "wrong").unwrap().0, ScalarProofCallOutcome::Unavailable);
        assert!(matches!(
            check(&unreachable, "consumer"),
            Err(CanonicalLoadFailure::Input(error)) if error.contains("caller source unreachable.w"),
        ));
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_proof_adapter_refuses_unselected_sources_and_build_erases_named_proofs() {
        let vertex = PackageRelativePath::new("main.w".into()).unwrap();
        let (project, configuration) = routing_test_project("internal-proof-outside-graph", &[
            ("main.w", "%%start\nu8(u8) identity = fn(input) { input };\n%%end"),
            ("stray.w", "this source is not in the target graph"),
        ]);
        let (result, counts) = routing_observations::observe(|| check_loaded_proofs(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
            PackageRelativePath::new("stray.w".into()).unwrap(), &["unselected"],
        ));
        assert!(matches!(result, Err(CanonicalLoadFailure::Input(_))));
        assert_eq!(counts, routing_observations::Counts { extractions: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());

        let text = "%%start\nu8(u8) identity = fn(input) { input };\nproof alpha for identity(input) -> (out) ensures (out <= 10) {}\n%%end";
        let (project, configuration) = routing_test_project("ordinary-build-erases-proof", &[("main.w", text)]);
        let selected = check_loaded_proofs(&project.root, &project, &configuration, ScalarTargetLayout::NATIVE64, vertex, &["alpha"]);
        let (outcomes, _) = selected.expect("valid explicit proof source");
        assert_eq!(outcomes.unwrap(), [wosy_compiler::ScalarModuleProofOutcome::Refuted]);
        assert!(!project.root.join(".wosy").exists());
        let (built, counts) = routing_observations::observe(|| {
            compile_project(&project.root, &project, &configuration, ScalarTargetLayout::NATIVE64)
        });
        let (llvm, observations) = built.expect("ordinary Build ignores the proof");
        assert_eq!(observations.len(), 1);
        assert!(!llvm.contains("alpha"));
        assert_eq!(counts, routing_observations::Counts {
            extractions: 1, preparations: 1, builds: 1, single_module_emissions: 1, ..Default::default()
        });
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_ordered_effects_visit_only_authored_bytes_and_deferred_files_in_order() {
        use wosy_compiler::{ScalarCheckerScope, ScalarOrderedCheckerOutcome};
        let text = r#"%%start
            core.require(false, { .stdout = "ação\n"; .stderr = "first"; .code = 0;
                .files = [{ .path = "logs/a.txt"; .text = "☕"; }, { .path = "empty.txt"; .text = ""; }]; });
            core.require(false, { .stdout = ""; .stderr = "last\t"; .code = 0;
                .files = [{ .path = "logs/b.txt"; .text = "β"; }]; });
        %%end"#;
        let (project, configuration) = routing_test_project("ordered-internal-effects", &[("main.w", text)]);
        let receipt = check_loaded_ordered_project(
            &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
            ScalarCheckerScope::CurrentPackage, Vec::new(),
        ).expect("complete ordered receipt");
        let ScalarOrderedCheckerOutcome::Failed(reports) = &receipt.outcome else { panic!("expected authored effects"); };
        assert_eq!(receipt.selected_status(), Some(InternalCheckStatus::Zero));
        let mut events = Vec::new();
        receipt.visit_authored_effects(|effect| {
            match effect {
                InternalCheckEffect::Stdout(bytes) => events.push(("stdout", None, bytes.to_vec())),
                InternalCheckEffect::Stderr(bytes) => events.push(("stderr", None, bytes.to_vec())),
                InternalCheckEffect::DeferredFile(file) => events.push(("file", Some(file.path.to_string()), file.text.to_vec())),
            }
            Ok::<(), ()>(())
        }).unwrap();
        assert_eq!(events, [
            ("stdout", None, "ação\n".as_bytes().to_vec()),
            ("stderr", None, b"first".to_vec()),
            ("file", Some("logs/a.txt".into()), "☕".as_bytes().to_vec()),
            ("file", Some("empty.txt".into()), Vec::new()),
            ("stderr", None, b"last\t".to_vec()),
            ("file", Some("logs/b.txt".into()), "β".as_bytes().to_vec()),
        ]);
        let mut calls = 0;
        let refused = receipt.visit_authored_effects(|effect| {
            calls += 1;
            if let InternalCheckEffect::DeferredFile(file) = effect {
                assert!(std::ptr::eq(file, &reports[0].files()[0]));
                return Err("sink refused");
            }
            Ok(())
        });
        assert_eq!(refused, Err("sink refused"));
        assert_eq!(calls, 3);
        assert!(!project.root.join("logs").exists());
        assert!(!project.root.join("empty.txt").exists());
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_ordered_effects_share_payload_by_origin_without_merging_reports() {
        use wosy_compiler::{ScalarCheckerScope, ScalarOrderedCheckerOutcome};
        let output = r#"{ .stdout = "same out"; .stderr = "same err"; .code = 23;
            .files = [{ .path = "report.txt"; .text = "same bytes"; }]; }"#;
        let text = format!(r#"%%start
            unit(u8) leaf = fn(value) {{ core.require(value < 2, {output}); }};
            leaf(3);
            core.require(false, {output});
            leaf(4);
        %%end"#);
        let (project, configuration) = routing_test_project("ordered-effects-shared-origin", &[("main.w", &text)]);
        let receipt = check_loaded_ordered_project(
            &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
            ScalarCheckerScope::MainFlowOnly, Vec::new(),
        ).expect("completed ordered receipt");
        let ScalarOrderedCheckerOutcome::Failed(reports) = &receipt.outcome else { panic!("expected reports"); };
        let [first, separate, repeated] = reports.as_slice() else { panic!("three authored reports"); };
        assert_eq!(first.directive, repeated.directive);
        assert_ne!(first.directive, separate.directive);
        assert_eq!(first.stdout(), separate.stdout());
        assert_eq!(first.stderr(), separate.stderr());
        assert_eq!(first.files()[0].text, separate.files()[0].text);
        assert!(std::ptr::eq(first.stdout(), repeated.stdout()));
        assert!(std::ptr::eq(first.stderr(), repeated.stderr()));
        assert!(std::ptr::eq(&first.files()[0], &repeated.files()[0]));
        assert!(std::ptr::eq(first.code(), repeated.code()));
        assert!(!std::ptr::eq(first.stdout(), separate.stdout()));
        assert!(!std::ptr::eq(&first.files()[0], &separate.files()[0]));
        assert_eq!(first.witness.intervals_at(0, 0).unwrap().next(), Some((&3.into(), &3.into())));
        assert_eq!(repeated.witness.intervals_at(0, 0).unwrap().next(), Some((&4.into(), &4.into())));
        let Some(InternalCheckStatus::FirstNonzero(selected)) = receipt.selected_status() else {
            panic!("expected first authored nonzero");
        };
        assert!(std::ptr::eq(first, selected));
        let mut visited = Vec::new();
        receipt.visit_authored_effects(|effect| {
            match effect {
                InternalCheckEffect::Stdout(bytes) => {
                    assert_eq!(bytes, b"same out");
                    visited.push(("stdout", bytes.as_ptr()));
                }
                InternalCheckEffect::Stderr(bytes) => {
                    assert_eq!(bytes, b"same err");
                    visited.push(("stderr", bytes.as_ptr()));
                }
                InternalCheckEffect::DeferredFile(file) => {
                    assert_eq!(&*file.text, b"same bytes");
                    visited.push(("file", file.text.as_ptr()));
                }
            }
            Ok::<(), ()>(())
        }).unwrap();
        assert_eq!(visited, [
            ("stdout", first.stdout().as_ptr()), ("stderr", first.stderr().as_ptr()), ("file", first.files()[0].text.as_ptr()),
            ("stdout", separate.stdout().as_ptr()), ("stderr", separate.stderr().as_ptr()), ("file", separate.files()[0].text.as_ptr()),
            ("stdout", repeated.stdout().as_ptr()), ("stderr", repeated.stderr().as_ptr()), ("file", repeated.files()[0].text.as_ptr()),
        ]);
        assert!(!project.root.join("report.txt").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_ordered_effects_do_not_emit_for_invalid_unsupported_or_no_failure() {
        use wosy_compiler::{ScalarCheckerScope, ScalarOrderedCheckerOutcome};
        let authored = r#"core.require(false, { .stdout = "never"; .stderr = ""; .code = 0; .files = []; });"#;
        for (tail, expected) in [
            ("unit() leaf = fn { leaf(); }; unit() nested = fn { leaf(); }; nested();", "unsupported"),
            ("core.require(true, {});", "invalid"),
            ("", "failed"),
        ] {
            let text = format!("%%start\n{authored}\n{tail}\n%%end");
            let (project, configuration) = routing_test_project("ordered-internal-effect-completion", &[("main.w", &text)]);
            let receipt = check_loaded_ordered_project(
                &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
                ScalarCheckerScope::MainFlowOnly, Vec::new(),
            ).expect("well-formed canonical source");
            match &receipt.outcome {
                ScalarOrderedCheckerOutcome::Unsupported => assert_eq!(expected, "unsupported"),
                ScalarOrderedCheckerOutcome::Invalid(_) => assert_eq!(expected, "invalid"),
                ScalarOrderedCheckerOutcome::Failed(_) => assert_eq!(expected, "failed"),
                outcome => panic!("unexpected {outcome:?}"),
            }
            let mut effects = 0;
            receipt.visit_authored_effects(|_| { effects += 1; Ok::<(), ()>(()) }).unwrap();
            assert_eq!(effects, usize::from(expected == "failed"));
        }
        for outcome in [ScalarOrderedCheckerOutcome::Passed, ScalarOrderedCheckerOutcome::NoChecks] {
            let receipt = InternalCheckReceipt { outcome, texts: BTreeMap::new() };
            receipt.visit_authored_effects::<()>(|_| panic!("no authored effects")).unwrap();
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_ordered_dependency_uses_its_own_declared_target_and_layout_once() {
        use wosy_compiler::{ScalarCheckerScope, ScalarOrderedCheckerOutcome};
        let (scratch, _) = routing_test_project("ordered-own-dependency", &[]);
        let root = &scratch.root;
        let dep = root.join("dep");
        fs::create_dir_all(dep.join("src")).unwrap();
        let manifest = |package, source_root, target, path, profile, output| format!(r#"
[package]
name = "{package}"
default_target = "{target}"
source_root = "{source_root}"

[comments]
categories = []

[builders]

[runners]

[targets.{target}]
root = "{path}"

[targets.{target}.artifacts.main]
artifact = "RuntimeStdio"

[targets.{target}.artifacts.main.profiles.{profile}]
backend = "llvm"
output = "{output}"
builder = "never"
run_runner = "never"

[targets.{target}.profiles.{profile}]
main_artifact = "main"
"#);
        let parent_manifest = manifest("app", ".", "main", "main.w", "dev", "native");
        fs::write(root.join("wosy.toml"), format!("{parent_manifest}
[dependencies.dep]
path = \"dep\"
")).unwrap();
        fs::write(dep.join("wosy.toml"), manifest("dep", "src", "own", "src/own.w", "checked", "wasm-wasip1")).unwrap();
        let emit = |text, code| format!(
            "core.require(false, {{ .stdout = \"{text}\"; .stderr = \"\"; .code = {code}; .files = []; }});",
        );
        fs::write(root.join("main.w"), format!("%%start
vendor = namespace dep \"borrowed.w\";
{}
%%end", emit("parent", 3))).unwrap();
        fs::write(dep.join("src/borrowed.w"), format!("%%start
{}
%%end", emit("parent import", 5))).unwrap();
        fs::write(dep.join("src/own.w"), format!(
            "%%start
{}
unit(u8) unused = fn(value) {{ core.require(value < 1, {{ .stdout = \"unused dep\"; .stderr = \"\"; .code = 9; .files = []; }}); }};
%%end",
            emit("own dep", 7),
        )).unwrap();

        let (parent, document, selection) = ProjectConfiguration::load_with_direct_dependency_check_target(root, "dep", "own", "checked").unwrap();
        let parent_project = ProjectContext::from_manifest(root, "main", document.as_table()).unwrap();
        let parent_layout = parent.artifact_profile("main", "dev").unwrap().2.target_layout().unwrap();
        assert_eq!(parent_layout.pointer_size, 8);
        assert_eq!(selection.layout.pointer_size, 4);
        assert_eq!(selection.project.root, dep);
        assert_eq!(selection.project.source_root, std::path::PathBuf::from("src/own.w"));
        let (child, counts) = routing_observations::observe(|| check_loaded_ordered_dependency(&selection));
        assert_eq!(counts.checks, 1);
        assert_eq!(counts.extractions, 1);
        assert_eq!(counts.builds, 0);
        let child = child.unwrap();
        let ScalarOrderedCheckerOutcome::Failed(failures) = child.outcome else { panic!("own target and unused root failures"); };
        assert_eq!(failures.iter().map(|failure| failure.stdout()).collect::<Vec<_>>(), [b"own dep".as_slice(), b"unused dep".as_slice()]);
        assert_eq!(child.texts.len(), 1);
        let parent = check_loaded_ordered_project(root, &parent_project, &parent, scalar_target_layout(parent_layout), ScalarCheckerScope::CurrentPackage, Vec::new()).unwrap();
        let ScalarOrderedCheckerOutcome::Failed(failures) = parent.outcome else { panic!("parent and imported startup failures"); };
        assert_eq!(failures.iter().map(|failure| failure.stdout()).collect::<Vec<_>>(), [b"parent import".as_slice(), b"parent".as_slice()]);
        assert_eq!(parent.texts.len(), 2);
        assert_eq!(
            ProjectConfiguration::load_with_direct_dependency_check_target(root, "dep", "own", "dev").err().as_deref(),
            Some("target own profile dev is missing"),
        );
        assert!(!root.join(".wosy").exists() && !dep.join(".wosy").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_ordered_checker_selects_exact_typed_status_and_keeps_every_report() {
        use wosy_compiler::{ScalarCheckerScope, ScalarOrderedCheckerOutcome};
        for (second, third) in [
            ("23", "9"),
            ("0", "0"),
            ("340282366920938463463374607431768211456", "9"),
        ] {
            let main = r#"%%start
                core.require(false, { .stdout = "primeiro"; .stderr = ""; .code = 0;
                    .files = [{ .path = "report.txt"; .text = "α"; }]; });
                lib = namespace app "lib.w";
            %%end"#;
            let library = format!(r#"%%start
                core.require(false, {{ .stdout = ""; .stderr = "second☕"; .code = {second}; .files = []; }});
            %%end"#);
            let dormant = format!(r#"%%start
                core.require(false, {{ .stdout = "must not initialize"; .stderr = ""; .code = 7; .files = []; }});
                unit(u8) unused = fn(value) {{
                    core.require(value < 3, {{ .stdout = "third"; .stderr = ""; .code = {third}; .files = []; }});
                }};
            %%end"#);
            let (project, configuration) = routing_test_project("ordered-internal-checker-receipt", &[
                ("main.w", main), ("lib.w", &library), ("extra.w", "invalid on-disk source"),
            ]);
            let observed = |scope| routing_observations::observe(|| check_loaded_ordered_project(
                &project.root, &project, &configuration, ScalarTargetLayout::WASM32, scope,
                vec![received_source(&project.root, "extra.w", &dormant)],
            ));
            let (receipt, counts) = observed(ScalarCheckerScope::CurrentPackage);
            let receipt = receipt.expect("ordered canonical receipt");
            let ScalarOrderedCheckerOutcome::Failed(reports) = &receipt.outcome else { panic!("expected authored reports"); };
            let [first, next, last] = reports.as_slice() else { panic!("expected all three reports"); };
            assert_eq!(reports.iter().map(|report| report.code().to_string()).collect::<Vec<_>>(), ["0", second, third]);
            assert_eq!((first.source.path.as_str(), next.source.path.as_str(), last.source.path.as_str()),
                ("main.w", "lib.w", "extra.w"));
            assert_eq!(first.stdout(), b"primeiro");
            assert_eq!(next.stderr(), "second☕".as_bytes());
            assert_eq!(last.stdout(), b"third");
            assert_eq!(&*first.files()[0].text, "α".as_bytes());
            if second == "0" {
                assert_eq!(receipt.selected_status(), Some(InternalCheckStatus::Zero));
            } else {
                let Some(InternalCheckStatus::FirstNonzero(selected)) = receipt.selected_status() else {
                    panic!("expected exact first nonzero candidate");
                };
                assert!(std::ptr::eq(selected, next));
                assert_eq!(selected.code().to_string(), second);
            }
            assert_eq!(receipt.texts.len(), 3);
            assert!(receipt.texts.values().any(|text| text == &dormant));
            assert_eq!(counts, routing_observations::Counts { extractions: 3, checks: 1, ..Default::default() });

            let (main_only, counts) = observed(ScalarCheckerScope::MainFlowOnly);
            let main_only = main_only.expect("without dormant selection");
            assert!(matches!(&main_only.outcome, ScalarOrderedCheckerOutcome::Failed(reports) if reports.len() == 2));
            assert_eq!(main_only.texts.len(), 2);
            assert_eq!(counts, routing_observations::Counts { extractions: 2, checks: 1, ..Default::default() });
            assert!(!project.root.join(".wosy").exists());
            assert!(!project.root.join("report.txt").exists());
        }
        assert!(Arguments::try_parse_from(["wosy", "check"]).is_err());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_ordered_checker_does_not_map_incomplete_analysis_to_zero() {
        use wosy_compiler::{ScalarCheckerScope, ScalarOrderedCheckerOutcome};
        let text = r#"%%start
            unit() leaf = fn { leaf(); };
            unit() parent = fn { leaf(); };
            core.require(false, { .stdout = "not published"; .stderr = ""; .code = 0; .files = []; });
            parent();
        %%end"#;
        let (project, configuration) = routing_test_project("ordered-internal-unsupported", &[("main.w", text)]);
        let (receipt, counts) = routing_observations::observe(|| check_loaded_ordered_project(
            &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
            ScalarCheckerScope::MainFlowOnly, Vec::new(),
        ));
        let receipt = receipt.expect("well-formed canonical source");
        assert_eq!(receipt.outcome, ScalarOrderedCheckerOutcome::Unsupported);
        assert_eq!(receipt.selected_status(), None);
        assert_eq!(receipt.texts.len(), 1);
        assert_eq!(counts, routing_observations::Counts { extractions: 1, checks: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_checker_adapter_uses_canonical_graph_once_and_never_publishes() {
        use wosy_compiler::{ScalarCheckerOperation, ScalarCheckerOutcome};
        let sources = [
            ("main.w", "%%start\nlib = namespace app \"lib.w\"; lib.require_small(8);\n%%end"),
            ("lib.w", r#"%%start
                unit(u8) require_small = fn(value) {
                    core.require(value <= 7, { .stdout = "out"; .stderr = "bad"; .code = 17; .files = []; });
                };
            %%end"#),
        ];
        let (project, configuration) = routing_test_project("internal-checked-graph", &sources);
        let (result, counts) = routing_observations::observe(|| {
            check_loaded_project(
                &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
                wosy_compiler::ScalarCheckerScope::default(), Vec::new(),
            )
        });
        let InternalCheckReceipt { outcome, texts } = result.expect("canonical checker input");
        let ScalarCheckerOutcome::Failed(failure) = outcome else { panic!("expected authored dependency failure"); };
        assert_eq!(failure.operation, ScalarCheckerOperation::Require);
        assert_eq!(failure.source.path, "lib.w");
        assert_eq!(failure.source.revision, blake3::hash(sources[1].1.as_bytes()).to_string());
        assert_eq!(&*failure.stdout, b"out");
        assert_eq!(&*failure.stderr, b"bad");
        assert_eq!(failure.code, 17.into());
        let (binding, ty) = failure.witness.axes().next().expect("canonical call input with inclusive singleton");
        assert_eq!(failure.witness.axes().len(), 1);
        assert_eq!(binding.source, failure.source);
        let at = |span: wosy_syntax::ByteSpan| &sources[1].1[span.start as usize..span.end as usize];
        assert_eq!(at(binding.declaration_span), "value");
        assert_eq!(at(failure.condition), "value <= 7");
        assert_eq!(ty, &wosy_compiler::ScalarType::U8);
        assert_eq!(failure.witness.row_count(), 1);
        let mut singleton = failure.witness.intervals_at(0, 0).expect("existing axis and row");
        assert_eq!(singleton.next(), Some((&8.into(), &8.into())));
        assert_eq!(singleton.next(), None);
        assert!(failure.witness.intervals_at(1, 0).is_none());
        assert!(failure.witness.intervals_at(0, 1).is_none());
        assert_eq!(texts.len(), 2);
        assert!(texts.values().any(|text| text == sources[1].1));
        assert_eq!(counts, routing_observations::Counts { extractions: 2, checks: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_checker_adapter_retains_exact_signed_intervals_without_writing() {
        use wosy_compiler::{ScalarCheckerOutcome, ScalarCheckerScope, ScalarType};
        let (project, configuration) = routing_test_project("internal-signed-receipt", &[(
            "main.w", r#"%%start
                unit(i128) unused = fn(value) {
                    core.require(value == 0, { .stdout = "saída"; .stderr = "☕"; .code = 340282366920938463463374607431768211456; .files = [{ .path = "report.txt"; .text = "α"; }]; });
                };
            %%end"#,
        )]);
        let (result, counts) = routing_observations::observe(|| {
            check_loaded_project(
                &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
                ScalarCheckerScope::CurrentPackage, Vec::new(),
            )
        });
        let InternalCheckReceipt { outcome, texts } = result.expect("canonical symbolic receipt");
        let ScalarCheckerOutcome::Failed(failure) = outcome else { panic!("expected authored symbolic failure"); };
        let mut axes = failure.witness.axes();
        let (binding, ty) = axes.next().expect("canonical symbolic input");
        assert_eq!(binding.source, failure.source);
        assert_eq!(ty, &ScalarType::I128);
        assert_eq!(axes.next(), None);
        assert_eq!(failure.witness.row_count(), 1);
        let mut intervals = failure.witness.intervals_at(0, 0).expect("exact failing intervals");
        assert_eq!(intervals.next(), Some((&i128::MIN.into(), &(-1).into())));
        assert_eq!(intervals.next(), Some((&1.into(), &i128::MAX.into())));
        assert_eq!(intervals.next(), None);
        assert_eq!(failure.stdout.as_ref(), "saída".as_bytes());
        assert_eq!(failure.stderr.as_ref(), "☕".as_bytes());
        assert_eq!(failure.code.to_string(), "340282366920938463463374607431768211456");
        assert_eq!(failure.files.len(), 1);
        assert_eq!(texts.len(), 1);
        assert_eq!(counts, routing_observations::Counts { extractions: 1, checks: 1, ..Default::default() });
        assert!(!project.root.join("report.txt").exists());
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_checker_adapter_preserves_load_errors_without_build_receipts() {
        let samples = [
            ("checker-parse-rejection", "%%start\n???\n%%end"),
            ("checker-graph-rejection", "%%start\nmissing = namespace app \"missing.w\";\n%%end"),
        ];
        for (name, text) in samples {
            let (project, configuration) = routing_test_project(name, &[("main.w", text)]);
            let (result, counts) = routing_observations::observe(|| {
                check_loaded_project(
                &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
                wosy_compiler::ScalarCheckerScope::default(), Vec::new(),
            )
            });
            if name == "checker-parse-rejection" {
                let Err(CanonicalLoadFailure::Syntax { source, diagnostics, .. }) = result else { panic!("expected typed syntax error"); };
                assert_eq!(source.path, "main.w");
                assert_eq!(diagnostics[0].code, "S0001");
                assert_eq!(counts.extractions, 0);
            } else {
                let Err(CanonicalLoadFailure::Graph { error, texts }) = result else { panic!("expected typed graph error"); };
                assert_eq!(error.diagnostics.len(), 1);
                assert_eq!(format_graph_error(error), "M0001: missing module app::missing.w");
                assert_eq!(texts.len(), 1);
                assert_eq!(counts.extractions, 1);
            }
            assert_eq!(counts.checks, 0);
            assert_eq!(counts.builds, 0);
            assert!(!project.root.join(".wosy").exists());
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_checker_adapter_forwards_main_flow_scope_without_public_flag() {
        use wosy_compiler::{ScalarCheckerOutcome, ScalarCheckerScope};
        let (project, configuration) = routing_test_project("internal-main-flow", &[(
            "main.w", r#"%%start
                unit() unused = fn {
                    core.require(false, { .stdout = ""; .stderr = "unused"; .code = 4; .files = []; });
                };
                unit() main_flow = fn {};
                main_flow();
            %%end"#,
        )]);
        let observed = |scope| {
            let (result, counts) = routing_observations::observe(|| {
                check_loaded_project(
                    &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64, scope, Vec::new(),
                )
            });
            assert_eq!(counts, routing_observations::Counts { extractions: 1, checks: 1, ..Default::default() });
            result.expect("valid source").outcome
        };
        assert!(matches!(observed(ScalarCheckerScope::default()), ScalarCheckerOutcome::Failed(_)));
        assert_eq!(observed(ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::NoChecks);
        assert!(!project.root.join(".wosy").exists());
        assert!(Arguments::try_parse_from(["wosy", "check", "--main-flow-only"]).is_err());
    }

    #[cfg(feature = "internal-checker")]
    fn received_source(root: &std::path::Path, path: &str, text: &str) -> DormantSourceInput {
        DormantSourceInput {
            identity: ProjectSourceIdentity::new(
                root.to_string_lossy().into_owned(), "app".into(),
                PackageRelativePath::new(path.into()).expect("relative fixture"),
            ),
            text: text.to_owned(),
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_checker_adapter_prepares_explicit_dormant_receipts_once_without_reading_files() {
        use wosy_compiler::{ScalarCheckerOutcome, ScalarCheckerScope};
        let extra = r#"%%start
            core.require(false, { .stdout = "initializer must not run"; .stderr = ""; .code = 5; .files = []; });
            unit(u8) dormant_check = fn(value) {
                core.require(value < 3, { .stdout = "dormant witness"; .stderr = ""; .code = 7; .files = []; });
            };
        %%end"#;
        let (project, configuration) = routing_test_project("internal-dormant-graph", &[
            ("main.w", "%%start\nunit() main_flow = fn {}; main_flow();\n%%end"),
            ("extra.w", "invalid on-disk source, unused"),
            ("not-selected.w", "not valid source"),
        ]);
        let (result, counts) = routing_observations::observe(|| check_loaded_project(
            &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
            ScalarCheckerScope::CurrentPackage, vec![received_source(&project.root, "extra.w", extra)],
        ));
        let InternalCheckReceipt { outcome, texts } = result.expect("canonical dormant receipt");
        let ScalarCheckerOutcome::Failed(failure) = outcome else { panic!("expected unused function obligation"); };
        assert_eq!(failure.source.package, project.package);
        assert_eq!(failure.source.project, project.root.to_string_lossy());
        assert_eq!(failure.source.path, "extra.w");
        assert_eq!(failure.source.revision, blake3::hash(extra.as_bytes()).to_string());
        assert_eq!(failure.stdout.as_ref(), b"dormant witness");
        assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&3.into(), &255.into())));
        assert_eq!(texts.len(), 2);
        assert_eq!(counts, routing_observations::Counts { extractions: 2, checks: 1, ..Default::default() });

        let (result, counts) = routing_observations::observe(|| check_loaded_project(
            &project.root, &project, &configuration, ScalarTargetLayout::WASM32,
            ScalarCheckerScope::MainFlowOnly, vec![received_source(&project.root, "missing.w", "invalid syntax")],
        ));
        let InternalCheckReceipt { outcome, texts } = result.expect("no dormant parsing for this scope");
        assert_eq!(outcome, ScalarCheckerOutcome::NoChecks);
        assert_eq!(texts.len(), 1);
        assert_eq!(counts, routing_observations::Counts { extractions: 1, checks: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn internal_checker_adapter_rejects_dormant_identity_aliases_before_parsing_any_receipt() {
        use wosy_compiler::ScalarCheckerScope;
        let main = "%%start\nunit() main_flow = fn {};\n%%end";
        let (project, mut configuration) = routing_test_project("internal-dormant-aliases", &[("main.w", main)]);
        let dependency = project.root.join("vendor");
        configuration.resolved_packages.insert("vendor".into(), wosy_project::ResolvedPackage {
            name: "vendor".into(),
            project_root: dependency.clone(),
            source_root: dependency.clone(),
            identity: ProjectSourceIdentity::new(dependency.to_string_lossy().into_owned(), "vendor".into(), PackageRelativePath::new("src".into()).unwrap()),
        });
        for invalid in ["main.w", "./main.w", "duplicate.w", "deps/other.w", "vendor/other.w"] {
            let (result, counts) = routing_observations::observe(|| check_loaded_project(
                &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
                ScalarCheckerScope::CurrentPackage,
                vec![received_source(&project.root, "duplicate.w", "invalid syntax"), received_source(&project.root, invalid, "invalid syntax")],
            ));
            assert!(matches!(result, Err(CanonicalLoadFailure::Input(_))), "{invalid}: {result:?}");
            assert_eq!(counts, routing_observations::Counts { extractions: 1, ..Default::default() });
        }
        let mut excluded = received_source(&project.root, "foreign.w", "invalid syntax");
        excluded.identity.package = "foreign".into();
        let (result, counts) = routing_observations::observe(|| check_loaded_project(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
            ScalarCheckerScope::CurrentPackage, vec![excluded],
        ));
        assert!(matches!(result, Err(CanonicalLoadFailure::Input(_))));
        assert_eq!(counts, routing_observations::Counts { extractions: 1, ..Default::default() });
        assert!(!project.root.join(".wosy").exists());
    }

    #[test]
    fn static_boundary_routes_single_and_multi_module_build_once() {
        let single = [(
            "main.w",
            "%%start\ni32() answer = fn() {\n\t42\n};\n%%end\n",
        )];
        let multi = [
            (
                "main.w",
                "%%start\nmath = namespace app \"math.w\";\ni32 value = math.answer();\n%%end\n",
            ),
            ("math.w", single[0].1),
        ];
        for (name, sources, single_module_emissions, project_emissions) in [
            ("single", single.as_slice(), 1, 0),
            ("multi", multi.as_slice(), 0, 1),
        ] {
            let (project, configuration) = routing_test_project(name, sources);
            let (result, counts) = routing_observations::observe_compile(&project, &configuration);
            let (llvm, observations) = result.expect("compile routing test project");
            assert_eq!(
                counts,
                routing_observations::Counts {
                    extractions: sources.len(),
                    preparations: 1,
                    builds: 1,
                    checks: 0,
                    single_module_emissions,
                    project_emissions,
                },
                "{name}"
            );

            if sources.len() == 1 {
                assert!(llvm.contains("define i32 @answer()"), "{llvm}");
            } else {
                assert!(llvm.contains("define i32 @wosy_fn__"), "{llvm}");
                assert!(llvm.contains("call i32 @wosy_fn__"), "{llvm}");
            }
            assert!(llvm.contains("ret i32 42"), "{llvm}");

            let expected = sources
                .iter()
                .map(|&(path, text)| SourceObservation {
                    source: ProjectSourceIdentity::new(
                        project.root.to_string_lossy().into_owned(),
                        project.package.clone(),
                        PackageRelativePath::new(path.into()).expect("test source path"),
                    ),
                    content_revision: ContentRevision(blake3::hash(text.as_bytes()).to_string()),
                })
                .collect::<Vec<_>>();
            assert_eq!(observations, expected);
            for observation in &observations {
                let snapshot = project
                    .root
                    .join(".wosy/cst")
                    .join(format!("{}.json", observation.content_revision.0));
                assert!(
                    snapshot.is_file(),
                    "missing snapshot: {}",
                    snapshot.display()
                );
            }
        }
    }

    #[test]
    fn static_lookup_rejection_skips_build_and_emission() {
        let (project, configuration) = routing_test_project(
            "static-lookup-rejection",
            &[(
                "main.w",
                "%%start\nstruct Callback { i32(i32) run; }\ni32(i32) identity = fn(input) { input };\nCallback callback = { .run = identity; };\ni32(i32) invoke = fn(input) { callback.run(input) };\n%%end",
            )],
        );
        let (result, counts) = routing_observations::observe_compile(&project, &configuration);
        assert_eq!(
            result.expect_err("reject static call lookup"),
            "build rejected by semantic diagnostics"
        );
        assert_eq!(
            counts,
            routing_observations::Counts {
                extractions: 1,
                preparations: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn namespace_discovery_uses_only_source_declarations() {
        let (project, configuration) = routing_test_project(
            "missing-module",
            &[(
                "main.w",
                "%%start\nmissing = namespace app \"missing.w\";\n%%end\n",
            )],
        );
        let (result, counts) = routing_observations::observe_compile(&project, &configuration);
        assert_eq!(
            result.expect_err("reject missing namespace module"),
            "M0001: missing module app::missing.w"
        );
        assert_eq!(
            counts,
            routing_observations::Counts {
                extractions: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn loads_deep_archived_snapshot_and_reaches_scalar_derivation() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let output = parse_source(
            source,
            include_str!("../../../stdlib/src/bootstrap.w").into(),
            &[],
        );
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let root = snapshot_test_path("deep").with_extension("");
        let path = root.join(".wosy/cst/r1.json");
        let tree = output.result.canonical_cst();
        let borrowed = serde_json::to_vec_pretty(&tree.snapshot_json_serializable())
            .expect("serialize borrowed CST snapshot");
        let owned = serde_json::to_vec_pretty(&tree.snapshot())
            .expect("serialize owned CST snapshot");
        assert!(borrowed == owned, "borrowed CST snapshot changed bytes");
        persist_cst(&root, &output).expect("publish archived snapshot");
        assert!(fs::read(&path).unwrap() == borrowed, "streamed CST snapshot changed bytes");

        let canonical = load_canonical_cst(&path).expect("load deep canonical CST snapshot");
        let scalar = wosy_compiler::derive_scalar_program_from_cst(&canonical);

        assert!(!scalar.program.items.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn failed_snapshot_serialization_preserves_existing_publication() {
        struct PartialSnapshot;
        impl serde::Serialize for PartialSnapshot {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut fields = serializer.serialize_seq(Some(2))?;
                serde::ser::SerializeSeq::serialize_element(&mut fields, "partial snapshot")?;
                Err(serde::ser::Error::custom("snapshot rejected"))
            }
        }

        let root = snapshot_test_path("rejected").with_extension("");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("prior.json");
        fs::write(&path, b"prior snapshot").unwrap();
        assert_eq!(publish_cst_snapshot(&path, &PartialSnapshot).unwrap_err(), "snapshot rejected");
        assert_eq!(fs::read(&path).unwrap().as_slice(), b"prior snapshot");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn snapshot_publication_keeps_output_private_and_preserves_rejected_destinations() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = snapshot_test_path("permissions").with_extension("");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("snapshot.json");
        publish_cst_snapshot(&path, &"new").unwrap();
        let mode = |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path) & !0o600, 0);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        publish_cst_snapshot(&path, &"replacement").unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read(&path).unwrap().as_slice(), b"\"replacement\"");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(publish_cst_snapshot(&path, &"not published").unwrap_err(),
            "canonical CST snapshot destination is not a writable single-link file");
        assert_eq!(mode(&path), 0o400);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let alias = root.join("alias.json");
        fs::hard_link(&path, &alias).unwrap();
        assert_eq!(publish_cst_snapshot(&path, &"not published").unwrap_err(),
            "canonical CST snapshot destination is not a writable single-link file");
        assert_eq!(fs::read(&alias).unwrap().as_slice(), b"\"replacement\"");

        let link = root.join("link.json");
        symlink(&path, &link).unwrap();
        let directory = root.join("directory.json");
        fs::create_dir(&directory).unwrap();
        for destination in [&link, &directory] {
            assert_eq!(publish_cst_snapshot(destination, &"not published").unwrap_err(),
                "canonical CST snapshot destination is not a regular file");
        }
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert!(directory.is_dir());
        assert_eq!(fs::read(&path).unwrap().as_slice(), b"\"replacement\"");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_malformed_canonical_cst_snapshot() {
        let path = snapshot_test_path("malformed");
        fs::write(&path, b"{").expect("write malformed snapshot");

        assert!(load_canonical_cst(&path).is_err());
    }

    #[test]
    fn rejects_trailing_input_after_canonical_cst_snapshot() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let output = parse_source(source, "%%start\n%%end".into(), &[]);
        let mut bytes = serde_json::to_vec(&output.result.canonical_cst().snapshot())
            .expect("serialize canonical CST snapshot");
        bytes.extend_from_slice(b" trailing");
        let path = snapshot_test_path("trailing");
        fs::write(&path, bytes).expect("write snapshot with trailing input");

        assert!(load_canonical_cst(&path).is_err());
    }

    #[test]
    fn compiler_namespace_target_retains_dependency_identity() {
        let source = ProjectSourceIdentity::new(
            "/workspace/stdlib".to_owned(),
            "std".to_owned(),
            PackageRelativePath::new("src/math.w".into()).expect("module path"),
        );
        let target = compiler_source_identity(&source, &ContentRevision("rev".to_owned()));

        assert_eq!(target.project, "/workspace/stdlib");
        assert_eq!(target.package, "std");
        assert_eq!(target.path, "src/math.w");
        assert_eq!(target.revision, "rev");
    }

    #[test]
    fn dependency_observation_lookup_uses_source_project_root() {
        let observation = SourceObservation {
            source: ProjectSourceIdentity::new(
                "/workspace/stdlib".to_owned(),
                "std".to_owned(),
                PackageRelativePath::new("src/math.w".into()).expect("module path"),
            ),
            content_revision: ContentRevision("rev".to_owned()),
        };

        assert_eq!(
            source_observation_path(&observation),
            std::path::PathBuf::from("/workspace/stdlib/src/math.w")
        );
    }
}
