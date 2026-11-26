use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic as Report, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
#[cfg(test)]
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufWriter, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::process::ExitCode;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, emit_scalar_build_project_artifact_llvm,
    emit_scalar_build_single_module_artifact_llvm, emit_scalar_build_wasm_root_exports_artifact_llvm,
    parse_source_with_snapshot, prepare_scalar_project, publication, serialize_publication,
    LlvmPartition, ParseOutput, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_project::{
    artifact_manifest_path, discover_root, load_artifact_manifest, publish_artifact,
    ArtifactIdentity, ContentRevision, PackageRelativePath, ProjectConfiguration, ProjectContext,
    SourceIdentity as ProjectSourceIdentity, SourceObservation,
};
#[cfg(test)]
use wosy_compiler::parse_source;
use wosy_syntax::SourceIdentity;

#[derive(Parser)]
#[command(name = "wosy")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a base-language project in the current directory
    Init,
    /// Manage project dependencies
    Dep {
        #[command(subcommand)]
        command: DepCommand,
    },
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

#[derive(Subcommand)]
enum DepCommand {
    /// Add a local package or resolve and lock a Git package
    Add {
        source: String,
        #[arg(long)]
        path: Option<String>,
    },
    /// Materialize the exact dependencies in wosy.lock
    Sync,
    /// Record live Git dependency edits as project patches
    Snapshot,
    /// Advance locked Git dependencies and replay their project patches
    Update,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Init => match env::current_dir()
            .map_err(|error| error.to_string())
            .and_then(|root| wosy_project::packages::init_project(&root))
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("wosy: {error}");
                ExitCode::from(1)
            }
        },
        Command::Dep { command } => {
            let result = match command {
                DepCommand::Add { source, path } => dep_command(|root| {
                    wosy_project::packages::add_dependency(root, &source, path.as_deref())
                }),
                DepCommand::Sync => dep_command(wosy_project::packages::sync_dependencies),
                DepCommand::Snapshot => dep_command(wosy_project::packages::snapshot_dependencies),
                DepCommand::Update => dep_command(wosy_project::packages::update_dependencies),
            };
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("wosy: {error}");
                    ExitCode::from(1)
                }
            }
        }
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

fn dep_command(action: impl FnOnce(&std::path::Path) -> Result<(), String>) -> Result<(), String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    action(&root)
}

fn parse_command(target: Option<&str>) -> Result<bool, String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let project = ProjectContext::load_parse_target(&root, target)?;
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
    let (fresh, diagnostics) = parse_source_with_snapshot(source, text, &project.comment_categories);
    persist_cst(
        &root,
        &fresh.result().source.revision,
        &fresh.snapshot_json_serializable(),
    )?;
    let output = ParseOutput {
        result: fresh.into_result(),
        diagnostics,
    };
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
    let configuration_identity = project_configuration_identity(document.raw().as_bytes(), &configuration)?;
    drop(document);
    let (llvm, source_observations) = compile_project(
        &root,
        &project,
        &configuration,
        target_layout,
        artifact_profile.exports.as_ref(),
    )?;
    let source_identity = wosy_project::source_identity(&source_observations);
    let artifact_name = &target_profile.main_artifact;
    let output_dir = root
        .join(".wosy/artifacts")
        .join(&target_name)
        .join(profile)
        .join(artifact_name);
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let partition_path = output_dir.join("partition.ll");
    let llvm_input_identity = write_llvm_partition(&partition_path, &llvm)?;
    drop(llvm);
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
        source_identity,
        llvm_input_identity,
    )?;
    let executable = output_dir.join("main");
    let pending_manifest = output_dir.join("artifact-manifest.pending.json");
    let pending = wosy_project::ArtifactManifest {
        identity,
        executable,
        source_observations,
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
    let wosy_project::ArtifactManifest {
        identity,
        executable,
        source_observations,
    } = pending;
    publish_artifact(&output_dir, executable, identity, source_observations)?;
    fs::remove_file(pending_manifest).map_err(|error| error.to_string())?;
    Ok(())
}

struct HashingWriter<W> {
    output: W,
    identity: blake3::Hasher,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.output.write(bytes)?;
        self.identity.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

fn write_llvm_partition(path: &std::path::Path, llvm: &LlvmPartition) -> Result<String, String> {
    let file = fs::File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(HashingWriter {
        output: file,
        identity: blake3::Hasher::new(),
    });
    llvm.write_text(&mut writer).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())?;
    Ok(writer.get_ref().identity.finalize().to_hex().to_string())
}

