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

mod checked_access_component;

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
        .map(current_source_observation)
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

fn current_source_observation(observation: &SourceObservation) -> Result<SourceObservation, String> {
    if let Some(component) = checked_access_component::refresh_observation(&observation.source)? {
        return Ok(component);
    }
    let path = source_observation_path(observation);
    let bytes = fs::read(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(SourceObservation {
        source: observation.source.clone(),
        content_revision: ContentRevision(blake3::hash(&bytes).to_hex().to_string()),
    })
}

fn compile_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
) -> Result<(String, Vec<SourceObservation>), String> {
    // Actual access-site selection is a separate lowering change. Merely
    // compiling a user's project must not pull in an unrelated module.
    compile_project_with_component(root, project, configuration, target_layout, None)
}

fn compile_project_with_component(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    component: Option<checked_access_component::CheckedAccessComponent>,
) -> Result<(String, Vec<SourceObservation>), String> {
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
        let source = &output.result.source;
        render_diagnostics(&output.result.text, &output.diagnostics, source);
        if !output.diagnostics.is_empty() {
            return Err("build rejected by syntax diagnostics".to_owned());
        }
        persist_cst(root, &output)?;
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
        declarations_by_path.insert(graph_source.clone(), declarations);
        texts.insert(graph_source, output.result.text);
    }
    drop((discovered, pending));

    let root_node = nodes
        .first()
        .cloned()
        .ok_or_else(|| "project root source is missing".to_owned())?;
    let mut graph = load_reachable_source_graph(
        &project.package,
        root_node,
        nodes,
        &configuration.resolved_packages,
    )
    .map_err(|error| {
        render_graph_diagnostics(&error, &texts);
        format_graph_error(error)
    })?;
    if let Some(component) = component {
        let (node, declarations, text) = component.into_preparation()?;
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
        declarations_by_path.insert(node.source.clone(), declarations);
        texts.insert(node.source.clone(), text);
        graph.append_independent_leaf(node)?;
    }
    let mut project_declarations = Vec::new();
    let sources: Vec<_> = graph.nodes.iter().map(|node| {
        compiler_source_identity(&node.source, &node.content_revision)
    }).collect();
    let mut derivation_diagnostics = Vec::new();
    for node in &graph.nodes {
        let declarations = declarations_by_path
            .remove(&node.source)
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
                    target: sources
                        .get(edge.target.0)
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
    render_project_diagnostics(&derivation_diagnostics, &texts);
    if !derivation_diagnostics.is_empty() {
        return Err("build rejected by CST derivation diagnostics".to_owned());
    }
    let initialization_order = graph
        .initialization_order
        .iter()
        .map(|id| {
            sources.get(id.0).cloned()
                .ok_or_else(|| format!("missing initialization node {}", id.0))
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(sources);
    drop(declarations_by_path);
    #[cfg(test)]
    routing_observations::record(|counts| counts.preparations += 1);
    let prepared = prepare_scalar_project(
        project_declarations,
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

    pub(super) fn observe_compile(
        project: &ProjectContext,
        configuration: &ProjectConfiguration,
    ) -> (Result<(String, Vec<SourceObservation>), String>, Counts) {
        struct Restore(Option<Counts>);

        impl Drop for Restore {
            fn drop(&mut self) {
                COUNTS.set(self.0);
            }
        }

        let _restore = Restore(COUNTS.replace(Some(Counts::default())));
        let result = compile_project(
            &project.root,
            project,
            configuration,
            ScalarTargetLayout::NATIVE64,
        );
        (result, COUNTS.get().expect("active routing observations"))
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

    #[test]
    fn release_checked_access_component_is_an_ordinary_independent_graph_leaf() {
        let user = "%%start\nu64() value = fn { 42 };\nvalue();\n%%end";
        let (project, configuration) = routing_test_project("release-access", &[("main.w", user)]);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let (plain, plain_observations) =
                compile_project(&project.root, &project, &configuration, layout).unwrap();
            let (linked, observations) = compile_project_with_component(
                &project.root, &project, &configuration, layout,
                Some(checked_access_component::CheckedAccessComponent::linked()),
            ).unwrap();
            assert_eq!(plain_observations.len(), 1);
            assert_eq!(observations.len(), 2);
            assert_eq!(observations[0], plain_observations[0]);
            assert_eq!(current_source_observation(&observations[1]).unwrap(), observations[1]);
            assert!(!plain.contains("636865636b5f696e6465785f696e5f657874656e74"));
            assert!(linked.lines().any(|line| line.starts_with("define void ")
                && line.contains("636865636b5f696e6465785f696e5f657874656e74")));
            assert!(linked.contains("ret i64 42"));
            let mut stale = observations[1].clone();
            stale.content_revision = ContentRevision("old-source-or-contract".to_owned());
            assert_ne!(current_source_observation(&stale).unwrap(), stale);
        }
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[test]
    fn lookalike_user_namespace_cannot_supply_the_release_checked_access_source() {
        let (mut project, configuration) = routing_test_project(
            "release-access-lookalike",
            &[("main.w", "%%start\nuser = namespace wosy_toolchain_stdlib \"src/checked_access.w\";\nuser.check_index_in_extent(2, 3);\n%%end")],
        );
        // Even the same package and relative path are not the same qualified
        // source. Ordinary user resolution continues to call its own function.
        project.package = "wosy_toolchain_stdlib".to_owned();
        fs::create_dir_all(project.root.join("src")).unwrap();
        let fake = "%%start\nunit(u64, u64) check_index_in_extent = fn(index, count) { index; count; };\n%%end";
        fs::write(project.root.join("src/checked_access.w"), fake).unwrap();

        let (llvm, observations) = compile_project_with_component(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
            Some(checked_access_component::CheckedAccessComponent::linked()),
        ).unwrap();
        let path = PackageRelativePath::new("src/checked_access.w".into()).unwrap();
        let same_path = observations.iter().filter(|entry| entry.source.path == path).collect::<Vec<_>>();
        let [user, release] = same_path.as_slice() else { panic!("expected two independently qualified sources") };
        assert_ne!(user.source, release.source);
        assert_ne!(user.content_revision, release.content_revision);
        assert_eq!(user.source.project, project.root.to_string_lossy());
        assert_eq!(current_source_observation(release).unwrap(), **release);
        let symbol_suffix = "__636865636b5f696e6465785f696e5f657874656e74";
        let user_revision = user.content_revision.0.bytes()
            .map(|byte| format!("{byte:02x}")).collect::<String>();
        let definitions = llvm.lines().filter(|line| line.starts_with("define void ")
            && line.contains(symbol_suffix)).count();
        assert_eq!(definitions, 2);
        let calls = llvm.lines().filter(|line| line.contains("call void @")
            && line.contains(symbol_suffix)).collect::<Vec<_>>();
        let [called] = calls.as_slice() else { panic!("{calls:?}") };
        assert!(called.contains(&user_revision), "{called}");
        fs::remove_dir_all(&project.root).unwrap();
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
