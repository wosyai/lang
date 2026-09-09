use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic as Report, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;
use wosy_compiler::{
    derive_scalar_diagnostics_from_cst, derive_scalar_program_from_cst, emit_scalar_llvm_text,
    emit_scalar_project_llvm, parse_source, publication, serialize_publication,
    validate_scalar_project, ScalarModule, ScalarNamespaceBinding, ScalarProject,
};
use wosy_project::{
    artifact_manifest_path, discover_root, load_artifact_manifest, load_reachable_source_graph,
    publish_artifact, ArtifactIdentity, ContentRevision, NamespaceEdgeInput, PackageRelativePath,
    ProjectConfiguration, ProjectContext, SourceIdentity as ProjectSourceIdentity, SourceNodeInput,
    SourceSpan as ProjectSourceSpan,
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
    let target_name = target.unwrap_or("app");
    let project = ProjectContext::load(&root, target_name)?;
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
    let target_name = target.unwrap_or("app");
    let project = ProjectContext::load(&root, target_name)?;
    let configuration = ProjectConfiguration::load(&root)?;
    let (_target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(target_name, profile)?;
    let (llvm, source_identity) = compile_project(&root, &project)?;
    let artifact_name = &target_profile.main_artifact;
    let output_dir = root
        .join(".wosy/artifacts")
        .join(target_name)
        .join(profile)
        .join(artifact_name);
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let partition_path = output_dir.join("partition.ll");
    fs::write(&partition_path, &llvm).map_err(|error| error.to_string())?;
    let identity = artifact_identity(
        &root,
        &configuration,
        target_name,
        profile,
        artifact,
        artifact_profile,
        &source_identity,
        &llvm,
    )?;
    let executable = output_dir.join("main");
    let pending_manifest = output_dir.join("artifact-manifest.pending.json");
    let pending = wosy_project::ArtifactManifest {
        identity: identity.clone(),
        executable: executable.clone(),
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
        ],
    )?;
    publish_artifact(&output_dir, executable, identity)?;
    fs::remove_file(pending_manifest).map_err(|error| error.to_string())?;
    Ok(())
}

fn run_command(target: Option<&str>, profile: &str, arguments: &[String]) -> Result<u8, String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let target_name = target.unwrap_or("app");
    let project = ProjectContext::load(&root, target_name)?;
    let configuration = ProjectConfiguration::load(&root)?;
    let (_target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(target_name, profile)?;
    let manifest_path =
        artifact_manifest_path(&root, target_name, profile, &target_profile.main_artifact);
    let manifest = load_artifact_manifest(&manifest_path)?;
    let (llvm, source_identity) = compile_project(&root, &project)?;
    let expected = artifact_identity(
        &root,
        &configuration,
        target_name,
        profile,
        artifact,
        artifact_profile,
        &source_identity,
        &llvm,
    )?;
    if manifest.identity != expected {
        return Err("artifact manifest is stale".to_owned());
    }
    if !manifest.executable.is_file() {
        return Err(format!(
            "artifact is missing: {}",
            manifest.executable.display()
        ));
    }
    let runner = configuration
        .runners
        .get(&artifact_profile.run_runner)
        .ok_or_else(|| format!("runner {} is missing", artifact_profile.run_runner))?;
    invoke_direct_with_arguments(
        &root,
        &runner.command,
        &runner.args,
        &manifest_path,
        arguments,
    )
}