fn project_configuration_identity(
    manifest: &[u8],
    configuration: &ProjectConfiguration,
) -> Result<String, String> {
    let manifest_identity = blake3::hash(manifest);
    // A manifest that declared source_root used to route its own namespaces
    // from the project root. Do not accept a pre-change artifact that observed
    // those other files, even if they still exist with the original bytes.
    let manifest_identity = if configuration.module_source_root.is_some() {
        blake3::Hasher::new()
            .update(b"wosy-selected-package-namespace-root-v1\0")
            .update(manifest_identity.as_bytes())
            .finalize()
    } else {
        manifest_identity
    };
    if configuration.resolved_packages.is_empty() {
        return Ok(manifest_identity.to_hex().to_string());
    }
    fn field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"wosy-package-closure-v2\0");
    hasher.update(manifest_identity.as_bytes());
    hasher.update(&(configuration.resolved_packages.len() as u64).to_le_bytes());
    for (name, package) in &configuration.resolved_packages {
        let source_root: std::path::PathBuf = package.identity.path.as_path().components()
            .filter_map(|component| match component {
                std::path::Component::Normal(name) => Some(name),
                _ => None,
            }).collect();
        field(&mut hasher, name.as_bytes());
        field(&mut hasher, source_root.as_os_str().as_encoded_bytes());
        if let Some(pin) = &package.git_pin_identity {
            field(&mut hasher, b"git");
            field(&mut hasher, pin.as_bytes());
            let patches = package.patch_stack_id
                .ok_or_else(|| format!("Git dependency {name} has no verified patch stack"))?;
            field(&mut hasher, patches.as_bytes());
        } else {
            field(&mut hasher, b"local");
            let local = package.local_root_identity.as_ref()
                .ok_or_else(|| format!("local dependency {name} has no canonical path"))?;
            field(&mut hasher, local.as_os_str().as_encoded_bytes());
        }
        hasher.update(&(package.dependencies.len() as u64).to_le_bytes());
        for dependency in &package.dependencies {
            field(&mut hasher, dependency.as_bytes());
        }
    }
    let identity = hasher.finalize();
    Ok(identity.to_hex().to_string())
}

