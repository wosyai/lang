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
use std::process::ExitCode;
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
}

#[derive(Clone, Copy)]
enum CanonicalLoadPurpose {
    Build,
    #[cfg(feature = "internal-checker")]
    Check,
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
        let revision = blake3::hash(text.as_bytes()).to_hex().to_string();
        let source = SourceIdentity::new(
            project_identity,
            package.clone(),
            path.as_path().to_string_lossy().replace('\\', "/"),
            revision.clone(),
        );
        let output = parse_source(source, text, &project.comment_categories);
        if !output.diagnostics.is_empty() {
            return Err(CanonicalLoadFailure::Syntax {
                source: output.result.source, text: output.result.text, diagnostics: output.diagnostics,
            });
        }
        let source = &output.result.source;
        if matches!(purpose, CanonicalLoadPurpose::Build) {
            persist_cst(root, &output)?;
        }
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
        let declarations = derive_scalar_declarations_from_cst(output.result.canonical_cst());
        let mut namespace_edges = Vec::new();
        for namespace in declarations.namespaces() {
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
            content_revision: ContentRevision(revision),
            namespace_edges,
        });
        declarations_by_path.insert((package, path), declarations);
        texts.insert(graph_source, output.result.text);
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
        derivation_diagnostics.extend_from_slice(declarations.diagnostics());
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
        project_declarations.push((declarations, namespace_bindings));
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
    })
}

#[cfg(feature = "internal-checker")]
type InternalCheckResult = Result<
    (wosy_compiler::ScalarCheckerOutcome, BTreeMap<ProjectSourceIdentity, String>),
    CanonicalLoadFailure,
>;

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    scope: wosy_compiler::ScalarCheckerScope,
) -> InternalCheckResult {
    let CanonicalProjectInputs { declarations, initialization_order, texts, .. } =
        load_canonical_project_inputs(root, project, configuration, CanonicalLoadPurpose::Check)?;
    #[cfg(test)]
    routing_observations::record(|counts| counts.checks += 1);
    let outcome = wosy_compiler::check_scalar_package(
        declarations, initialization_order, ScalarPreparationInputs { target_layout }, scope,
    );
    Ok((outcome, texts))
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
    let bytes = serde_json::to_vec_pretty(&output.result.canonical_cst().snapshot_json_serializable())
        .map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, bytes).map_err(|error| error.to_string())
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
                wosy_compiler::ScalarCheckerScope::default(),
            )
        });
        let (outcome, texts) = result.expect("canonical checker input");
        let ScalarCheckerOutcome::Failed(failure) = outcome else { panic!("expected authored dependency failure"); };
        assert_eq!(failure.operation, ScalarCheckerOperation::Require);
        assert_eq!(failure.source.path, "lib.w");
        assert_eq!(failure.source.revision, blake3::hash(sources[1].1.as_bytes()).to_string());
        assert_eq!(&*failure.stdout, b"out");
        assert_eq!(&*failure.stderr, b"bad");
        assert_eq!(failure.code, 17.into());
        assert_eq!(texts.len(), 2);
        assert!(texts.values().any(|text| text == sources[1].1));
        assert_eq!(counts, routing_observations::Counts { extractions: 2, checks: 1, ..Default::default() });
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
                wosy_compiler::ScalarCheckerScope::default(),
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
                    &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64, scope,
                )
            });
            assert_eq!(counts, routing_observations::Counts { extractions: 1, checks: 1, ..Default::default() });
            result.expect("valid source").0
        };
        assert!(matches!(observed(ScalarCheckerScope::default()), ScalarCheckerOutcome::Failed(_)));
        assert_eq!(observed(ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::NoChecks);
        assert!(!project.root.join(".wosy").exists());
        assert!(Arguments::try_parse_from(["wosy", "check", "--main-flow-only"]).is_err());
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
        let path = snapshot_test_path("deep");
        let tree = output.result.canonical_cst();
        let borrowed = serde_json::to_vec_pretty(&tree.snapshot_json_serializable())
            .expect("serialize borrowed CST snapshot");
        let owned = serde_json::to_vec_pretty(&tree.snapshot())
            .expect("serialize owned CST snapshot");
        assert!(borrowed == owned, "borrowed CST snapshot changed bytes");
        fs::write(&path, borrowed).expect("write archived snapshot");

        let canonical = load_canonical_cst(&path).expect("load deep canonical CST snapshot");
        let scalar = wosy_compiler::derive_scalar_program_from_cst(&canonical);

        assert!(!scalar.program.items.is_empty());
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