fn compile_project(
    root: &std::path::Path,
    project: &ProjectContext,
) -> Result<(String, String), String> {
    let root_path = PackageRelativePath::new(project.source_root.clone())?;
    let mut pending = vec![root_path.clone()];
    let mut index = 0;
    let mut nodes = Vec::new();
    let mut canonical = BTreeMap::new();
    let mut texts = BTreeMap::new();

    while index < pending.len() {
        let path = pending[index].clone();
        index += 1;
        if nodes
            .iter()
            .any(|node: &SourceNodeInput| node.source.path == path)
        {
            continue;
        }
        let source_path = root.join(path.as_path());
        let text = fs::read_to_string(&source_path)
            .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
        let revision = blake3::hash(text.as_bytes()).to_hex().to_string();
        let source = SourceIdentity::new(
            root.to_string_lossy().into_owned(),
            project.package.clone(),
            path.as_path().to_string_lossy().replace('\\', "/"),
            revision.clone(),
        );
        let output = parse_source(source, text.clone(), &project.comment_categories);
        render_diagnostics(&text, &output.diagnostics, &output.result.source);
        if !output.diagnostics.is_empty() {
            return Err("build rejected by syntax diagnostics".to_owned());
        }
        let snapshot_path = persist_cst(root, &output)?;
        let cst = load_canonical_cst(&snapshot_path)?;
        let validation = derive_scalar_program_from_cst(&cst);
        let mut namespace_edges = Vec::new();
        for item in &validation.program.items {
            let wosy_compiler::ScalarItem::Namespace(namespace) = item else {
                continue;
            };
            let module_path = serde_json::from_str::<String>(&namespace.path)
                .map_err(|error| format!("invalid namespace path: {error}"))?;
            let module_path = PackageRelativePath::new(module_path.into())?;
            if root.join(module_path.as_path()).is_file() {
                pending.push(module_path.clone());
            }
            namespace_edges.push(NamespaceEdgeInput {
                origin: namespace.package.clone(),
                path: module_path,
                binding: namespace.binding.clone(),
                span: ProjectSourceSpan {
                    source: ProjectSourceIdentity::new(
                        root.to_string_lossy().into_owned(),
                        project.package.clone(),
                        path.clone(),
                    ),
                    range: wosy_project::ByteSpan::new(namespace.span.start, namespace.span.end),
                },
            });
        }
        let graph_source = ProjectSourceIdentity::new(
            root.to_string_lossy().into_owned(),
            project.package.clone(),
            path.clone(),
        );
        nodes.push(SourceNodeInput {
            source: graph_source,
            content_revision: ContentRevision(revision),
            namespace_edges,
        });
        canonical.insert(path.clone(), cst);
        texts.insert(path, text);
    }

    let root_node = nodes
        .iter()
        .find(|node| node.source.path == root_path)
        .cloned()
        .ok_or_else(|| "project root source is missing".to_owned())?;
    let graph =
        load_reachable_source_graph(&project.package, root_node, nodes).map_err(|error| {
            render_graph_diagnostics(&error);
            format_graph_error(error)
        })?;
    let mut modules = Vec::new();
    let mut source_by_path = BTreeMap::new();
    let mut derivation_diagnostics = Vec::new();
    for node in &graph.nodes {
        let source =
            compiler_source_identity(root, project, &node.source.path, &node.content_revision);
        source_by_path.insert(node.source.path.clone(), source);
    }
    for node in &graph.nodes {
        let cst = canonical.get(&node.source.path).ok_or_else(|| {
            format!(
                "missing canonical CST for {}",
                node.source.path.as_path().display()
            )
        })?;
        derivation_diagnostics.extend(derive_scalar_diagnostics_from_cst(cst));
        let validation = derive_scalar_program_from_cst(cst);
        let namespace_bindings = node
            .namespace_edges
            .iter()
            .map(|edge| {
                Ok(ScalarNamespaceBinding {
                    binding: edge.binding.clone(),
                    target: source_by_path.get(&edge.path).cloned().ok_or_else(|| {
                        format!("missing module {}", edge.path.as_path().display())
                    })?,
                    span: wosy_syntax::ByteSpan::new(edge.span.range.start, edge.span.range.end),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        modules.push(ScalarModule::new(
            source_by_path
                .get(&node.source.path)
                .cloned()
                .ok_or_else(|| {
                    format!("missing module {}", node.source.path.as_path().display())
                })?,
            validation.program.items,
            namespace_bindings,
        ));
    }
    render_project_diagnostics(&derivation_diagnostics, &texts);
    if !derivation_diagnostics.is_empty() {
        return Err("build rejected by CST derivation diagnostics".to_owned());
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
                .get(&node.source.path)
                .cloned()
                .ok_or_else(|| format!("missing module {}", node.source.path.as_path().display()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let validation = validate_scalar_project(ScalarProject::new(modules, initialization_order));
    render_project_diagnostics(&validation.diagnostics, &texts);
    if !validation.diagnostics.is_empty() {
        return Err("build rejected by semantic diagnostics".to_owned());
    }
    let llvm = if graph.nodes.len() == 1 {
        emit_scalar_llvm_text(&derive_scalar_program_from_cst(
            canonical
                .get(&root_path)
                .ok_or_else(|| "missing canonical CST for project root".to_owned())?,
        ))?
    } else {
        let partition = emit_scalar_project_llvm(&validation)?;
        partition.to_text()
    };
    let source_identity = graph
        .nodes
        .iter()
        .map(|node| {
            format!(
                "{}:{}:{}",
                node.source.package,
                node.source.path.as_path().display(),
                node.content_revision.0
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    Ok((llvm, source_identity))
}

fn persist_cst(
    root: &std::path::Path,
    output: &wosy_compiler::ParseOutput,
) -> Result<std::path::PathBuf, String> {
    let path = root
        .join(".wosy/cst")
        .join(format!("{}.json", output.result.source.revision));
    let bytes = serde_json::to_vec_pretty(&output.result.canonical_cst().snapshot())
        .map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(path)
}

fn load_canonical_cst(path: &std::path::Path) -> Result<wosy_syntax::CanonicalCstRoot, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let snapshot = serde_json::from_slice(bytes.as_slice()).map_err(|error| error.to_string())?;
    Ok(wosy_syntax::CanonicalCstRoot::from_snapshot(snapshot))
}

fn compiler_source_identity(
    root: &std::path::Path,
    project: &ProjectContext,
    path: &PackageRelativePath,
    revision: &ContentRevision,
) -> SourceIdentity {
    SourceIdentity::new(
        root.to_string_lossy().into_owned(),
        project.package.clone(),
        path.as_path().to_string_lossy().replace('\\', "/"),
        revision.0.clone(),
    )
}

fn format_graph_error(error: wosy_project::GraphLoadError) -> String {
    error
        .diagnostics
        .into_iter()
        .map(|diagnostic| match diagnostic {
            wosy_project::GraphDiagnostic::MissingModule(diagnostic) => format!(
                "{}: missing module {}",
                diagnostic.code,
                diagnostic.requested_path.as_path().display()
            ),
            wosy_project::GraphDiagnostic::Cycle(diagnostic) => {
                format!("{}: import cycle", diagnostic.code)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_graph_diagnostics(error: &wosy_project::GraphLoadError) {
    for diagnostic in &error.diagnostics {
        match diagnostic {
            wosy_project::GraphDiagnostic::MissingModule(diagnostic) => eprintln!(
                "{}: {}:{}..{} requests {}",
                diagnostic.code,
                diagnostic.edge.source.path.as_path().display(),
                diagnostic.edge.range.start,
                diagnostic.edge.range.end,
                diagnostic.requested_path.as_path().display()
            ),
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
    texts: &BTreeMap<PackageRelativePath, String>,
) {
    for diagnostic in diagnostics {
        let mut files = SimpleFiles::new();
        let labels = diagnostic
            .labels
            .iter()
            .filter_map(|label| {
                let path = PackageRelativePath::new(label.span.source.path.clone().into()).ok()?;
                let text = texts.get(&path)?;
                let file_id = files.add(label.span.source.path.clone(), text.clone());
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
    root: &std::path::Path,
    configuration: &ProjectConfiguration,
    target: &str,
    profile: &str,
    artifact: &wosy_project::ArtifactConfig,
    artifact_profile: &wosy_project::ArtifactProfile,
    source_identity: &str,
    llvm: &str,
) -> Result<ArtifactIdentity, String> {
    let config = fs::read_to_string(root.join("wosy.toml")).map_err(|error| error.to_string())?;
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
        configuration_identity: blake3::hash(config.as_bytes()).to_hex().to_string(),
        artifact_identity: format!("{target}:{profile}:{}", artifact.artifact),
        backend: artifact_profile.backend.clone(),
        output: artifact_profile.output.clone(),
        builder_identity: serde_json::to_string(builder).map_err(|error| error.to_string())?,
        runner_identity: serde_json::to_string(runner).map_err(|error| error.to_string())?,
        llvm_input_identity: blake3::hash(llvm.as_bytes()).to_hex().to_string(),
    })
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
    let mut files = SimpleFiles::new();
    let file_id = files.add(source.path.clone(), source_text.to_owned());
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