fn run_command(target: Option<&str>, profile: &str, arguments: &[String]) -> Result<u8, String> {
    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let (configuration, document) = ProjectConfiguration::load_with_manifest(&root)?;
    let target_name = configuration.resolve_target(target)?;
    let (_target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(&target_name, profile)?;
    let configuration_identity = project_configuration_identity(document.raw().as_bytes(), &configuration)?;
    drop(document);
    let manifest_path =
        artifact_manifest_path(&root, &target_name, profile, &target_profile.main_artifact);
    let manifest = load_artifact_manifest(&manifest_path)
        .map_err(|error| format!("artifact manifest error: {error}"))?;
    if manifest.identity.configuration_identity != configuration_identity {
        return Err("artifact manifest is stale".to_owned());
    }
    let current_observations = manifest
        .source_observations
        .iter()
        .map(|observation| {
            let content_revision = if let Some((package, checkout)) =
                configuration.git_package(&observation.source.package)?
            {
                if observation.source.project != package.identity.project {
                    return Err("artifact manifest is stale".to_owned());
                }
                let relative = observation.source.path.as_path();
                let file = checkout.source(relative)?
                    .ok_or_else(|| "artifact manifest is stale".to_owned())?;
                observe_open_source(file, relative)?
            } else {
                observe_source_revision(&source_observation_path(observation))?
            };
            Ok(SourceObservation {
                source: observation.source.clone(),
                content_revision,
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

fn observe_source_revision(path: &std::path::Path) -> Result<ContentRevision, String> {
    let read_error = |error| format!("failed to read {}: {error}", path.display());
    let file = fs::File::open(path).map_err(read_error)?;
    observe_open_source(file, path)
}

fn observe_open_source(file: fs::File, path: &std::path::Path) -> Result<ContentRevision, String> {
    let read_error = |error| format!("failed to read {}: {error}", path.display());
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file).map_err(read_error)?;
    Ok(ContentRevision(hasher.finalize().to_hex().to_string()))
}

fn compile_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    root_exports: Option<&BTreeMap<String, String>>,
) -> Result<(LlvmPartition, Vec<SourceObservation>), String> {
    let mut declarations_by_path = BTreeMap::new();
    let mut texts = BTreeMap::new();
    let graph = wosy_cli::project_sources::load(
        root,
        project,
        configuration,
        |source, revision, text| {
            let identity = compiler_source_identity(source, revision);
            let (fresh, diagnostics) =
                parse_source_with_snapshot(identity, text, &project.comment_categories);
            let result = fresh.result();
            render_diagnostics(&result.text, &diagnostics, &result.source);
            if !diagnostics.is_empty() {
                return Err("build rejected by syntax diagnostics".to_owned());
            }
            persist_cst(root, &result.source.revision, &fresh.snapshot_json_serializable())?;
            #[cfg(test)]
            routing_observations::record(|counts| counts.extractions += 1);
            let declarations = derive_scalar_declarations_from_cst(result.canonical_cst());
            let imports = wosy_cli::project_sources::imports(&declarations)?;
            declarations_by_path.insert((source.package.clone(), source.path.clone()), declarations);
            texts.insert(source.clone(), fresh.into_result().text);
            Ok(imports)
        },
    )
    .map_err(|error| match error {
        wosy_cli::project_sources::Error::Observe(error) => error,
        wosy_cli::project_sources::Error::Graph(error) => {
            render_graph_diagnostics(&error, &texts);
            format_graph_error(error)
        }
    })?;
    let source_identities: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| compiler_source_identity(&node.source, &node.content_revision))
        .collect();
    let root_exports = root_exports.map(|exports| (exports, source_identities[0].clone()));
    let mut project_declarations = Vec::with_capacity(graph.nodes.len());
    let mut derivation_diagnostics = Vec::new();
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
            .map(|edge| ScalarNamespaceBinding {
                binding: edge.binding.clone(),
                target: source_identities[edge.target.0].clone(),
                span: wosy_syntax::ByteSpan::new(edge.span.range.start, edge.span.range.end),
            })
            .collect();
        project_declarations.push((declarations, namespace_bindings));
    }
    render_project_diagnostics(&derivation_diagnostics, &texts);
    if !derivation_diagnostics.is_empty() {
        return Err("build rejected by CST derivation diagnostics".to_owned());
    }
    let initialization_order = graph
        .initialization_order
        .iter()
        .map(|id| source_identities[id.0].clone())
        .collect();
    drop(source_identities);
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
    let llvm = if let Some((exports, source)) = root_exports {
        emit_scalar_build_wasm_root_exports_artifact_llvm(&validation, &source, exports)?
    } else if graph.nodes.len() == 1 {
        #[cfg(test)]
        routing_observations::record(|counts| counts.single_module_emissions += 1);
        emit_scalar_build_single_module_artifact_llvm(&validation)?
    } else {
        #[cfg(test)]
        routing_observations::record(|counts| counts.project_emissions += 1);
        emit_scalar_build_project_artifact_llvm(&validation)?
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

fn persist_cst(
    root: &std::path::Path,
    revision: &str,
    snapshot: &impl serde::Serialize,
) -> Result<(), String> {
    let path = root.join(".wosy/cst").join(format!("{revision}.json"));
    let create_parent = || {
        path.parent().map_or(Ok(()), |parent| {
            fs::create_dir_all(parent).map_err(|error| error.to_string())
        })
    };
    #[cfg(target_os = "linux")]
    {
        create_parent()?;
        publish_cst_snapshot(&path, snapshot)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let bytes = serde_json::to_vec_pretty(snapshot).map_err(|error| error.to_string())?;
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
        let stream = StandardStream::stderr(ColorChoice::Auto);
        let mut stream_lock = stream.lock();
        let _ = wosy_cli::diagnostic_output::emit_project_diagnostic(&mut stream_lock, diagnostic, |source| {
            let key = ProjectSourceIdentity::new(
                source.project.clone(), source.package.clone(),
                PackageRelativePath::new(source.path.clone().into()).ok()?,
            );
            texts.get(&key).map(String::as_str)
        });
    }
}

fn artifact_identity(
    configuration_identity: String,
    configuration: &ProjectConfiguration,
    target: &str,
    profile: &str,
    artifact: &wosy_project::ArtifactConfig,
    artifact_profile: &wosy_project::ArtifactProfile,
    source_identity: String,
    llvm_input_identity: String,
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
        source_identity,
        configuration_identity,
        artifact_identity: format!("{target}:{profile}:{}", artifact.artifact),
        backend: artifact_profile.backend.clone(),
        output: artifact_profile.output.clone(),
        builder_identity: serde_json::to_string(builder).map_err(|error| error.to_string())?,
        runner_identity: serde_json::to_string(runner).map_err(|error| error.to_string())?,
        llvm_input_identity,
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
            None,
        )
        .map(|(llvm, sources)| (llvm.to_text(), sources));
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
            module_source_root: None,
            builders: BTreeMap::new(),
            runners: BTreeMap::new(),
            targets: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            resolved_packages: BTreeMap::new(),
        };
        (project, configuration)
    }

    fn assert_wasm_root_forwarder(llvm: &str, label: &str, result: &str, types: &[&str]) {
        let header = format!("define {result} @{label}(");
        let body = llvm
            .split_once(header.as_str())
            .unwrap_or_else(|| panic!("missing wrapper {label}: {llvm}"))
            .1;
        let (args, body) = body.split_once(')').expect("wrapper argument list");
        let parameters = args.split(',').map(str::trim).collect::<Vec<_>>();
        assert_eq!(parameters.len(), types.len(), "{label}: {args}");
        for (parameter, ty) in parameters.iter().zip(types) {
            let (actual, name) = parameter.split_once(' ').expect("typed wrapper parameter");
            assert_eq!(&actual, ty, "{label}: {parameter}");
            assert!(name.starts_with('%'), "{label}: {parameter}");
        }
        let body = body.split_once("\n}").expect("wrapper body").0;
        let call_start = format!("call {result} @wosy_fn__");
        let call = body.split_once(call_start.as_str()).expect("qualified root call").1;
        let forwarded = call.split_once('(').expect("call arguments").1;
        let forwarded = forwarded.split_once(')').expect("end of call arguments").0;
        assert_eq!(
            forwarded.split(',').map(str::trim).collect::<Vec<_>>(),
            parameters,
            "{label}: {body}",
        );
    }

    #[test]
    fn writes_partition_and_hashes_exact_canonical_bytes() {
        let sources = [(
            "main.w",
            "%%start\nenv = extern wasm \"host\" { unit() log; };\nenv.log();\n%%end\n",
        )];
        let (project, configuration) = routing_test_project("streamed-partition", &sources);
        let (partition, observations) = compile_project(
            &project.root,
            &project,
            &configuration,
            ScalarTargetLayout::WASM32,
            None,
        )
        .expect("compile actual extern partition");
        let path = project.root.join("partition.ll");
        let expected = partition.to_text();
        let identity = write_llvm_partition(&path, &partition).expect("stream partition");
        assert_eq!(fs::read(&path).expect("partition bytes"), expected.as_bytes());
        assert_eq!(identity, blake3::hash(expected.as_bytes()).to_hex().as_str());
        assert!(expected.contains(r#"wasm-import-module"="host""#));
        assert_eq!(observations.len(), 1);
        fs::remove_dir_all(&project.root).expect("remove routing fixture");
    }

    #[test]
    fn partition_writer_hashes_only_bytes_accepted_by_output() {
        let mut storage = [0; 3];
        let mut writer = HashingWriter {
            output: &mut storage[..],
            identity: blake3::Hasher::new(),
        };
        assert_eq!(writer.write(b"abcdef").expect("short write"), 3);
        assert!(writer.write_all(b"def").is_err());
        assert_eq!(writer.identity.finalize(), blake3::hash(b"abc"));
        assert_eq!(storage, *b"abc");
    }

    #[test]
    fn selected_wasm_root_exports_keep_mixed_abi_and_exact_root_identity() {
        let sources = [
            (
                "main.w",
                "%%start\nmath = namespace app \"math.w\";\ni32(f32,i32,f32) score = fn(left,points,right) { if (left < right) { math.score() + points } else { points } };\nunit(f32,f32,f32,i32,f32) step = fn(a,b,c,n,d) { if (a + b + c > d) { if (n > 0) {}; }; };\n%%end\n",
            ),
            ("math.w", "%%start\ni32() score = fn { 42 };\ni32() hidden = fn { 7 };\n%%end\n"),
        ];
        let (project, configuration) = routing_test_project("explicit-wasm-root-exports", &sources);
        let compile = |exports| {
            compile_project(
                &project.root,
                &project,
                &configuration,
                ScalarTargetLayout::WASM32,
                exports,
            )
            .map(|(llvm, sources)| (llvm.to_text(), sources))
        };
        let exports = BTreeMap::from([
            ("demo_score".into(), "score".into()),
            ("tick".into(), "step".into()),
        ]);
        let (llvm, _) = compile(Some(&exports)).expect("emit selected root exports");
        assert_wasm_root_forwarder(&llvm, "demo_score", "i32", &["float", "i32", "float"]);
        assert_wasm_root_forwarder(&llvm, "tick", "void", &["float", "float", "float", "i32", "float"]);
        let (default_llvm, _) = compile(None).expect("emit the unchanged implicit partition");
        assert!(!default_llvm.contains("@demo_score"), "{default_llvm}");
        assert!(!default_llvm.contains("@tick"), "{default_llvm}");
        let absent = BTreeMap::from([("demo_score".into(), "hidden".into())]);
        assert!(compile(Some(&absent)).expect_err("dependency is not the root").contains("no root function"));
        let reserved = BTreeMap::from([("_start".into(), "score".into())]);
        assert!(compile(Some(&reserved)).expect_err("runtime label is reserved").contains("reserved"));
        fs::remove_dir_all(&project.root).expect("remove routing fixture");
    }

    #[test]
    fn singleton_wasm_root_export_keeps_identity_and_rejects_symbol_collision() {
        let sources = [(
            "main.w",
            "%%start\ni32() score = fn { 42 };\ni32() occupied = fn { 7 };\n%%end\n",
        )];
        let (project, configuration) = routing_test_project("singleton-root-export", &sources);
        let compile = |exports| {
            compile_project(
                &project.root,
                &project,
                &configuration,
                ScalarTargetLayout::WASM32,
                exports,
            )
            .map(|(llvm, sources)| (llvm.to_text(), sources))
        };
        let same = BTreeMap::from([("score".into(), "score".into())]);
        let (llvm, _) = compile(Some(&same)).expect("use emitted score symbol");
        let (default_llvm, _) = compile(None).expect("ordinary singleton");
        assert_eq!(llvm, default_llvm);
        let collision = BTreeMap::from([("occupied".into(), "score".into())]);
        assert!(
            compile(Some(&collision))
                .expect_err("cannot reuse another function's emitted symbol")
                .contains("collides with an emitted symbol")
        );
        fs::remove_dir_all(&project.root).expect("remove singleton routing fixture");
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
        let (fresh, diagnostics) = parse_source_with_snapshot(
            source,
            include_str!("../../../stdlib/src/bootstrap.w").into(),
            &[],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let root = snapshot_test_path("deep").with_extension("");
        let path = root.join(".wosy/cst/r1.json");
        let tree = fresh.result().canonical_cst();
        let borrowed = serde_json::to_vec_pretty(&tree.snapshot_json_serializable())
            .expect("serialize borrowed CST snapshot");
        let owned = serde_json::to_vec_pretty(&tree.snapshot())
            .expect("serialize owned CST snapshot");
        assert!(borrowed == owned, "borrowed CST snapshot changed bytes");
        persist_cst(
            &root,
            &fresh.result().source.revision,
            &fresh.snapshot_json_serializable(),
        )
        .expect("publish archived snapshot");
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
    fn streamed_source_revision_preserves_raw_bytes_and_read_errors() {
        let path = snapshot_test_path("streamed-source-revision");
        let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(147_459).collect();
        fs::write(&path, &bytes).expect("write multichunk non-UTF-8 source");
        assert_eq!(
            observe_source_revision(&path).expect("read source revision"),
            ContentRevision(blake3::hash(&bytes).to_hex().to_string()),
        );

        fs::write(&path, []).expect("empty source");
        assert_eq!(
            observe_source_revision(&path).expect("read empty source revision"),
            ContentRevision(blake3::hash(&[]).to_hex().to_string()),
        );

        fs::remove_file(&path).expect("remove fixture");
        let error = observe_source_revision(&path).expect_err("report missing file");
        assert!(error.starts_with(&format!("failed to read {}: ", path.display())));
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
