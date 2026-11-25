use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic as Report, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
#[cfg(test)]
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{BufWriter, Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::process::ExitCode;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, emit_scalar_build_project_artifact_llvm,
    emit_scalar_build_single_module_artifact_llvm, emit_scalar_build_wasm_root_exports_artifact_llvm,
    parse_source_with_snapshot, prepare_scalar_project_with_access,
    publication, LlvmPartition,
    serialize_publication, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_project::{
    artifact_manifest_path, discover_root, load_artifact_manifest, load_reachable_source_graph,
    publish_artifact, ArtifactIdentity, ContentRevision, NamespaceEdgeInput, NamespacePackages,
    PackageRelativePath, ProjectConfiguration, ProjectContext,
    SourceIdentity as ProjectSourceIdentity, SourceNodeInput, SourceObservation,
    SourceSpan as ProjectSourceSpan,
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
    persist_cst(&root, &fresh.result().source.revision, &fresh.snapshot_json_serializable())?;
    let output = wosy_compiler::ParseOutput {
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
    let (llvm, source_observations) = compile_project_partition(
        &root, &project, &configuration, target_layout, artifact_profile.exports.as_ref(),
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
        .map(|observation| current_source_observation(&configuration, observation))
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

fn current_source_observation(
    configuration: &ProjectConfiguration,
    observation: &SourceObservation,
) -> Result<SourceObservation, String> {
    if let Some(component) = checked_access_component::refresh_observation(&observation.source)? {
        return Ok(component);
    }
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

#[cfg(test)]
fn compile_project(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
) -> Result<(String, Vec<SourceObservation>), String> {
    compile_project_with_root_exports(root, project, configuration, target_layout, None)
}

#[cfg(test)]
fn compile_project_with_root_exports(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    root_exports: Option<&BTreeMap<String, String>>,
) -> Result<(String, Vec<SourceObservation>), String> {
    compile_project_partition(root, project, configuration, target_layout, root_exports)
        .map(|(partition, sources)| (partition.to_text(), sources))
}

fn compile_project_partition(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    root_exports: Option<&BTreeMap<String, String>>,
) -> Result<(LlvmPartition, Vec<SourceObservation>), String> {
    compile_project_with_access_policy(
        root, project, configuration, target_layout,
        Some(checked_access_component::CheckedAccessComponent::linked()), true, root_exports,
    )
}

#[cfg(test)]
fn compile_project_with_component(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    component: Option<checked_access_component::CheckedAccessComponent>,
) -> Result<(String, Vec<SourceObservation>), String> {
    compile_project_with_access_policy(root, project, configuration, target_layout, component, false, None)
        .map(|(partition, sources)| (partition.to_text(), sources))
}

fn compile_project_with_access_policy(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    component: Option<checked_access_component::CheckedAccessComponent>,
    demand_only: bool,
    root_exports: Option<&BTreeMap<String, String>>,
) -> Result<(LlvmPartition, Vec<SourceObservation>), String> {
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
    let mut root_node = None;
    let mut nodes = Vec::new();
    let mut declarations_by_path = BTreeMap::new();
    let mut texts = BTreeMap::new();

    while let Some((package, path, checked_git_source)) = pending.pop_front() {
        let (package_root, project_identity) = if package == project.package {
            (root, root_project.clone())
        } else {
            let resolved = configuration
                .resolved_packages
                .get(package)
                .ok_or_else(|| format!("dependency package {package} is missing"))?;
            (
                resolved.project_root.as_path(),
                resolved.identity.project.clone(),
            )
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
        let revision = blake3::hash(text.as_bytes()).to_hex().to_string();
        let source = SourceIdentity::new(
            project_identity,
            package.to_owned(),
            path.as_path().to_string_lossy().replace('\\', "/"),
            revision.clone(),
        );
        let (fresh, diagnostics) = parse_source_with_snapshot(source, text, &project.comment_categories);
        let result = fresh.result();
        let source = &result.source;
        render_diagnostics(&result.text, &diagnostics, source);
        if !diagnostics.is_empty() {
            return Err("build rejected by syntax diagnostics".to_owned());
        }
        persist_cst(root, &source.revision, &fresh.snapshot_json_serializable())?;
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
        let declarations = derive_scalar_declarations_from_cst(result.canonical_cst());
        let mut namespace_edges = Vec::new();
        for namespace in declarations.namespaces() {
            let module_path = serde_json::from_str::<String>(&namespace.path)
                .map_err(|error| format!("invalid namespace path: {error}"))?;
            let module_path = PackageRelativePath::new(module_path.into())?;
            let module = packages
                .resolve(&namespace.package, &module_path)
                .ok_or_else(|| format!("dependency package {} is missing", namespace.package))?;
            let target_package = module.package;
            let already_discovered = discovered
                .get(target_package)
                .is_some_and(|paths| paths.contains(module.path.as_ref()));
            if !already_discovered {
                let available = if let Some((_, checkout)) = configuration.git_package(target_package)? {
                    checkout.source(module.path.as_path())?.map(Some)
                } else {
                    let target_root = module
                        .dependency
                        .map_or(root, |resolved| resolved.project_root.as_path());
                    target_root.join(module.path.as_path()).is_file().then_some(None)
                };
                if let Some(checked_git_source) = available {
                    let target_path = module.path.into_owned();
                    discovered
                        .entry(target_package)
                        .or_default()
                        .insert(target_path.clone());
                    pending.push_back((target_package, target_path, checked_git_source));
                }
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
            path,
        );
        let node = SourceNodeInput {
            source: graph_source.clone(),
            content_revision: ContentRevision(revision),
            namespace_edges,
        };
        if root_node.is_none() {
            root_node = Some(node);
        } else {
            nodes.push(node);
        }
        declarations_by_path.insert(graph_source.clone(), declarations);
        texts.insert(graph_source, fresh.into_result().text);
    }
    drop((discovered, pending));

    let root_node = root_node.ok_or_else(|| "project root source is missing".to_owned())?;
    let mut graph = load_reachable_source_graph(&packages, root_node, nodes).map_err(|error| {
        render_graph_diagnostics(&error, &texts);
        format_graph_error(error)
    })?;
    let mut demanded_component = None;
    if let Some(component) = component {
        if demand_only {
            let (node, access) = component.into_source();
            texts.insert(node.source.clone(), access.text().to_owned());
            demanded_component = Some((node, access));
        } else {
            let (node, declarations, text) = component.into_preparation()?;
            #[cfg(test)]
            routing_observations::record(|counts| counts.extractions += 1);
            declarations_by_path.insert(node.source.clone(), declarations);
            texts.insert(node.source.clone(), text);
            graph.append_independent_leaf(node)?;
        }
    }
    let mut project_declarations = Vec::with_capacity(graph.nodes.len());
    let sources: Vec<_> = graph.nodes.iter().map(|node| {
        compiler_source_identity(&node.source, &node.content_revision)
    }).collect();
    let export_root = root_exports.map(|_| sources[0].clone());
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
            .map(|edge| ScalarNamespaceBinding {
                binding: edge.binding.clone(),
                target: sources[edge.target.0].clone(),
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
        .map(|id| sources[id.0].clone())
        .collect();
    drop(sources);
    drop(declarations_by_path);
    #[cfg(test)]
    routing_observations::record(|counts| counts.preparations += 1);
    let (node, access) = match demanded_component {
        Some((node, access)) => (Some(node), Some(access)),
        None => (None, None),
    };
    let prepared = prepare_scalar_project_with_access(
        project_declarations,
        initialization_order,
        ScalarPreparationInputs { target_layout },
        access,
    )
    .map_err(|rejected| {
        render_project_diagnostics(&rejected.into_ordered_diagnostics(), &texts);
        "build rejected by semantic diagnostics".to_owned()
    })?;
    if let (Some(node), Some(source)) = (node, prepared.checked_access_source()) {
        if compiler_source_identity(&node.source, &node.content_revision) != *source {
            return Err("release checked-access source identity changed during preparation".to_owned());
        }
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
        graph.append_independent_leaf(node)?;
    }
    #[cfg(test)]
    routing_observations::record(|counts| counts.builds += 1);
    let validation = analyze_scalar_build(&prepared);
    render_project_diagnostics(validation.diagnostics(), &texts);
    if !validation.diagnostics().is_empty() {
        return Err("build rejected by semantic diagnostics".to_owned());
    }
    drop(texts);
    let singleton = graph.nodes.len() == 1;
    #[cfg(test)]
    routing_observations::record(|counts| {
        if singleton { counts.single_module_emissions += 1; } else { counts.project_emissions += 1; }
    });
    let llvm = if let (Some(exports), Some(root)) = (root_exports, export_root.as_ref()) {
        emit_scalar_build_wasm_root_exports_artifact_llvm(&validation, root, exports)?
    } else if singleton {
        emit_scalar_build_single_module_artifact_llvm(&validation)?
    } else {
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
    format!("{}+checked-access-fixed-runtime-local-v1+configured-wasm-web-v1", wosy_compiler::COMPILER_IDENTITY)
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
        observe(|| compile_project(
            &project.root,
            project,
            configuration,
            ScalarTargetLayout::NATIVE64,
        ))
    }

    pub(super) fn observe<T>(compile: impl FnOnce() -> T) -> (T, Counts) {
        struct Restore(Option<Counts>);

        impl Drop for Restore {
            fn drop(&mut self) {
                COUNTS.set(self.0);
            }
        }

        let _restore = Restore(COUNTS.replace(Some(Counts::default())));
        let result = compile();
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

    #[test]
    fn default_build_wires_release_checked_access_without_a_user_std_import() {
        let text = r#"%%start
struct copy Entry { u8 value; }
struct RawBytes { u8[3] values; }
Entry[2] entries = [{ .value = 1; }, { .value = 2; }];
u64(u64) select_index = fn(index) { index };
u8(u8) replacement = fn(value) { value };
unit(u64, u8) aggregate_update = fn(index, value) {
    entries[select_index(index)] = { .value = replacement(value); };
};
u8(u64, u8) scalar_index = fn(index, value) {
    u8[3] cells = [7, 8, 9];
    cells[index] = value;
    cells[index]
};
u8(*u8) read_checked = fn(value) { *value };
u8() witness = fn {
    u8 byte = 42;
    read_checked(&byte)
};
u8(u64) read_explicit_raw = fn(index) {
    RawBytes record = { .values = [4, 5, 6]; };
    unsafe {
        *?RawBytes raw = &?record;
        *?u8 element = &?(*raw).values[index];
        core.load<u8>(element)
    }
};
aggregate_update(1, 12);
if (scalar_index(2, 23) != 23 || witness() != 42 || read_explicit_raw(1) != 5) { core.system_panic(); };
%%end"#;
        let (project, configuration) = routing_test_project("default-release-access", &[("main.w", text)]);
        let hex = |name: &str| name.bytes().map(|byte| format!("{byte:02x}")).collect::<String>();
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let (llvm, observations) = compile_project(&project.root, &project, &configuration, layout).unwrap();
            let [_, access] = observations.as_slice() else { panic!("expect one release component") };
            assert_eq!(current_source_observation(&configuration, access).unwrap(), *access);
            let release = hex(&access.content_revision.0);
            let bounds = format!("__{release}__{}", hex("check_index_in_extent"));
            let non_null = format!("__{release}__{}", hex("check_non_null_address"));
            let body = |name: &str| {
                let suffix = format!("__{}(", hex(name));
                let (_, rest) = llvm.lines().find(|line| line.starts_with("define ") && line.contains(&suffix))
                    .and_then(|line| llvm.split_once(line)).expect("ordinary function definition");
                rest.split_once("\n}").unwrap().0
            };
            let aggregate = body("aggregate_update");
            let stage_rhs = aggregate.find(&format!("__{}(", hex("replacement"))).unwrap();
            let selector = format!("__{}(", hex("select_index"));
            let stage_index = aggregate.find(&selector).unwrap();
            let check = aggregate.find(&bounds).unwrap();
            let address = aggregate[check..].find("getelementptr inbounds").unwrap() + check;
            assert!(stage_rhs < stage_index && stage_index < check && check < address);
            assert_eq!(aggregate.matches(&selector).count(), 1);
            assert_eq!(aggregate.matches(&bounds).count(), 1);
            assert!(!aggregate.contains("assignment_index_out_of_bounds"));
            let call = aggregate.lines().find(|line| line.contains(&bounds)).unwrap();
            let (index, extent) = call.split_once("(i64 ").unwrap().1.split_once(", i64 ").unwrap();
            assert_eq!(extent, "2)");
            let assert_guarded_address = |between: &str, gep: &str, index: &str| {
                let narrowing = format!(" = trunc i64 {index} to i32");
                if layout == ScalarTargetLayout::WASM32 {
                    let mut narrowed = between.lines().filter_map(|l| l.trim().strip_suffix(&narrowing));
                    let selected = narrowed.next().expect("narrow the guarded u64 after checking it");
                    assert!(narrowed.next().is_none());
                    assert!(gep.ends_with(&format!(", i32 {selected}")), "{gep}");
                } else {
                    assert!(!between.contains(&narrowing));
                    assert!(gep.ends_with(&format!(", i64 {index}")), "{gep}");
                }
            };
            assert_guarded_address(&aggregate[check..address], aggregate[address..].lines().next().unwrap(), index);

            let scalar = body("scalar_index");
            let mut checks = 0;
            let mut outstanding = None;
            let mut offset = 0;
            for line in scalar.split_inclusive('\n') {
                let at = offset;
                offset += line.len();
                let line = line.trim_end();
                if line.contains(&bounds) {
                    assert!(outstanding.is_none(), "index guard was not consumed by an address");
                    let arguments = line.split_once("(i64 ").unwrap().1;
                    let (index, length) = arguments.split_once(", i64 ").unwrap();
                    assert_eq!(length, "3)");
                    outstanding = Some((at, index));
                    checks += 1;
                }
                if line.contains("getelementptr inbounds [3 x i8]")
                    && line.rsplit_once(", ").is_some_and(|(_, operand)| operand.contains(" %"))
                {
                    let (from, index) = outstanding.take().expect("dynamic fixed-array GEP needs a guard");
                    assert_guarded_address(&scalar[from..at], line, index);
                }
            }
            assert_eq!(checks, 2);
            assert!(outstanding.is_none());

            let dereference = body("read_checked");
            assert_eq!(dereference.matches(&non_null).count(), 1);
            let check = dereference.lines().find(|line| line.contains(&non_null)).unwrap();
            let width = layout.pointer_size * 8;
            let argument = check.split_once(&format!("(i{width} ")).unwrap().1.trim_end_matches(')');
            assert!(!argument.is_empty());
            assert!(dereference.find(&non_null).unwrap()
                < dereference.find(&format!("inttoptr i{width} {argument} to ptr")).unwrap());
            assert!(!dereference.contains("deref_is_null"));
            assert!(!dereference.contains("__wosy_core_system_panic"));
            let raw = body("read_explicit_raw");
            assert!(raw.contains("getelementptr inbounds [3 x i8]"));
            assert!(!raw.contains(&non_null) && !raw.contains(&bounds), "{raw}");
        }
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[test]
    fn default_build_preserves_original_runtime_extent_in_local_and_same_backing_routes() {
        let fixtures = [
            (
                "runtime-local-extent",
                include_str!("../../../tests/fixtures/build/checked_access_release/project/src/main.w"),
                "runtime_update",
                false,
            ),
            (
                "runtime-closed-rebind",
                include_str!("../../../tests/fixtures/build/checked_access_release/project/src/runtime_rebound.w"),
                "drive",
                true,
            ),
        ];
        let hex = |name: &str| name.bytes().map(|byte| format!("{byte:02x}")).collect::<String>();
        for (name, text, function, rebound) in fixtures {
            let (project, configuration) = routing_test_project(name, &[("main.w", text)]);
            for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
                let (llvm, observations) = compile_project(&project.root, &project, &configuration, target).unwrap();
                let [_, access] = observations.as_slice() else { panic!("{name}: missing release bounds source") };
                let guard = format!("__{}__{}", hex(&access.content_revision.0), hex("check_index_in_extent"));
                let fn_suffix = format!("__{}(", hex(function));
                let definition = llvm.lines().find(|line| line.starts_with("define ") && line.contains(&fn_suffix))
                    .unwrap_or_else(|| panic!("{name}: missing {function}"));
                let body = llvm.split_once(definition).unwrap().1.split_once("\n}").unwrap().0;
                if rebound {
                    let after_rebind = body.rsplit_once(&format!("__{}(", hex("carry"))).unwrap().1;
                    for (owner, index) in [("left", "1"), ("right", "0")] {
                        let extent_ssa = after_rebind.lines().find_map(|line| line
                            .split_once(&format!(" = load i64, ptr %{owner}_length,"))
                            .map(|(value, _)| value.trim()))
                            .unwrap_or_else(|| panic!("{name}: missing {owner} extent snapshot"));
                        let call = format!("{guard}(i64 {index}, i64 {extent_ssa})");
                        let location = after_rebind.find(&call).expect("check uses actual retained length slot");
                        assert!(after_rebind[location..].contains("getelementptr inbounds i8"));
                    }
                } else {
                    let count_symbol = format!("__{}(", hex("construction_count"));
                    assert_eq!(body.matches(&count_symbol).count(), 1);
                    let original_count = body.lines().find(|line| line.contains(&count_symbol))
                        .unwrap().split_once(" = call i64 ").unwrap().0.trim();
                    let lines = body.lines().map(str::trim).collect::<Vec<_>>();
                    let unique_line = |message, predicate: &dyn Fn(&str) -> bool| {
                        let matches = lines.iter().enumerate().filter(|(_, line)| predicate(line))
                            .collect::<Vec<_>>();
                        assert_eq!(matches.len(), 1, "{message}: {body}");
                        (matches[0].0, *matches[0].1)
                    };
                    let (count_at, _) = unique_line("construction count", &|line| line.contains(&count_symbol));
                    let alloc = format!(" = call ptr @__wosy_core_alloc(i64 {original_count}, i64 1)");
                    let (backing_at, allocation) = unique_line("allocation of original count", &|line| line.ends_with(&alloc));
                    let backing = allocation.strip_suffix(&alloc).unwrap();
                    let selector_symbol = format!("__{}(", hex("select_index"));
                    let (selector_at, selector) = unique_line("effectful selector", &|line| {
                        line.contains(" = call i64 @") && line.contains(&selector_symbol)
                    });
                    let selection = selector.split_once(" = call i64 @").unwrap().0;
                    let guard_call = format!("{guard}(i64 {selection}, i64 {original_count})");
                    let (guard_at, _) = unique_line("full-width guard of selector result", &|line| {
                        line.starts_with("call void @") && line.ends_with(&guard_call)
                    });
                    let gep_operands = format!(" = getelementptr inbounds i8, ptr {backing}, i64 {selection}");
                    let (gep_at, _) = unique_line("indexed GEP from same backing and selector result", &|line| {
                        line.ends_with(&gep_operands)
                    });
                    assert!(count_at < backing_at && backing_at < selector_at && selector_at < guard_at
                        && guard_at < gep_at, "{name}: {body}");
                    let checks = body.lines().filter(|line| line.contains(&guard)).collect::<Vec<_>>();
                    assert_eq!(checks.len(), 2);
                    assert!(checks.iter().all(|line| line.ends_with(&format!(", i64 {original_count})"))));
                    assert_eq!(body.matches("call void @__wosy_core_free(").count(), 1);
                }
            }
            fs::remove_dir_all(&project.root).unwrap();
        }
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
            assert_eq!(current_source_observation(&configuration, &observations[1]).unwrap(), observations[1]);
            assert!(!plain.contains("636865636b5f696e6465785f696e5f657874656e74"));
            assert!(linked.lines().any(|line| line.starts_with("define void ")
                && line.contains("636865636b5f696e6465785f696e5f657874656e74")));
            assert!(linked.contains("ret i64 42"));
            let mut stale = observations[1].clone();
            stale.content_revision = ContentRevision("old-source-or-contract".to_owned());
            assert_ne!(current_source_observation(&configuration, &stale).unwrap(), stale);
        }
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[test]
    fn lookalike_user_namespace_cannot_supply_the_release_checked_access_source() {
        let (mut project, configuration) = routing_test_project(
            "release-access-lookalike",
            &[("main.w", "%%start\nuser = namespace wosy_toolchain_stdlib \"src/checked_access.w\";\nuser.check_index_in_extent(2, 3);\nu8(u64) read = fn(index) { u8[3] values = [7, 8, 9]; values[index] };\nread(2);\n%%end")],
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
        assert_eq!(current_source_observation(&configuration, release).unwrap(), **release);
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
        let (migrated, migrated_sources) = compile_project(
            &project.root, &project, &configuration, ScalarTargetLayout::NATIVE64,
        ).unwrap();
        assert_eq!(migrated_sources, observations);
        let release_revision = release.content_revision.0.bytes()
            .map(|byte| format!("{byte:02x}")).collect::<String>();
        let migrated_calls = migrated.lines().filter(|line| line.contains("call void @")
            && line.contains(symbol_suffix)).collect::<Vec<_>>();
        assert_eq!(migrated_calls.len(), 2);
        assert_eq!(migrated_calls.iter().filter(|call| call.contains(&user_revision)).count(), 1);
        assert_eq!(migrated_calls.iter().filter(|call| call.contains(&release_revision)).count(), 1);
        fs::remove_dir_all(&project.root).unwrap();
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
            compile_project_with_root_exports(
                &project.root,
                &project,
                &configuration,
                ScalarTargetLayout::WASM32,
                exports,
            )
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
            compile_project_with_root_exports(
                &project.root,
                &project,
                &configuration,
                ScalarTargetLayout::WASM32,
                exports,
            )
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
    fn configured_wasm_root_exports_preserve_dev4_language_admission() {
        let root = "%%start\ni32(i32) score = fn(points) { points };\n";
        let fixture = format!("{root}%%end");
        let (project, configuration) =
            routing_test_project("wasm-root-exports-language-admission", &[("main.w", &fixture)]);
        let exports = BTreeMap::from([("score".into(), "score".into())]);
        let compile = |exports| {
            compile_project_with_root_exports(
                &project.root, &project, &configuration, ScalarTargetLayout::WASM32, exports,
            )
        };
        assert_eq!(compile(Some(&exports)).unwrap(), compile(None).unwrap());

        // Named Dev3 proof declarations are not part of this Dev4 baseline.
        // Selecting an export must not introduce another frontend or admit them.
        let proof = "proof impossible for score(points) -> (out) ensures (false) {}";
        fs::write(project.root.join("main.w"), format!("{root}{proof}\n%%end")).unwrap();
        for configured in [None, Some(&exports)] {
            assert_eq!(compile(configured).unwrap_err(), "build rejected by syntax diagnostics");
        }

        // Supported syntax still goes through the ordinary type checker.
        fs::write(project.root.join("main.w"), fixture.replace("{ points }", "{ points == 0 }")).unwrap();
        for configured in [None, Some(&exports)] {
            assert_eq!(compile(configured).unwrap_err(), "build rejected by semantic diagnostics");
        }
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[test]
    fn configured_web_export_keeps_checked_release_leaf_and_guarded_owner() {
        let text = r#"%%start
u64(u64) observe = fn(index) {
    u8[3] cells = [7, 8, 9];
    u64 amount = core.int_extend<u64>(cells[index]);
    u64 held =? amount;
    if (&held == null) { 0 } else { held }
};
unit() stop = fn { core.system_panic(); };
%%end"#;
        let (project, configuration) =
            routing_test_project("web-checked-release-root", &[("main.w", text)]);
        let exports = BTreeMap::from([
            ("observe_value".into(), "observe".into()),
            ("stop_now".into(), "stop".into()),
        ]);
        let ((partition, observations), counts) = routing_observations::observe(|| {
            compile_project_partition(
                &project.root, &project, &configuration, ScalarTargetLayout::WASM32, Some(&exports),
            ).expect("canonical checked-access project with exports")
        });
        let llvm = partition.to_text();
        let [root, release] = observations.as_slice() else { panic!("{observations:?}") };
        assert_ne!(root.source.project, release.source.project);
        assert_eq!((counts.preparations, counts.builds, counts.project_emissions, counts.single_module_emissions),
            (1, 1, 1, 0));
        assert_wasm_root_forwarder(&llvm, "observe_value", "i64", &["i64"]);
        assert!(llvm.contains("define void @stop_now("));
        let symbol = |name: &str| name.bytes().map(|byte| format!("{byte:02x}")).collect::<String>();
        let bounds = format!("__{}__{}(", symbol(&release.content_revision.0), symbol("check_index_in_extent"));
        assert_eq!(llvm.lines().filter(|row| row.contains("call void @") && row.contains(&bounds)).count(), 1);
        assert_eq!(llvm.matches("call ptr @__wosy_core_alloc(i64 8, i64 8)").count(), 1);
        assert_eq!(llvm.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1);
        let (baseline, baseline_observations) =
            compile_project(&project.root, &project, &configuration, ScalarTargetLayout::WASM32).unwrap();
        assert_eq!(baseline_observations, observations);
        assert!(!baseline.contains("@observe_value") && !baseline.contains("@stop_now"));
        let directory = std::env::var_os("WOSY_CONFIGURED_WEB_IR_DIR");
        let path = directory.as_ref().map_or_else(|| project.root.join("partition.ll"),
            |directory| std::path::PathBuf::from(directory).join("partition.ll"));
        let written_identity = write_llvm_partition(&path, &partition).unwrap();
        assert_eq!(fs::read(&path).unwrap(), llvm.as_bytes());
        assert_eq!(written_identity, blake3::hash(llvm.as_bytes()).to_hex().as_str());
        if let Some(directory) = directory {
            fs::write(std::path::PathBuf::from(directory).join("runtime.ll"),
                wosy_compiler::core_runtime("llvm", "wasm-web").unwrap()).unwrap();
        }
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[test]
    fn writes_partition_and_hashes_exact_canonical_bytes() {
        let sources = [(
            "main.w",
            "%%start\nenv = extern wasm \"host\" { unit() log; };\nenv.log();\n%%end\n",
        )];
        let (project, configuration) = routing_test_project("streamed-partition", &sources);
        let (partition, observations) = compile_project_partition(
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
        let snapshot = fresh.snapshot_json_serializable();
        let borrowed = serde_json::to_vec_pretty(&snapshot)
            .expect("serialize borrowed CST snapshot");
        let owned = serde_json::to_vec_pretty(&tree.snapshot())
            .expect("serialize owned CST snapshot");
        assert!(borrowed == owned, "borrowed CST snapshot changed bytes");
        persist_cst(&root, &fresh.result().source.revision, &snapshot).expect("publish archived snapshot");
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
        let output = wosy_compiler::parse_source(source, "%%start\n%%end".into(), &[]);
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
