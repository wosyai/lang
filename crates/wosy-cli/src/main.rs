use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic as Report, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
#[cfg(test)]
use serde::Deserialize;
use std::collections::BTreeMap;
#[cfg(feature = "internal-checker")]
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufWriter, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::process::ExitCode;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
use wosy_compiler::{
    analyze_scalar_build, emit_scalar_build_project_artifact_llvm,
    emit_scalar_build_single_module_artifact_llvm, emit_scalar_build_wasm_root_exports_artifact_llvm,
    parse_source_with_snapshot, prepare_scalar_project, publication, serialize_publication, LlvmPartition, ParseOutput,
    ScalarPreparationInputs, ScalarTargetLayout,
};
#[cfg(feature = "internal-checker")]
use wosy_compiler::{ScalarCheckerScope, ScalarNamespaceBinding};
use wosy_project::{
    artifact_manifest_path, discover_root, load_artifact_manifest,
    publish_artifact, ArtifactIdentity, ContentRevision, PackageRelativePath,
    ProjectConfiguration, ProjectContext, SourceIdentity as ProjectSourceIdentity,
    SourceObservation,
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
    /// Check the selected target and unused functions in the current package
    #[cfg(feature = "internal-checker")]
    #[command(group(clap::ArgGroup::new("check_scope").args(["main_flow_only", "declared_package"])))]
    Check {
        target: Option<String>,
        #[arg(long, default_value = "dev")]
        profile: String,
        #[arg(long, help = "Check only the selected target's main flow")]
        main_flow_only: bool,
        #[arg(long, help = "Check the current package (the default scope)")]
        declared_package: bool,
        #[arg(long, help = "Also check each resolved dependency's independent primary target")]
        dependencies: bool,
        #[cfg(feature = "private-check-cap-process")]
        #[arg(long, hide = true)]
        max_errors: Option<std::num::NonZeroUsize>,
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
        #[cfg(feature = "internal-checker")]
        Command::Check {
            target, profile, main_flow_only, dependencies,
            #[cfg(feature = "private-check-cap-process")]
            max_errors,
            ..
        } => {
            // Clap rejects both scope flags. An omitted scope is exactly the explicit package scope.
            let scope = if main_flow_only {
                ScalarCheckerScope::MainFlowOnly
            } else {
                ScalarCheckerScope::CurrentPackage
            };
            let run_incumbent = || check_command(target.as_deref(), &profile, scope, dependencies);
            #[cfg(not(feature = "private-check-cap-process"))]
            let result = run_incumbent();
            #[cfg(feature = "private-check-cap-process")]
            let result = match max_errors {
                None => run_incumbent(),
                Some(limit) => check_and_publish(scope, dependencies, |root| {
                    let load = if dependencies {
                        configured_check::check_targets_with_private_authored_limit
                    } else {
                        configured_check::check_target_with_private_authored_limit
                    };
                    load(root, target.as_deref(), &profile, scope, limit)
                }),
            };
            match result {
                Ok(code) => ExitCode::from(code),
                Err(error) => {
                    eprintln!("wosy: {error}");
                    ExitCode::from(2)
                }
            }
        }
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

mod canonical_loader;
#[cfg(feature = "internal-checker")]
#[allow(dead_code)]
mod configured_check;
#[cfg(feature = "internal-checker")]
use configured_check::{InternalCheckEffect, InternalCheckStatus, OrderedCheckEffects};

use canonical_loader::{
    CanonicalLoadFailure, CanonicalLoadPurpose, CanonicalProjectInputs,
};
#[cfg(feature = "internal-checker")]
use canonical_loader::{canonical_source_identity, parse_canonical_ordinary_source};
use canonical_loader::compiler_source_identity;

fn load_canonical_project_inputs(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    purpose: CanonicalLoadPurpose,
) -> Result<CanonicalProjectInputs, CanonicalLoadFailure> {
    canonical_loader::load_canonical_project_inputs(root, project, configuration, purpose, || {
        #[cfg(test)]
        routing_observations::record(|counts| counts.extractions += 1);
    })
}

impl CanonicalLoadFailure {
    fn report(self, purpose: &str) -> String {
        match self {
            Self::Input(error) => error,
            Self::Syntax { source, text, diagnostics } => {
                render_diagnostics(&text, &diagnostics, &source);
                format!("{purpose} rejected by syntax diagnostics")
            }
            Self::Derivation { diagnostics, texts } => {
                render_project_diagnostics(&diagnostics, &texts);
                format!("{purpose} rejected by CST derivation diagnostics")
            }
            Self::Graph { error, texts } => {
                render_graph_diagnostics(&error, &texts);
                format_graph_error(error)
            }
        }
    }
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
impl InternalCheckReceipt<wosy_compiler::ScalarOrderedCheckerOutcome> {
    fn effects(&self) -> Option<OrderedCheckEffects<'_>> {
        use wosy_compiler::ScalarOrderedCheckerOutcome;
        match &self.outcome {
            ScalarOrderedCheckerOutcome::Passed | ScalarOrderedCheckerOutcome::NoChecks =>
                Some(OrderedCheckEffects::new(&[])),
            ScalarOrderedCheckerOutcome::Failed(reports) => Some(OrderedCheckEffects::new(reports)),
            ScalarOrderedCheckerOutcome::Unsupported | ScalarOrderedCheckerOutcome::Invalid(_) => None,
        }
    }

    fn selected_status(&self) -> Option<InternalCheckStatus<'_>> {
        self.effects().map(OrderedCheckEffects::selected_status)
    }

    fn visit_authored_effects<E>(
        &self,
        sink: impl FnMut(InternalCheckEffect<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        match self.effects() {
            Some(effects) => effects.visit_authored_effects(sink),
            None => Ok(()),
        }
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
    let boundaries = canonical_loader::DormantSourceBoundaries::new(root, project, configuration);
    let mut selected = HashSet::with_capacity(sources.len());
    for DormantSourceInput { identity, .. } in &sources {
        boundaries.validate(identity)?;
        let spelling = identity.path.as_path().display();
        if texts.contains_key(identity) || !selected.insert(identity) {
            return Err(format!("dormant source was already selected: {spelling}").into());
        }
    }

    sources.into_iter().map(|DormantSourceInput { identity, text }| {
        let source = canonical_source_identity(identity.project.clone(), &identity.package, &identity.path, &text);
        let (text, declarations) = parse_canonical_ordinary_source(
            source, text, &project.comment_categories, false, true, None,
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
    root_exports: Option<&BTreeMap<String, String>>,
) -> Result<(LlvmPartition, Vec<SourceObservation>), String> {
    compile_project_with_persistence(
        root, project, configuration, target_layout, Some(persist_fresh_cst), root_exports,
    )
}

fn compile_project_with_persistence(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target_layout: ScalarTargetLayout,
    persist: Option<canonical_loader::PersistCst>,
    root_exports: Option<&BTreeMap<String, String>>,
) -> Result<(LlvmPartition, Vec<SourceObservation>), String> {
    let CanonicalProjectInputs {
        declarations,
        initialization_order,
        graph,
        texts,
        ..
    } = load_canonical_project_inputs(root, project, configuration, CanonicalLoadPurpose::Build(persist))
        .map_err(|failure| failure.report("build"))?;
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
    let llvm = if let Some(exports) = root_exports {
        let entry = graph.nodes.first().ok_or_else(|| "project root source is missing".to_owned())?;
        let source = compiler_source_identity(&entry.source, &entry.content_revision);
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

fn persist_fresh_cst(
    root: &std::path::Path,
    fresh: &wosy_syntax::FreshParseResult,
) -> Result<(), String> {
    persist_cst(root, &fresh.result().source.revision, &fresh.snapshot_json_serializable())
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
    render_project_diagnostics_with_packages(diagnostics, texts, false);
}

fn render_project_diagnostics_with_packages(
    diagnostics: &[wosy_compiler::Diagnostic], texts: &BTreeMap<ProjectSourceIdentity, String>, packages: bool,
) {
    let stream = StandardStream::stderr(ColorChoice::Auto);
    render_project_diagnostics_to(&mut stream.lock(), diagnostics, texts, packages);
}

fn render_project_diagnostics_to(
    stderr: &mut impl codespan_reporting::term::termcolor::WriteColor,
    diagnostics: &[wosy_compiler::Diagnostic], texts: &BTreeMap<ProjectSourceIdentity, String>, packages: bool,
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
                let name = if packages { std::borrow::Cow::Owned(format!("{}::{}", label.span.source.package, label.span.source.path)) }
                    else { std::borrow::Cow::Borrowed(label.span.source.path.as_str()) };
                let file_id = files.add(name, text.as_str());
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
        let mut report = Report::error()
            .with_message(diagnostic.message.clone())
            .with_labels(labels)
            .with_notes(diagnostic.notes.clone());
        if !diagnostic.code.is_empty() {
            report = report.with_code(diagnostic.code.clone());
        }
        let config = term::Config::default();
        let _ = term::emit(stderr, &config, &files, &report);
    }
}

#[cfg(feature = "internal-checker")]
fn check_command(
    target: Option<&str>, profile: &str, scope: ScalarCheckerScope, dependencies: bool,
) -> Result<u8, String> {
    check_and_publish(scope, dependencies, |root| {
        if dependencies {
            configured_check::check_target_with_dependencies(root, target, profile, scope)
        } else {
            match scope {
                ScalarCheckerScope::MainFlowOnly => configured_check::check_target(root, target, profile, scope, Vec::new()),
                ScalarCheckerScope::CurrentPackage => configured_check::check_declared_package_target(root, target, profile),
            }.map(|receipt| vec![receipt])
        }
    })
}

#[cfg(feature = "internal-checker")]
fn check_and_publish(
    scope: ScalarCheckerScope, dependencies: bool,
    load: impl FnOnce(&std::path::Path) -> Result<Vec<configured_check::ConfiguredCheckReceipt>, configured_check::ConfiguredCheckLoadError>,
) -> Result<u8, String> {
    use configured_check::ConfiguredCheckLoadError as LoadError;

    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let receipts = load(&root).map_err(|failure| match failure {
            LoadError::Canonical(failure) | LoadError::Dormant(failure) => failure.report("check"),
            LoadError::Root => "check: invalid project root".to_owned(),
            LoadError::Manifest(error) => format!("check: invalid manifest: {error}"),
            LoadError::Target => "check: invalid target".to_owned(),
            LoadError::Profile => "check: invalid target profile".to_owned(),
            LoadError::Layout => "check: unsupported target layout".to_owned(),
            LoadError::Dependency(error) => format!("check: {error}"),
            LoadError::PackageSourceRoot(error) => format!("check: {error}"),
            LoadError::DormantPath => "check: invalid declared source path".to_owned(),
        })?;
    let stream = StandardStream::stderr(ColorChoice::Auto);
    let mut stderr = stream.lock();
    publish_configured_check_receipts(&receipts, scope, dependencies,
        &mut std::io::stdout().lock(), &mut stderr)
}

#[cfg(feature = "internal-checker")]
fn publish_configured_check_receipts(
    receipts: &[configured_check::ConfiguredCheckReceipt], scope: ScalarCheckerScope, dependencies: bool,
    stdout: &mut impl std::io::Write,
    stderr: &mut impl codespan_reporting::term::termcolor::WriteColor,
) -> Result<u8, String> {
    use configured_check::{CheckPublicationError, CheckPublicationPreflightError};
    use wosy_compiler::ScalarOrderedCheckerOutcome;
    let unsupported_output = || "check: result requires unsupported output; no result published".to_owned();
    let primary = receipts.first().ok_or_else(unsupported_output)?;
    let checked = match checked_publication_reports(receipts) {
        Ok(checked) => checked,
        Err(receipt) => match &receipt.outcome {
            Err(ScalarOrderedCheckerOutcome::Invalid(diagnostics)) => {
                render_project_diagnostics_to(stderr, diagnostics, &receipt.texts, dependencies);
                return Ok(2);
            }
            _ => return Err("check: unsupported analysis; no result published".to_owned()),
        },
    };
    let publication = if receipts.len() == 1 {
        primary.plan_publication()
    } else {
        configured_check::plan_publication_for(checked[0], &primary.texts, checked.iter().copied())
    }.map_err(|error| match error {
        CheckPublicationError::Incomplete(_) => "check: unsupported analysis; no result published".to_owned(),
        CheckPublicationError::InvalidPath { .. } | CheckPublicationError::ConflictingPath { .. } => unsupported_output(),
    })?;
    if scope == ScalarCheckerScope::MainFlowOnly
        && (publication.report.symbolic_diagnostic.is_some() || !publication.report.additional_symbolic_diagnostics.is_empty())
    {
        return Err(unsupported_output());
    }
    let diagnostics = checked.iter().zip(receipts)
        .map(|(report, receipt)| ordered_check_diagnostics(report, &receipt.texts))
        .collect::<Result<Vec<_>, _>>()?;
    let authored = publication.prepare(&primary.root).map_err(|error| match error {
        CheckPublicationPreflightError::UnsupportedFilePublication => unsupported_output(),
        CheckPublicationPreflightError::File(error) =>
            format!("check: could not prepare authored files; no result published: {error}"),
        CheckPublicationPreflightError::ExitStatusOutsideByteRange =>
            "check: selected exit code is outside 0..=255; no result published".to_owned(),
    })?;
    let authored_status = authored.publish(stdout, stderr)
        .map_err(|error| format!("check: could not complete authored output; earlier records may already be published: {error}"))?;
    let mut has_failure = false;
    for (diagnostics, receipt) in diagnostics.iter().zip(receipts) {
        has_failure |= !diagnostics.is_empty();
        render_project_diagnostics_to(stderr, diagnostics, &receipt.texts, dependencies);
    }
    Ok(if authored_status != 0 { authored_status } else { u8::from(has_failure) })
}

#[cfg(feature = "internal-checker")]
fn checked_publication_reports(
    receipts: &[configured_check::ConfiguredCheckReceipt],
) -> Result<Vec<&wosy_compiler::MainFlowProofReceipt>, &configured_check::ConfiguredCheckReceipt> {
    receipts.iter().map(|receipt| receipt.outcome.as_ref().map_err(|_| receipt)).collect()
}

#[cfg(feature = "internal-checker")]
fn ordered_check_diagnostics(
    report: &wosy_compiler::MainFlowProofReceipt,
    texts: &BTreeMap<ProjectSourceIdentity, String>,
) -> Result<Vec<wosy_compiler::Diagnostic>, String> {
    use wosy_compiler::{Diagnostic, DiagnosticLabel, DiagnosticLabelKind, DiagnosticSeverity};
    let actual = report.diagnostics.iter().map(|failure| {
        let mut labels = vec![
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Primary,
                span: failure.failed_step.clone().unwrap_or_else(|| failure.goal.clone()),
                message: "not satisfied by this call".to_owned(),
            },
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.proof.clone(),
                message: "proof declaration".to_owned(),
            },
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.invocation.call.clone(),
                message: "applicable call".to_owned(),
            },
        ];
        if failure.failed_step.is_some() {
            labels.push(DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.goal.clone(),
                message: "proof goal".to_owned(),
            });
        }
        labels.extend(failure.invocation.intermediate_calls.iter().map(|span| DiagnosticLabel {
            kind: DiagnosticLabelKind::Secondary,
            span: span.clone(),
            message: "through this call".to_owned(),
        }));
        if let Some(outer) = &failure.invocation.outer_call {
            labels.push(DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: outer.clone(),
                message: "reached from this call".to_owned(),
            });
        }
        if let Some(owner) = &failure.invocation.namespaced_callable {
            labels.push(DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary, span: owner.clone(),
                message: "proved callable declared here".to_owned(),
            });
        }
        let value = proof_scalar_text(&failure.invocation.returned, failure.invocation.returned_literal);
        let mut notes = vec![if let Some(pair) = &failure.invocation.returned_pair {
            format!("call returned {}", proof_pair_text(pair))
        } else if let Some(slot) = failure.invocation.result_slot {
            format!("call returned output {} = {value}", slot + 1)
        } else { format!("call returned {value}") }];
        if let Some((name, value)) = &failure.quantified_witness {
            notes.push(format!("quantified witness input: {name} = {value}"));
        }
        if (failure.entry_dependent_witness || failure.invocation.outer_call.is_some()
            || failure.invocation.result_slot.is_some() || failure.invocation.returned_pair.is_some()
            || matches!(failure.invocation.returned_literal, Some(wosy_compiler::ScalarCheckerProofLiteral::Character(_))))
            && !failure.invocation.arguments.is_empty()
        {
            let entries = failure.invocation.arguments.iter().enumerate().map(|(index, (ty, value))| {
                format!("argument {} = {}", index + 1, proof_scalar_text(value, wosy_compiler::ScalarCheckerProofLiteral::from_atom(ty, value)))
            }).collect::<Vec<_>>();
            notes.push(format!("call entry: {}", entries.join(", ")));
        }
        Diagnostic {
            code: String::new(), severity: DiagnosticSeverity::Error,
            message: "proof obligation failed".to_owned(), labels, notes,
        }
    });
    let symbolic = report.symbolic_diagnostic.iter().chain(&report.additional_symbolic_diagnostics).map(|failure| {
        let mut labels = vec![
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Primary,
                span: failure.failed_step.clone().unwrap_or_else(|| failure.goal.clone()),
                message: "not satisfied by this symbolic call".to_owned(),
            },
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.proof.clone(),
                message: "proof declaration".to_owned(),
            },
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.callable.clone(),
                message: "unused function checked symbolically".to_owned(),
            },
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.output.clone(),
                message: "return with a counterexample".to_owned(),
            },
        ];
        if let Some(output) = &failure.other_output {
            labels.push(DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: output.clone(),
                message: "other output from this return".to_owned(),
            });
        }
        if failure.failed_step.is_some() {
            labels.push(DiagnosticLabel {
                kind: DiagnosticLabelKind::Secondary,
                span: failure.goal.clone(),
                message: "proof goal".to_owned(),
            });
        }
        let value = proof_scalar_text(&failure.returned, failure.returned_literal);
        let mut notes = vec![if let Some(pair) = &failure.returned_pair {
            format!("symbolic call can return {}", proof_pair_text(pair))
        } else if let Some(slot) = failure.result_slot {
            format!("symbolic call can return output {} = {value}", slot + 1)
        } else { format!("symbolic call can return {value}") }];
        if let Some((name, value)) = &failure.quantified_witness {
            notes.push(format!("quantified witness input: {name} = {value}"));
        }
        if let Some(ieee) = &failure.ieee_entry {
            let name = entry_binding_name(&ieee.binding.source, ieee.binding.range, texts).unwrap_or("argument 1");
            use wosy_compiler::ScalarCheckerIeeeValue;
            let detail = match ieee.value {
                ScalarCheckerIeeeValue::F32(bits) => format!("{:?} (f32 bits 0x{bits:08x})", f32::from_bits(bits)),
                ScalarCheckerIeeeValue::F64(bits) => format!("{:?} (f64 bits 0x{bits:016x})", f64::from_bits(bits)),
            };
            notes.push(format!("symbolic call entry: {name} = {detail}"));
        } else if let Some(entry) = entry_point_note(&failure.witness, texts, false, failure.returned_literal.is_some()) {
            notes.push(entry);
        }
        Diagnostic {
            code: String::new(), severity: DiagnosticSeverity::Error,
            message: "proof obligation failed".to_owned(), labels,
            notes,
        }
    });
    let contextual = report.contextual_diagnostic.iter().chain(&report.additional_contextual_diagnostics).map(|failure| {
        let mut labels = vec![
            DiagnosticLabel { kind: DiagnosticLabelKind::Primary, span: failure.goal.clone(),
                message: if failure.symbolic { "not satisfied by this symbolic contextual call" }
                    else { "not satisfied by this contextual call" }.to_owned() },
            DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.proof.clone(),
                message: "proof declaration".to_owned() },
        ];
        if failure.symbolic {
            labels.push(DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.call.clone(),
                message: "unused callable checked with an arbitrary contextual entry".to_owned() });
        } else {
            labels.extend([
                DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.supplying_call.clone(),
                    message: "call supplying this context".to_owned() },
                DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.call.clone(),
                    message: "checked call".to_owned() },
                DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.binding.clone(),
                    message: "active contextual binding".to_owned() },
            ]);
        }
        let mut notes = vec![format!("contextual value at entry: {}; at exit: {}", failure.entry, failure.exit)];
        if let Some((name, value)) = &failure.explicit_entry {
            let origin = if failure.symbolic { "symbolic" } else { "call" };
            notes.push(format!("explicit {origin} entry: {name} = {value}"));
        }
        Diagnostic {
            code: String::new(), severity: DiagnosticSeverity::Error,
            message: "proof obligation failed".to_owned(), labels, notes,
        }
    });
    let aggregate = report.aggregate_diagnostics.iter().map(|failure| {
        let actual = failure.call.is_some();
        let (primary, output, note) = if let Some(cell) = &failure.cell {
            ("not satisfied by this returned array cell", "returned cell",
                if actual { format!("call returned cell [{}] = {}", cell.index, cell.value) }
                else { format!("symbolic call can return cell [{}] = {}", cell.index, cell.value) })
        } else {
            ("no index satisfies this existential goal", "returned array",
                if actual { "call returned an array with no matching index".to_owned() }
                else { "symbolic call can return an array with no matching index".to_owned() })
        };
        let mut labels = vec![
            DiagnosticLabel {
                kind: DiagnosticLabelKind::Primary,
                span: failure.failed_step.as_ref().unwrap_or(&failure.goal).clone(),
                message: primary.to_owned(),
            },
            DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.proof.clone(),
                message: "proof declaration".to_owned() },
            DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.output.clone(),
                message: output.to_owned() },
        ];
        if failure.failed_step.is_some() {
            labels.push(DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.goal.clone(),
                message: "proof goal".to_owned() });
        }
        if let Some(call) = &failure.call {
            labels.push(DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: call.clone(),
                message: "applicable call".to_owned() });
        }
        let mut notes = vec![note];
        if let Some((name, value)) = failure.cell.as_ref().and_then(|cell| cell.universal.as_ref()) {
            notes.push(format!("quantified witness input: {name} = {value}"));
        }
        if let Some(entry) = entry_point_note(&failure.witness, texts, actual, false) {
            notes.push(entry);
        }
        Diagnostic {
            code: String::new(), severity: DiagnosticSeverity::Error,
            message: "proof obligation failed".to_owned(), labels, notes,
        }
    });
    let pointer_effect = report.pointer_effect_diagnostic.iter().map(|failure| Diagnostic {
        code: String::new(), severity: DiagnosticSeverity::Error,
        message: "proof obligation failed".to_owned(),
        labels: vec![
            DiagnosticLabel { kind: DiagnosticLabelKind::Primary, span: failure.goal.clone(),
                message: "not satisfied by this symbolic pointee update".to_owned() },
            DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.proof.clone(),
                message: "proof declaration".to_owned() },
            DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.read.clone(),
                message: "first read of the entry pointee".to_owned() },
            DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: failure.write.clone(),
                message: "write determining the exit pointee".to_owned() },
        ],
        notes: std::iter::once(format!("pointee at entry: {}; at exit: {}; goal requires: {}",
            failure.entry, failure.observed, failure.expected))
            .chain(failure.guard.map(|value| format!("Boolean guard input at entry: {value}"))).collect(),
    });
    let mut diagnostics = Vec::new();
    if let Some(positions) = &report.mixed_context_after_scalar {
        let mut scalar = actual.chain(symbolic);
        let mut contextual = contextual;
        let mut emitted = 0;
        for &position in positions {
            if position < emitted { return Err("check: invalid mixed proof diagnostic order".to_owned()); }
            while emitted < position {
                diagnostics.push(scalar.next().ok_or_else(|| "check: incomplete mixed proof diagnostic order".to_owned())?);
                emitted += 1;
            }
            diagnostics.push(contextual.next().ok_or_else(|| "check: missing mixed contextual diagnostic".to_owned())?);
        }
        if contextual.next().is_some() { return Err("check: incomplete mixed contextual diagnostic order".to_owned()); }
        diagnostics.extend(scalar);
    } else {
        diagnostics.extend(actual);
        diagnostics.extend(symbolic);
        diagnostics.extend(contextual);
    }
    diagnostics.extend(aggregate);
    diagnostics.extend(pointer_effect);
    Ok(diagnostics)
}

#[cfg(feature = "internal-checker")]
fn proof_scalar_text(value: impl std::fmt::Display, literal: Option<wosy_compiler::ScalarCheckerProofLiteral>) -> String {
    use wosy_compiler::ScalarCheckerProofLiteral::{Boolean, Character};
    match literal {
        Some(Boolean(value)) => value.to_string(),
        Some(Character(value)) => format!("{value:?}"),
        None => value.to_string(),
    }
}

#[cfg(feature = "internal-checker")]
fn proof_pair_text<T: std::fmt::Display>([(left, left_literal), (right, right_literal)]: &[(T, Option<wosy_compiler::ScalarCheckerProofLiteral>); 2])
    -> String
{
    format!("(output 1 = {}, output 2 = {})", proof_scalar_text(left, *left_literal), proof_scalar_text(right, *right_literal))
}

#[cfg(feature = "internal-checker")]
fn entry_binding_name<'a>(
    binding_source: &SourceIdentity,
    declaration: wosy_syntax::ByteSpan,
    texts: &'a BTreeMap<ProjectSourceIdentity, String>,
) -> Option<&'a str> {
    PackageRelativePath::new(binding_source.path.clone().into()).ok()
        .map(|path| ProjectSourceIdentity::new(binding_source.project.clone(), binding_source.package.clone(), path))
        .and_then(|source| texts.get(&source))
        .and_then(|text| text.get(declaration.start as usize..declaration.end as usize))
}

#[cfg(feature = "internal-checker")]
fn entry_point_note(
    witness: &wosy_compiler::ScalarCheckerIntegerWitness,
    texts: &BTreeMap<ProjectSourceIdentity, String>,
    actual: bool, categorical: bool,
) -> Option<String> {
    if witness.row_count() != 1 || witness.axes().len() == 0 { return None; }
    let entries = witness.axes().enumerate().map(|(axis, (binding, ty))| {
        let mut points = witness.intervals_at(0, axis)?;
        let (first, last) = points.next()?;
        if first != last || points.len() != 0 { return None; }
        let name = entry_binding_name(&binding.source, binding.declaration_span, texts);
        let name = name.map_or_else(|| format!("argument {}", axis + 1), str::to_owned);
        let literal = if categorical { wosy_compiler::ScalarCheckerProofLiteral::from_atom(ty, first) } else { None };
        Some(format!("{name} = {}", proof_scalar_text(first, literal)))
    }).collect::<Option<Vec<_>>>()?;
    let origin = if actual { "call entry" } else { "symbolic call entry" };
    Some(format!("{origin}: {}", entries.join(", ")))
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
        observe(|| {
            compile_project(&project.root, project, configuration, ScalarTargetLayout::NATIVE64, None)
                .map(|(llvm, sources)| (llvm.to_text(), sources))
        })
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

    fn compile_project_text(
        root: &std::path::Path,
        project: &ProjectContext,
        configuration: &ProjectConfiguration,
        target_layout: ScalarTargetLayout,
        root_exports: Option<&BTreeMap<String, String>>,
    ) -> Result<(String, Vec<SourceObservation>), String> {
        compile_project(root, project, configuration, target_layout, root_exports)
            .map(|(llvm, sources)| (llvm.to_text(), sources))
    }

    fn compile_project_text_with_persistence(
        root: &std::path::Path,
        project: &ProjectContext,
        configuration: &ProjectConfiguration,
        target_layout: ScalarTargetLayout,
        persist: Option<canonical_loader::PersistCst>,
        root_exports: Option<&BTreeMap<String, String>>,
    ) -> Result<(String, Vec<SourceObservation>), String> {
        compile_project_with_persistence(root, project, configuration, target_layout, persist, root_exports)
            .map(|(llvm, sources)| (llvm.to_text(), sources))
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

    #[test]
    fn canonical_build_statically_types_then_erases_proofs_without_verifying_them() {
        let erased = r#"%%start
            u8(u8) identity = fn(input) { input };
            unit() noop = fn {};
            (u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };
            generic T;
            T(T) generic_identity = fn(value) { value };
            single = overload { u8(u8) => fn(value) { value }; };
            choose = overload { u8(u8) => fn(value) { value }; bool(bool) => fn(value) { value }; };
        %%end"#;
        let (project, configuration) = routing_test_project("build-statically-types-proofs", &[("main.w", erased)]);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let build_without_persistence = || compile_project_text_with_persistence(
                &project.root, &project, &configuration, layout, None, None,
            );
            let build = || compile_project_text(&project.root, &project, &configuration, layout, None);
            std::fs::write(project.root.join("main.w"), erased).unwrap();
            let (baseline, _) = build().expect("runtime-only baseline");
            let with_proof = |proof: &str| erased.replace("%%end", &format!("{proof}\n%%end"));
            let false_proof = "proof impossible for identity(input) -> (out) ensures (out == core.old(input) && out != input) { core.show(false); }";
            let nullary = "proof empty for noop() -> () ensures (false) {}";
            let pair = "proof two for pair(input) -> (amount, flag) ensures (amount == 7 && flag == input) {}";
            let single = "proof arm for single(input) -> (out) ensures (out == input) {}";
            std::fs::write(project.root.join("main.w"),
                with_proof(&format!("{false_proof}\n{nullary}\n{pair}\n{single}"))).unwrap();
            let (result, counts) = routing_observations::observe(build);
            let (proved_erased, _) = result.expect("well-typed refuted proof still builds");
            assert_eq!(proved_erased.as_bytes(), baseline.as_bytes());
            assert_eq!(build_without_persistence().unwrap().0.as_bytes(), baseline.as_bytes());
            assert_eq!(counts, routing_observations::Counts {
                extractions: 1, preparations: 1, builds: 1, single_module_emissions: 1, ..Default::default()
            });
            for invalid in [
                false_proof.replace("core.old(input)", "unknown_name"),
                false_proof.replace("out == core.old(input) && out != input", "input"),
                false_proof.replace("core.show(false)", "core.show(unknown_step_name)"),
                false_proof.replace("for identity(input)", "for unknown_target(input)"),
                false_proof.replace("for identity(input)", "for generic_identity(input)"),
                false_proof.replace("for identity(input)", "for choose(input)"),
                pair.replace("(amount, flag)", "(amount)"),
                nullary.replace("noop()", "noop(arg)"),
            ] {
                std::fs::write(project.root.join("main.w"), with_proof(&invalid)).unwrap();
                let (rejected, counts) = routing_observations::observe(build);
                assert_eq!(rejected.unwrap_err(), "build rejected by semantic diagnostics", "{invalid}");
                assert_eq!(build_without_persistence().unwrap_err(), "build rejected by semantic diagnostics", "{invalid}");
                assert_eq!(counts, routing_observations::Counts { extractions: 1, preparations: 1, ..Default::default() });
            }
        }
    }

    #[test]
    fn configured_wasm_root_exports_keep_canonical_static_proof_boundary() {
        let root = "%%start\ni32(i32) score = fn(points) { points };\n";
        let proof = "proof impossible for score(points) -> (out) ensures (false) {}";
        let source = format!("{root}{proof}\n%%end");
        let (project, configuration) =
            routing_test_project("wasm-root-exports-keep-proof-boundary", &[("main.w", &source)]);
        let exports = BTreeMap::from([("demo_score".into(), "score".into())]);
        let compile = || {
            compile_project_text(
                &project.root, &project, &configuration, ScalarTargetLayout::WASM32, Some(&exports),
            )
        };
        let (with_proof, _) = compile().expect("well-typed proof is erased");
        assert!(with_proof.contains("define i32 @demo_score("));
        fs::write(project.root.join("main.w"), format!("{root}%%end")).unwrap();
        assert_eq!(compile().unwrap().0, with_proof);
        fs::write(
            project.root.join("main.w"),
            format!("{root}{}\n%%end", proof.replace("false", "unknown_name")),
        ).unwrap();
        assert_eq!(compile().unwrap_err(), "build rejected by semantic diagnostics");
        fs::remove_dir_all(&project.root).unwrap();
    }

    #[test]
    fn canonical_build_prepares_proof_against_inferred_callable_binding() {
        let text = r#"%%start
            u8(u8) identity = fn(input) { input };
            operation = identity;
            proof inferred for operation(input) -> (out) ensures (out == input) {}
        %%end"#;
        let (project, configuration) = routing_test_project("build-inferred-proof-target", &[("main.w", text)]);
        let load = || load_canonical_project_inputs(&project.root, &project, &configuration, CanonicalLoadPurpose::Build(None)).unwrap();
        let CanonicalProjectInputs { declarations, initialization_order, .. } = load();
        prepare_scalar_project(declarations, initialization_order,
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::NATIVE64 })
            .unwrap_or_else(|rejected| panic!("canonical inferred callable target: {:?}", rejected.into_ordered_diagnostics()));
        std::fs::write(project.root.join("main.w"), text.replace("out == input", "out == false")).unwrap();
        let CanonicalProjectInputs { declarations, initialization_order, .. } = load();
        let rejected = prepare_scalar_project(declarations, initialization_order,
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::NATIVE64 });
        let Err(rejected) = rejected else { panic!("inferred callable result must be typed"); };
        assert!(rejected.into_ordered_diagnostics().iter().any(|diagnostic| diagnostic.code == "B0003"));
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
            compile_project_text(&project.root, &project, &configuration, ScalarTargetLayout::NATIVE64, None)
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

        let (parent, document, selection) = ProjectConfiguration::load_with_direct_dependency_check_target(root, "dep", Some("own"), Some("checked")).unwrap();
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
            ProjectConfiguration::load_with_direct_dependency_check_target(root, "dep", Some("own"), Some("dev")).err().as_deref(),
            Some("target own profile dev is missing"),
        );
        use configured_check::{check_direct_dependency_primary, CheckPublicationError, ConfiguredCheckLoadError};
        let child_manifest = manifest("dep", "src", "own", "src/own.w", "checked", "wasm-wasip1");
        let explicit = "root = \"src/own.w\"";
        let selectors = format!("{explicit}\ndefault_profile = \"checked\"");
        let child_manifest = child_manifest.replace(explicit, &selectors);
        fs::write(dep.join("wosy.toml"), &child_manifest).unwrap();
        fs::write(dep.join("src/own.w"), r#"%%start
    i16(i16, i16) choose = fn(left, right) { if (right < 0) { left } else { right } };
    proof identity for choose(entry, selector) -> (out)
        where (entry >= -5) ensures (out == core.old(entry)) {}
    i16 answer = choose(4, 8);
    core.require(false, { .stdout = "child startup"; .stderr = ""; .code = 11;
        .files = [{ .path = "child.txt"; .text = "not yet published"; }]; });
%%end"#).unwrap();
        fs::write(dep.join("src/fallback.w"), r#"%%start
    u8(u8) fallback = fn(value) { value };
    proof every_return for fallback(entry) -> (out) ensures (out != 7) {}
%%end"#).unwrap();

        let (_, _, primary) = ProjectConfiguration::load_with_direct_dependency_check_target(root, "dep", None, None).unwrap();
        assert_eq!(primary.layout.pointer_size, 4);
        let checked = check_direct_dependency_primary(root, "dep").ok().expect("direct child input");
        let publication = checked.plan_publication().ok().expect("closed child proof scope");
        assert_eq!(checked.root, dep);
        assert_eq!(publication.report.diagnostics.len(), 1);
        assert_eq!(publication.report.diagnostics[0].invocation.call.source.path, "src/own.w");
        assert_eq!(publication.report.symbolic_diagnostic.as_ref().unwrap().proof.source.path, "src/fallback.w");
        assert_eq!(publication.report.directives.len(), 1);
        assert_eq!(publication.report.directives[0].stdout(), b"child startup");
        assert!(!root.join(".wosy").exists() && !dep.join(".wosy").exists());

        let unsupported = fs::read_to_string(dep.join("src/own.w")).unwrap().replace("core.old(entry)", "core.old(unknown)");
        fs::write(dep.join("src/own.w"), unsupported).unwrap();
        let incomplete = check_direct_dependency_primary(root, "dep").ok().expect("incomplete child proof");
        assert!(matches!(incomplete.plan_publication(), Err(CheckPublicationError::Incomplete(_))));
        assert!(!root.join(".wosy").exists() && !dep.join(".wosy").exists());
        fs::write(dep.join("wosy.toml"), child_manifest.replace(&selectors, explicit)).unwrap();
        assert!(matches!(check_direct_dependency_primary(root, "dep"), Err(ConfiguredCheckLoadError::Dependency(message))
            if message.contains("target own has no declared default_profile")));
        assert!(ProjectConfiguration::load_with_direct_dependency_check_target(root, "dep", Some("own"), Some("checked")).is_ok());
        assert!(!root.join(".wosy").exists() && !dep.join(".wosy").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn configured_dependency_check_uses_one_graph_and_one_ordered_publication() {
        use codespan_reporting::term::termcolor::Buffer;
        use configured_check::{check_target_with_dependencies, plan_publication_for, CheckPublicationError, ConfiguredCheckLoadError};
        let options = Arguments::try_parse_from(["wosy", "check", "--main-flow-only", "--dependencies"]).unwrap();
        assert!(matches!(options.command, Command::Check { main_flow_only: true, dependencies: true, .. }));
        let (scratch, _) = routing_test_project("configured-dependency-batch", &[]);
        let root = &scratch.root;
        let manifest = |package: &str, target: &str, profile: &str, output: &str| format!(r#"
[package]
name = "{package}"
default_target = "{target}"
source_root = "src"
[comments]
categories = []
[builders]
[runners]
[targets.{target}]
root = "src/primary.w"
default_profile = "{profile}"
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
        let app = manifest("app", "main", "dev", "native") + "\n[dependencies.a]\npath = \"a\"\n[dependencies.b]\npath = \"b\"\n";
        let body = |mark: &str, code: u8| format!(r#"%%start
core.require(false, {{ .stdout = "{mark}"; .stderr = ""; .code = {code}; .files = []; }});
%%end"#);
        for (package, content, code) in [
            ("", app.clone(), 3),
            ("a", manifest("a", "own_a", "checked", "wasm-wasip1") + "\n[dependencies.leaf]\npath = \"../leaf\"\n", 4),
            ("b", manifest("b", "own_b", "nativecheck", "native") + "\n[dependencies.leaf]\npath = \"../leaf\"\n", 5),
            ("leaf", manifest("leaf", "own_leaf", "leafcheck", "wasm-wasip1"), 6),
        ] {
            let directory = root.join(package);
            fs::create_dir_all(directory.join("src")).unwrap();
            fs::write(directory.join("wosy.toml"), content).unwrap();
            fs::write(directory.join("src/primary.w"), body(if package.is_empty() { "app" } else { package }, code)).unwrap();
        }
        use std::num::NonZeroUsize;
        let private = |cap| configured_check::check_targets_with_private_authored_limit(
            root, None, "dev", ScalarCheckerScope::MainFlowOnly, NonZeroUsize::new(cap).unwrap(),
        ).ok().unwrap();
        fs::write(root.join("src/primary.w"), body("app", 0)).unwrap();
        for (cap, expected, status) in [(1, &b"app"[..], 0), (2, &b"appa"[..], 4), (8, &b"appableaf"[..], 4)] {
            let receipts = private(cap);
            let checked = checked_publication_reports(&receipts).ok().unwrap();
            let planned = plan_publication_for(checked[0], &receipts[0].texts, checked.iter().copied()).unwrap();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(planned.prepare(root).ok().unwrap().publish(&mut out, &mut err).unwrap(), status);
            assert_eq!(out, expected);
            assert!(err.is_empty());
            assert!(!root.join(".wosy").exists());
        }
        let leaf_source = root.join("leaf/src/primary.w");
        fs::write(&leaf_source, "%%start u8 bad = true; %%end").unwrap();
        let blocked = private(1);
        let [failure] = blocked.as_slice() else { panic!("later static failure preempts cap"); };
        assert_eq!(failure.root, root.join("leaf"));
        assert!(matches!(failure.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Invalid(_))));
        assert!(checked_publication_reports(&blocked).is_err());
        assert!(!root.join(".wosy").exists());
        fs::write(&leaf_source, body("leaf", 6)).unwrap();
        fs::write(root.join("src/primary.w"), body("app", 3)).unwrap();

        let named_primary = r#"%%start
lib = namespace app "lib.w";
u8(u8) app_sleep = fn(value) { value };
proof dormant for app_sleep(entry) -> (out) where (entry == 5) ensures (out == 6) {}
u8 actual = lib.echo(3);
core.require(actual == 4, { .stdout = "app"; .stderr = ""; .code = 0; .files = []; });
%%end"#;
        let named_lib = r#"%%start
u8(u8) echo = fn(value) { if (value < 4) { value } else { 7 } };
u8(u8) lib_sleep = fn(value) { value };
proof dormant for lib_sleep(entry) -> (out) where (entry == 9) ensures (out == 10) {}
proof live for echo(entry) -> (out) where (entry == 3) ensures (out == 4) {}
%%end"#;
        fs::write(root.join("src/primary.w"), named_primary).unwrap();
        fs::write(root.join("src/lib.w"), named_lib).unwrap();
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            for primary_layout in ["native", "wasm-wasip1"] {
                let configured = app.replace("output = \"native\"", &format!("output = \"{primary_layout}\""));
                fs::write(root.join("wosy.toml"), configured).unwrap();
                let run = |cap| configured_check::check_targets_with_private_authored_limit(
                    root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                ).ok().unwrap();
                let legacy = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                let legacy_reports = checked_publication_reports(&legacy).ok().unwrap();
                let legacy_diagnostics = ordered_check_diagnostics(legacy_reports[0], &legacy[0].texts).unwrap();
                for (cap, expected, status) in [(1, &b"app"[..], 0), (2, &b"appa"[..], 4)] {
                    let receipts = run(cap);
                    let reports = checked_publication_reports(&receipts).ok().unwrap();
                    let [proof] = &reports[0].diagnostics[..] else { panic!("original invoked qualified proof remains independent"); };
                    assert_eq!(proof.invocation.call.source.path, "src/primary.w");
                    assert_eq!(proof.proof.source.path, "src/lib.w");
                    assert_eq!(proof.invocation.returned, 3.into());
                    assert_eq!(proof.invocation.namespaced_callable.as_ref().unwrap().source, proof.proof.source);
                    let symbolic = reports[0].symbolic_diagnostic.iter()
                        .chain(&reports[0].additional_symbolic_diagnostics)
                        .map(|failure| (failure.proof.source.path.as_str(), failure.returned.clone()))
                        .collect::<Vec<_>>();
                    if scope == ScalarCheckerScope::CurrentPackage {
                        assert_eq!(symbolic, [("src/primary.w", 5.into()), ("src/lib.w", 9.into())]);
                    } else {
                        assert!(symbolic.is_empty());
                    }
                    assert_eq!(ordered_check_diagnostics(reports[0], &receipts[0].texts).unwrap(), legacy_diagnostics);
                    let plan = plan_publication_for(reports[0], &receipts[0].texts, reports.iter().copied()).unwrap();
                    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
                    assert_eq!(plan.prepare(root).ok().unwrap().publish(&mut stdout, &mut stderr).unwrap(), status);
                    assert_eq!(stdout, expected);
                    assert!(stderr.is_empty() && !root.join(".wosy").exists());
                }
                fs::write(&leaf_source, "%%start u8 bad = true; %%end").unwrap();
                let blocked = run(1);
                let [failure] = blocked.as_slice() else { panic!("later static diagnostic suppresses all prior staging"); };
                assert_eq!(failure.root, root.join("leaf"));
                assert!(matches!(failure.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Invalid(_))));
                assert!(checked_publication_reports(&blocked).is_err() && !root.join(".wosy").exists());
                fs::write(&leaf_source, body("leaf", 6)).unwrap();
            }
        }
        fs::write(root.join("wosy.toml"), &app).unwrap();
        fs::write(root.join("src/primary.w"), body("app", 3)).unwrap();
        fs::remove_file(root.join("src/lib.w")).unwrap();

        fs::write(root.join("leaf/src/dormant.w"), r#"%%start
unit(u8) unseen = fn(value) {
    core.require(value < 1, { .stdout = "dormant"; .stderr = ""; .code = 7; .files = []; });
};
%%end"#).unwrap();
        let (_, _, targets) = ProjectConfiguration::load_with_dependency_check_targets(root).unwrap();
        assert_eq!(targets.iter().map(|child| (child.project.package.as_str(), child.layout.pointer_size)).collect::<Vec<_>>(),
            [("a", 4), ("b", 8), ("leaf", 4)]);
        assert_eq!(targets[0].configuration.resolved_packages.keys().map(String::as_str).collect::<Vec<_>>(), ["leaf"]);
        assert!(targets[2].configuration.resolved_packages.is_empty());
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            let receipts = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
            assert_eq!(receipts.len(), 4);
            let reports = receipts.iter().map(|receipt| receipt.outcome.as_ref().ok().unwrap()).collect::<Vec<_>>();
            let checked = plan_publication_for(reports[0], &receipts[0].texts, reports.iter().copied()).unwrap();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(checked.prepare(root).ok().unwrap().publish(&mut stdout, &mut stderr).unwrap(), 3);
            assert_eq!(stdout, b"appableafdormant");
            assert!(stderr.is_empty());
            assert!(!root.join(".wosy").exists());
        }

        let proof_only_leaf = r#"%%start
u8(u8) echo = fn(value) { value };
proof leaf_contract for echo(entry) -> (out) where (entry == 3) ensures (out == 4) {}
%%end"#;
        let leaf_dormant = root.join("leaf/src/dormant.w");
        let old_dormant = fs::read_to_string(&leaf_dormant).unwrap();
        fs::remove_file(&leaf_dormant).unwrap();
        fs::write(&leaf_source, proof_only_leaf).unwrap();
        let leaf_manifest = root.join("leaf/wosy.toml");
        let saved_leaf_manifest = fs::read_to_string(&leaf_manifest).unwrap();
        for layout in ["native", "wasm-wasip1"] {
            fs::write(&leaf_manifest, saved_leaf_manifest.replace("wasm-wasip1", layout)).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let legacy = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                let last = legacy.last().unwrap();
                let legacy_diagnostics = ordered_check_diagnostics(last.outcome.as_ref().ok().unwrap(), &last.texts).unwrap();
                assert_eq!(legacy_diagnostics.len(), 1);
                for (cap, expected, package_order) in [
                    (1, &b"app"[..], &["app", "leaf"][..]),
                    (2, &b"appa"[..], &["app", "a", "leaf"][..]),
                ] {
                    let receipts = configured_check::check_targets_with_private_authored_limit(
                        root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                    ).ok().unwrap();
                    let reports = checked_publication_reports(&receipts).ok().unwrap();
                    let actual_order = receipts.iter().map(|r|
                        if &r.root == root { "app" } else { r.root.file_name().unwrap().to_str().unwrap() }
                    ).collect::<Vec<_>>();
                    assert_eq!(actual_order, package_order);
                    let leaf = receipts.last().unwrap();
                    let formal = reports.last().unwrap();
                    assert!(formal.directives.is_empty());
                    let proof = formal.symbolic_diagnostic.as_ref().unwrap();
                    assert_eq!(proof.proof.source.package, "leaf");
                    assert_eq!(proof.returned, 3.into());
                    assert_eq!(ordered_check_diagnostics(formal, &leaf.texts).unwrap(), legacy_diagnostics);
                    let checked = plan_publication_for(reports[0], &receipts[0].texts, reports.iter().copied()).unwrap();
                    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
                    assert_eq!(checked.prepare(root).ok().unwrap().publish(&mut stdout, &mut stderr).unwrap(), 3);
                    assert_eq!(stdout, expected);
                    assert!(stderr.is_empty() && !root.join(".wosy").exists());
                }
            }
        }
        let mixed_leaf = r#"%%start
u8(u8) echo = fn(value) { value };
proof live for echo(entry) -> (out) where (entry == 3) ensures (out == 4) {}
u8[2]() make = fn { [7, 1] };
proof cells for make() -> (items)
    forall (u64 i where i == 0) ensures (items[i] == 8)
{ core.show(items[i] == 8); }
core.require(false, { .stdout = "leaf"; .stderr = ""; .code = 6; .files = []; });
u8 observed = echo(3);
core.require(observed == 3, { .stdout = "not selected"; .stderr = ""; .code = 9; .files = []; });
u8[2] actual = make();
u8 cell = actual[0];
core.require(cell == 7, { .stdout = "not selected"; .stderr = ""; .code = 9; .files = []; });
%%end"#;
        for layout in ["native", "wasm-wasip1"] {
            fs::write(&leaf_manifest, saved_leaf_manifest.replace("wasm-wasip1", layout)).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let run = |cap| configured_check::check_targets_with_private_authored_limit(
                    root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                ).ok().unwrap();
                for deferred in [false, true] {
                    let leaf = if deferred { mixed_leaf.replace("u8[2] actual = make();\nu8 cell = actual[0];", "u8 cell = 7;") }
                        else { mixed_leaf.to_owned() };
                    assert_eq!(leaf != mixed_leaf, deferred);
                    fs::write(&leaf_source, &leaf).unwrap();
                    let old = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                    let old_last = old.last().unwrap();
                    let old_diagnostics = ordered_check_diagnostics(old_last.outcome.as_ref().unwrap(), &old_last.texts).unwrap();
                    for (cap, output, authored_leaf) in [(1, "app", false), (4, "appableaf", true), (8, "appableaf", true)] {
                        let receipts = run(cap);
                        let reports = checked_publication_reports(&receipts).unwrap_or_else(|receipt| panic!("mixed dependency check rejected: {:?}", receipt.outcome.as_ref().err()));
                        let last = receipts.last().unwrap();
                        assert_eq!(last.root, root.join("leaf"));
                        let formal = reports.last().unwrap();
                        assert_eq!(formal.directives.len(), usize::from(authored_leaf));
                        let [scalar] = formal.diagnostics.as_slice() else { panic!("actual transitive dependency scalar proof survives cap"); };
                        assert_eq!(scalar.proof.source.package, "leaf");
                        assert_eq!(scalar.invocation.returned, 3.into());
                        // Selected dependency targets use their own declared package scope,
                        // including when the app uses --main-flow-only.
                        let [array] = formal.aggregate_diagnostics.as_slice() else { panic!("transitive dependency array proof survives cap"); };
                        assert_eq!(array.proof.source.package, "leaf");
                        assert_eq!(array.call.is_none(), deferred);
                        assert_eq!(array.cell.as_ref().unwrap().value, 7.into());
                        assert_eq!(ordered_check_diagnostics(formal, &last.texts).unwrap(), old_diagnostics);
                        let plan = plan_publication_for(reports[0], &receipts[0].texts, reports.iter().copied()).unwrap();
                        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
                        assert_eq!(plan.prepare(root).ok().unwrap().publish(&mut stdout, &mut stderr).unwrap(), 3);
                        assert_eq!(stdout, output.as_bytes());
                        assert!(stderr.is_empty() && !root.join(".wosy").exists());
                    }
                }
                fs::write(&leaf_source, mixed_leaf.replace("core.require(false,", "core.require(false || true,")).unwrap();
                let refused = run(1);
                let [rejected] = refused.as_slice() else { panic!("unsupported mixed owner must not disappear behind cap"); };
                assert_eq!(rejected.root, root.join("leaf"));
                assert!(matches!(rejected.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(checked_publication_reports(&refused).is_err());
                assert!(!root.join(".wosy").exists());
                fs::write(&leaf_source, mixed_leaf.replace("u8 cell = actual[0]", "bool cell = actual[0]")).unwrap();
                let invalid = run(1);
                assert!(matches!(invalid.as_slice(), [failure]
                    if failure.root == root.join("leaf") && matches!(failure.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Invalid(_)))));
                assert!(checked_publication_reports(&invalid).is_err());
                assert!(!root.join(".wosy").exists());
            }
        }
        let guarded_leaf = r#"%%start
unit(u8) consume = fn(charged) {
    if (charged == 1) { core.context.credit = core.context.credit - 1; }
    else { core.context.credit = core.context.credit - 1; };
};
unit() entry = fn {
    u8 core.context.credit = 3;
    core.require(false, { .stdout = "guarded"; .stderr = ""; .code = 6; .files = []; });
    consume(1);
};
proof debit for consume(cost) -> (null) where (cost <= 1)
    ensures (core.context.credit == core.old(core.context.credit) - 1) {}
entry();
%%end"#;
        let split_leaf = r#"%%start
lib = namespace leaf "credit.w";
unit() entry = fn {
    u8 core.context.credit = 3;
    lib.spend();
};
u8(u8) score = fn(value) { value };
proof scalar_value for score(entry) -> (out) where (entry == 4) ensures (out == 3) {}
core.require(false, { .stdout = "guarded"; .stderr = ""; .code = 6; .files = []; });
entry();
u8 observed = score(4);
core.require(observed == 4, { .stdout = "not selected"; .stderr = ""; .code = 9; .files = []; });
%%end"#;
        let credit_provider = r#"%%start
unit() consume = fn { core.context.credit = core.context.credit - 1; };
unit() spend = fn { consume(); };
proof debit for consume() -> (null)
    ensures (core.context.credit == core.old(core.context.credit) - 1) {}
%%end"#;
        let credit_source = root.join("leaf/src/credit.w");
        for layout in ["native", "wasm-wasip1"] {
            fs::write(&leaf_manifest, saved_leaf_manifest.replace("wasm-wasip1", layout)).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let run = |cap| configured_check::check_targets_with_private_authored_limit(
                    root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                ).ok().unwrap();
                for (mixed, refuted) in [(false, false), (false, true), (true, false), (true, true)] {
                    let proof = |text: &str| if refuted {
                        text.replace("core.old(core.context.credit) - 1", "core.old(core.context.credit) - 0")
                    } else { text.to_owned() };
                    if mixed { fs::write(&credit_source, proof(credit_provider)).unwrap(); }
                    let actual = if mixed { split_leaf.to_owned() } else { proof(guarded_leaf) };
                    fs::write(&leaf_source, &actual).unwrap();
                    let mut legacy = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                    if !mixed {
                        // Legacy rejects a typed where. The only actual
                        // cost, 1, satisfies this case's guard. Substitute
                        // an unguarded oracle without moving source offsets.
                        assert!(matches!(legacy.last().unwrap().outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                        let guard = " where (cost <= 1)";
                        let unguarded = actual.replace(guard, &" ".repeat(guard.len()));
                        assert_ne!(unguarded, actual);
                        fs::write(&leaf_source, unguarded).unwrap();
                        legacy = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                        fs::write(&leaf_source, &actual).unwrap();
                    }
                    let last = legacy.last().unwrap();
                    let legacy_diagnostics = ordered_check_diagnostics(last.outcome.as_ref().unwrap(), &last.texts).unwrap();
                    assert_eq!(legacy_diagnostics.len(), usize::from(refuted) + usize::from(mixed));
                    for cap in [1, 4, 8] {
                        let receipts = run(cap);
                        let reports = checked_publication_reports(&receipts)
                            .unwrap_or_else(|receipt| panic!("bounded contextual dependency refused: {:?}", receipt.outcome.as_ref().err()));
                        let leaf = receipts.last().unwrap();
                        assert_eq!(leaf.root, root.join("leaf"));
                        let formal = reports.last().unwrap();
                        let selected_leaf = cap != 1 && (mixed || !refuted);
                        assert_eq!(formal.directives.len(), usize::from(selected_leaf));
                        assert_eq!(formal.diagnostics.len(), usize::from(mixed));
                        let diagnostics = ordered_check_diagnostics(formal, &leaf.texts).unwrap();
                        if mixed {
                            assert_eq!(diagnostics, legacy_diagnostics);
                        } else {
                            assert_eq!(diagnostics.len(), legacy_diagnostics.len());
                            for (observed, expected) in diagnostics.iter().zip(&legacy_diagnostics) {
                                assert_eq!((&observed.message, &observed.notes), (&expected.message, &expected.notes));
                                assert_eq!(observed.labels.len(), expected.labels.len());
                                for (observed, expected) in observed.labels.iter().zip(&expected.labels) {
                                    assert_eq!((&observed.kind, &observed.message, &observed.span.source.path, observed.span.range),
                                        (&expected.kind, &expected.message, &expected.span.source.path, expected.span.range));
                                }
                            }
                        }
                        if refuted {
                            let failure = formal.contextual_diagnostic.as_ref().unwrap();
                            assert_eq!(failure.proof.source.package, "leaf");
                            assert_eq!((&failure.entry, &failure.exit), (&3.into(), &2.into()));
                            assert_eq!(failure.supplying_call.source.path, "src/primary.w");
                            assert_eq!(failure.call.source.path, if mixed { "src/credit.w" } else { "src/primary.w" });
                        } else {
                            assert!(formal.contextual_diagnostic.is_none());
                        }
                        let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                        assert_eq!(publish_configured_check_receipts(&receipts, scope, true, &mut stdout, &mut stderr), Ok(3));
                        let expected: &[u8] = if cap == 1 { b"app" }
                            else if selected_leaf { b"appabguarded" } else { b"appab" };
                        assert_eq!(stdout, expected);
                        assert_eq!(stderr.as_slice().is_empty(), !refuted && !mixed);
                        assert!(!root.join(".wosy").exists());
                    }
                    if mixed { fs::remove_file(&credit_source).unwrap(); }
                }
                // A valid but currently unmodelled condition before the
                // formal call cannot silently authorize its continued reach.
                fs::write(&leaf_source, guarded_leaf.replace("core.require(false,", "core.require(false || true,")).unwrap();
                let unsupported = run(1);
                assert!(matches!(unsupported.as_slice(), [failure] if failure.root == root.join("leaf")
                    && matches!(failure.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported))));
                let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                assert!(publish_configured_check_receipts(&unsupported, scope, true, &mut stdout, &mut stderr).is_err());
                assert!(stdout.is_empty() && stderr.as_slice().is_empty() && !root.join(".wosy").exists());
                // Separate static input validation from the exact
                // contextual transport's conservative Unsupported lane.
                fs::write(&leaf_source, "%%start u8 bad = true; %%end").unwrap();
                let invalid = run(1);
                assert!(matches!(invalid.as_slice(), [failure] if failure.root == root.join("leaf")
                    && matches!(failure.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Invalid(_)))));
                let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                assert_eq!(publish_configured_check_receipts(&invalid, scope, true, &mut stdout, &mut stderr), Ok(2));
                assert!(stdout.is_empty() && !stderr.as_slice().is_empty() && !root.join(".wosy").exists());
            }
        }
        fs::write(&leaf_manifest, &saved_leaf_manifest).unwrap();
        fs::write(&leaf_source, body("leaf", 6)).unwrap();

        let rank_leaf = r#"%%start
u8(u8) descend = fn(value) { if (value < 3) { 4 } else { descend(value - 3) } };
proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out <= 3) decreases (entry) {
    if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
}
core.require(false, { .stdout = "rankleaf"; .stderr = ""; .code = 6; .files = []; });
%%end"#;
        for layout in ["native", "wasm-wasip1"] {
            fs::write(&leaf_manifest, saved_leaf_manifest.replace("wasm-wasip1", layout)).unwrap();
            fs::write(&leaf_source, rank_leaf).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let old = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                let leaf = old.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap();
                let expected = leaf.outcome.as_ref().unwrap();
                assert_eq!(expected.directives.len(), 1);
                for cap in [1, 8] {
                    let selected = configured_check::check_targets_with_private_authored_limit(
                        root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                    ).ok().unwrap();
                    let leaf = selected.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap();
                    let formal = leaf.outcome.as_ref().unwrap();
                    assert_eq!(formal.directives, expected.directives[..usize::from(cap > 1)]);
                    // Explicit dependencies always prepare their own CurrentPackage scope.
                    let observed = formal.symbolic_diagnostic.as_ref().unwrap();
                    let original = expected.symbolic_diagnostic.as_ref().unwrap();
                    assert_eq!((&observed.goal, &observed.returned), (&original.goal, &original.returned));
                    assert_eq!(observed.proof.source.package, "leaf");
                    assert_eq!(observed.directive_position, usize::from(cap > 1));
                    let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                    assert_eq!(publish_configured_check_receipts(&selected, scope, true, &mut stdout, &mut stderr), Ok(3));
                    if cap == 1 { assert_eq!(stdout, b"app"); }
                    else { assert!(stdout.windows(8).any(|bytes| bytes == b"rankleaf")); }
                    assert!(!root.join(".wosy").exists());
                }
            }
        }
        let loan_leaf = r#"%%start
unit(*!u32) advance = fn(cell) { *cell = *cell +@ 1; };
proof epoch for advance(p) -> () ensures (*p == core.old(*p) +@ 1) {}
unit() start = fn {
    u32 local = 4294967295;
    advance(&!local);
    advance(&!local);
    core.require(local == 1, { .stdout = "unexpected"; .stderr = ""; .code = 9; .files = []; });
    core.require(local == 0, { .stdout = "loan-first"; .stderr = ""; .code = 6; .files = []; });
    core.require(local == 2, { .stdout = "loan-second"; .stderr = ""; .code = 7; .files = []; });
};
start();
%%end"#;
        for layout in ["native", "wasm-wasip1"] {
            fs::write(&leaf_manifest, saved_leaf_manifest.replace("wasm-wasip1", layout)).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                fs::write(&leaf_source, loan_leaf).unwrap();
                let old = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                let leaf = old.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap();
                let expected = leaf.outcome.as_ref().unwrap();
                assert_eq!(expected.directives.len(), 2);
                let run = |cap| configured_check::check_targets_with_private_authored_limit(
                    root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                ).ok().unwrap();
                for (cap, count, stdout_expected) in [
                    (1, 0, &b"app"[..]), (2, 0, &b"appa"[..]), (3, 0, &b"appab"[..]),
                    (4, 1, &b"appabloan-first"[..]),
                    (8, 2, &b"appabloan-firstloan-second"[..]),
                ] {
                    let selected = run(cap);
                    let leaf = selected.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap();
                    let report = leaf.outcome.as_ref().unwrap();
                    assert!(report.active && report.pointer_effect_diagnostic.is_none());
                    assert_eq!(report.directives, expected.directives[..count]);
                    let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                    assert_eq!(publish_configured_check_receipts(&selected, scope, true, &mut stdout, &mut stderr), Ok(3));
                    assert_eq!(stdout, stdout_expected);
                    assert!(stderr.as_slice().is_empty() && !root.join(".wosy").exists());
                }
                fs::write(&leaf_source, loan_leaf.replace("core.old(*p) +@ 1", "core.old(*p) +@ 2")).unwrap();
                let refused = run(1);
                let leaf = refused.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap();
                assert!(matches!(leaf.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                assert!(publish_configured_check_receipts(&refused, scope, true, &mut stdout, &mut stderr).is_err());
                assert!(stdout.is_empty() && stderr.as_slice().is_empty() && !root.join(".wosy").exists());
            }
        }
        let left_provider = root.join("leaf/src/left.w");
        let right_provider = root.join("leaf/src/right.w");
        let leaf_roster = r#"%%start
left = namespace leaf "left.w";
right = namespace leaf "right.w";
u8 x = left.echo(3);
u8 y = right.echo(5);
core.require(x == 3, { .stdout = "wrong left"; .stderr = ""; .code = 9; .files = []; });
core.require(y == 5, { .stdout = "wrong right"; .stderr = ""; .code = 9; .files = []; });
core.require(false, { .stdout = "leafroot"; .stderr = ""; .code = 6; .files = []; });
%%end"#;
        let make_provider = |name| format!(r#"%%start
u8(u8) echo = fn(value) {{ value }};
proof exact for echo(entry) -> (out) where (entry <= 5) ensures (out == 8) {{}}
core.require(false, {{ .stdout = "{name}"; .stderr = ""; .code = 6; .files = []; }});
%%end"#);
        let left_text = make_provider("left");
        fs::write(&leaf_source, leaf_roster).unwrap();
        fs::write(&right_provider, make_provider("right")).unwrap();
        for layout in ["native", "wasm-wasip1"] {
            fs::write(&leaf_manifest, saved_leaf_manifest.replace("wasm-wasip1", layout)).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                fs::write(&left_provider, &left_text).unwrap();
                let run = |cap| configured_check::check_targets_with_private_authored_limit(
                    root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                ).ok().unwrap();
                let original = check_target_with_dependencies(root, None, "dev", scope).ok().unwrap();
                let old = original.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap().outcome.as_ref().unwrap();
                assert_eq!(old.directives.len(), 3);
                for (cap, count, shown) in [
                    (1, 0, &b"app"[..]), (3, 0, &b"appab"[..]), (4, 1, &b"appableft"[..]),
                    (8, 3, &b"appableftrightleafroot"[..]),
                ] {
                    let receipts = run(cap);
                    let leaf = receipts.iter().find(|receipt| receipt.root == root.join("leaf")).unwrap();
                    let report = leaf.outcome.as_ref().unwrap();
                    assert_eq!(report.directives, old.directives[..count]);
                    assert_eq!(report.diagnostics.len(), 2);
                    for (index, (diagnostic, (path, returned))) in report.diagnostics.iter().zip([
                        ("src/left.w", 3), ("src/right.w", 5),
                    ]).enumerate() {
                        assert_eq!(diagnostic.proof.source.path, path);
                        assert_eq!(diagnostic.invocation.call, old.diagnostics[index].invocation.call);
                        assert_eq!(diagnostic.invocation.call.source.path, "src/primary.w");
                        assert_eq!(diagnostic.invocation.returned, returned.into());
                        assert_eq!(diagnostic.invocation.directive_position, count.min(2));
                    }
                    let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                    assert_eq!(publish_configured_check_receipts(&receipts, scope, true, &mut stdout, &mut stderr), Ok(3));
                    assert_eq!(stdout, shown);
                    assert!(!stderr.as_slice().is_empty() && !root.join(".wosy").exists());
                }
                // The candidate is invoked outside the selected package root
                // as well. That is not one of the authenticated startup sites.
                fs::write(&left_provider, left_text.replace("%%end", r#"u8 side = echo(3);
core.require(side == 3, { .stdout = "unexpected"; .stderr = ""; .code = 9; .files = []; });
%%end"#)).unwrap();
                let refused = run(1);
                let [failure] = refused.as_slice() else { panic!("whole publication refused"); };
                assert_eq!(failure.root, root.join("leaf"));
                assert!(matches!(failure.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                let (mut stdout, mut stderr) = (Vec::new(), Buffer::no_color());
                assert!(publish_configured_check_receipts(&refused, scope, true, &mut stdout, &mut stderr).is_err());
                assert!(stdout.is_empty() && stderr.as_slice().is_empty() && !root.join(".wosy").exists());
            }
        }
        fs::remove_file(&left_provider).unwrap();
        fs::remove_file(&right_provider).unwrap();
        fs::write(&leaf_manifest, saved_leaf_manifest).unwrap();
        fs::write(&leaf_source, body("leaf", 6)).unwrap();

        fs::write(&leaf_dormant, old_dormant).unwrap();
        let root_dormant = root.join("src/more.w");
        let dormant_program = fs::read_to_string(root.join("leaf/src/dormant.w")).unwrap();
        fs::write(&root_dormant, &dormant_program).unwrap();
        let root_only = |scope, cap| configured_check::check_target_with_private_authored_limit(
            root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
        ).ok().unwrap();
        for layout in ["native", "wasm-wasip1"] {
            let configured = app.replace("output = \"native\"", &format!("output = \"{layout}\""));
            fs::write(root.join("wosy.toml"), configured).unwrap();
            for (scope, cap, expected) in [
                (ScalarCheckerScope::CurrentPackage, 1, &b"app"[..]),
                (ScalarCheckerScope::CurrentPackage, 2, &b"appdormant"[..]),
                (ScalarCheckerScope::CurrentPackage, 8, &b"appdormant"[..]),
                (ScalarCheckerScope::MainFlowOnly, 8, &b"app"[..]),
            ] {
                let receipts = root_only(scope, cap);
                let [receipt] = receipts.as_slice() else { panic!("only primary owner selected"); };
                let report = receipt.outcome.as_ref().ok().unwrap();
                if cap >= 2 && scope == ScalarCheckerScope::CurrentPackage {
                    assert_eq!(report.directives[1].source.path, "src/more.w");
                }
                if cap == 8 {
                    let legacy = match scope {
                        ScalarCheckerScope::CurrentPackage => configured_check::check_declared_package_target(root, None, "dev"),
                        ScalarCheckerScope::MainFlowOnly => configured_check::check_target(root, None, "dev", scope, Vec::new()),
                    }.ok().unwrap();
                    let prior = legacy.outcome.as_ref().ok().unwrap();
                    assert_eq!(report.directives.len(), prior.directives.len());
                    for (current, prior) in report.directives.iter().zip(&prior.directives) {
                        assert_eq!((&current.source, current.directive, current.condition, current.code(), current.stdout(), &current.witness),
                            (&prior.source, prior.directive, prior.condition, prior.code(), prior.stdout(), &prior.witness));
                    }
                }
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let plan = receipt.plan_publication().ok().unwrap();
                assert_eq!(plan.prepare(root).ok().unwrap().publish(&mut stdout, &mut stderr).unwrap(), 3);
                assert_eq!(stdout, expected);
                assert!(stderr.is_empty() && !root.join(".wosy").exists());
            }
            fs::write(&root_dormant, "%%start u8 bad = true; %%end").unwrap();
            let blocked = root_only(ScalarCheckerScope::CurrentPackage, 1);
            assert!(matches!(blocked[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Invalid(_))));
            assert!(root_only(ScalarCheckerScope::MainFlowOnly, 1)[0].outcome.is_ok());
            assert!(!root.join(".wosy").exists());
            fs::write(&root_dormant, &dormant_program).unwrap();
        }
        fs::remove_file(&root_dormant).unwrap();
        fs::write(root.join("wosy.toml"), &app).unwrap();

        let sponsored_file = |packet: &str| format!(r#"%%start
core.require(false, {{ .stdout = "{packet}"; .stderr = ""; .code = 1;
    .files = [{{ .path = "same.txt"; .text = "{packet}"; }}]; }});
%%end"#);
        fs::write(root.join("src/primary.w"), sponsored_file("root")).unwrap();
        fs::write(root.join("leaf/src/primary.w"), sponsored_file("leaf")).unwrap();
        let receipts = check_target_with_dependencies(root, None, "dev", ScalarCheckerScope::CurrentPackage).ok().unwrap();
        let reports = receipts.iter().map(|receipt| receipt.outcome.as_ref().ok().unwrap()).collect::<Vec<_>>();
        assert!(matches!(plan_publication_for(reports[0], &receipts[0].texts, reports.iter().copied()),
            Err(CheckPublicationError::ConflictingPath { source, .. }) if source.package == "leaf"));
        assert!(!root.join(".wosy").exists());

        fs::write(root.join("wosy.toml"), app + "\n[dependencies.remote]\ngit = \"https://example.invalid/pkg\"\n").unwrap();
        assert!(matches!(check_target_with_dependencies(root, None, "dev", ScalarCheckerScope::CurrentPackage),
            Err(ConfiguredCheckLoadError::Dependency(error)) if error.contains("Git dependencies require wosy.lock; dep sync does not select revisions")));
        assert!(!root.join(".wosy").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(feature = "internal-checker", target_os = "linux"))]
    #[test]
    fn private_configured_release_matrix_classifies_real_incumbent_and_candidate() {
        use codespan_reporting::term::termcolor::Buffer;
        use std::{num::NonZeroUsize, path::Path, time::Instant};
        let (fixture, _) = routing_test_project("private-check-release-matrix", &[]);
        let root = &fixture.root;
        let manifest = |package: &str, layout: &str| format!(r#"
[package]
name = "{package}"
default_target = "main"
source_root = "src"
[comments]
categories = []
[builders]
[runners]
[targets.main]
root = "src/main.w"
default_profile = "dev"
[targets.main.artifacts.main]
artifact = "RuntimeStdio"
[targets.main.artifacts.main.profiles.dev]
backend = "llvm"
output = "{layout}"
builder = "never"
run_runner = "never"
[targets.main.profiles.dev]
main_artifact = "main"
"#);
        let output = |status, label: &str| format!(r#"{{ .stdout = "{label}"; .stderr = ""; .code = {status}; .files = []; }}"#);
        let first = output(0, "startup");
        let report = output(7, "after");
        let context = format!(r#"%%start
            unit(i8) keep = fn(entry) {{ core.narrow(entry >= 0, {first}); }};
            unit(i8) caller = fn(input) {{
                i8 core.context.count = input;
                keep(core.context.count);
                core.require(input >= 0, {first});
                if (input == 2) {{
                    i8 core.context.count = core.context.count;
                    core.context.count = 7;
                    core.require(core.context.count == 2, {});
                }};
                if (input == 2) {{ core.require(core.context.count == 7, {}); }};
            }};
            keep(0);
        %%end"#, output(51, "inner"), output(52, "restored"));
        let snapshots = r#"%%start
            struct Snapshot { i8 raised; i8 reflected; i8 plain; }
            unit(i8) inspect = fn(value) {
                core.narrow(value == 1 || value == 3, { .stdout = ""; .stderr = ""; .code = 1; .files = []; });
                Snapshot snapshot = { .raised = value + 2; .reflected = 7 - value; .plain = value; };
                if (value == 1) {
                    i8 read = snapshot.raised;
                    core.require(read == 3, { .stdout = ""; .stderr = "lost preimage"; .code = 58; .files = []; });
                };
                value = 7;
                core.require(value != 7, { .stdout = ""; .stderr = "construction"; .code = 59;
                    .files = [{ .path = "state.json"; .state = snapshot; }]; });
            };
        %%end"#;
        let units = format!(r#"%%start
            unit(u8, u8) leaf = fn(left, right) {{
                if (left < 128) {{ core.narrow(right < 64, {first}); }};
            }};
            unit(u8, u8) inspect = fn(a, b) {{
                leaf(a, b);
                core.require(b != 3, {report});
            }};
        %%end"#);
        let nominal = format!(r#"%%start
            enum copy Region {{ left(u8, u16); right(u16, u8); }}
            unit(i8, u16) inspect = fn(input, entry) {{
                u16 actual = match (if (input < 0) {{ Region.left(3, entry) }} else {{ Region.right(513, 7) }}) {{
                    right(wide, narrow) {{ wide }}
                    left(narrow, wide) {{ wide }}
                }};
                core.require(input < 0 || actual == 512, {report});
            }};
        %%end"#);
        let named = format!(r#"%%start
            u8(u8) echo = fn(value) {{ value }};
            proof honest for echo(entry) -> (out) where (entry == 3) ensures (out == 4) {{}}
            u8 actual = echo(3);
            core.require(actual == 4, {first});
        %%end"#);
        let startup = format!(r#"%%start
            bool(bool, i8) choose = fn(flag, entry) {{
                core.require(entry != 3, {first});
                if (flag) {{ true }} else {{ false }}
            }};
            bool initialized = true;
            bool ready = choose(initialized, 3);
            i8 status = if (ready) {{ 7 }} else {{ 8 }};
            core.require(status < 7, {report});
        %%end"#);
        let dep_app = format!("%%start\ncore.require(false, {first});\n%%end");
        let dependency_files = |label: &str| format!(r#"%%start
            core.require(false, {{ .stdout = "{label}"; .stderr = ""; .code = 11;
                .files = [{{ .path = "{label}.txt"; .text = "{label}"; }}]; }});
        %%end"#);
        let guarded_context = format!(r#"%%start
            unit(u8) consume = fn(charged) {{
                if (charged == 1) {{ core.context.credit = core.context.credit - 1; }}
                else {{ core.context.credit = core.context.credit - 1; }};
            }};
            unit() entry = fn {{
                u8 core.context.credit = 3;
                consume(1);
                core.require(false, {first});
                core.require(false, {report});
            }};
            proof debit for consume(cost) -> (null) where (cost <= 1)
                ensures (core.context.credit == core.old(core.context.credit) - 1) {{}}
            entry();
        %%end"#);
        let mixed_context = format!(r#"%%start
            lib = namespace app "credit.w";
            unit() entry = fn {{
                u8 core.context.credit = 3;
                lib.spend();
            }};
            u8(u8) score = fn(value) {{ value }};
            proof scalar_value for score(entry) -> (out) where (entry == 4) ensures (out == 3) {{}}
            core.require(false, {first});
            entry();
            u8 observed = score(4);
            core.require(observed == 4, {report});
            core.require(false, {report});
        %%end"#);
        let credit = r#"%%start
            unit() consume = fn { core.context.credit = core.context.credit - 1; };
            unit() spend = fn { consume(); };
            proof debit for consume() -> (null)
                ensures (core.context.credit == core.old(core.context.credit) - 1) {}
        %%end"#;
        let checked_loan = format!(r#"%%start
            unit(*!u32) advance = fn(cell) {{ *cell = *cell +@ 1; }};
            proof epoch for advance(p) -> () ensures (*p == core.old(*p) +@ 1) {{}}
            unit() start = fn {{
                u32 local = 4294967295;
                advance(&!local);
                advance(&!local);
                core.require(local == 1, {report});
                core.require(local == 0, {first});
                core.require(local == 2, {report});
            }};
            start();
        %%end"#);
        let typed_array = format!(r#"%%start
            u8[2](u8) make = fn(input) {{ if (input == 0) {{ [7, 1] }} else {{ [1, 7] }} }};
            proof cells for make(entry) -> (items)
                forall (u64 i where i == 0) ensures (items[i] == 8)
            {{ core.show(items[i] == 8); }}
            u8[2] first_array = make(0);
            u8 head = first_array[0];
            core.require(head == 8, {first});
            u8[2] second_array = make(1);
            u8 tail = second_array[1];
            core.require(tail == 2, {report});
        %%end"#);
        let qualified_nested = format!(r#"%%start
            lib = namespace app "wide.w";
            u8(u8,u16,u8,u16) forward = fn(left, page, flag, quota) {{ lib.descend(left, page, flag, quota) }};
            u8 actual = forward(8, 12, 1, 4);
            core.require(actual == 7, {report});
            core.require(false, {report});
        %%end"#);
        let wide = format!(r#"%%start
            u8(u8,u16,u8,u16) descend = fn(left, page, flag, quota) {{
                if (left < 3) {{ 7 }} else {{ descend(left - 3, page, flag, quota) }}
            }};
            proof rank for descend(left, page, flag, quota) -> (out)
                where (left <= 8 && page == 12 && flag == 1 && quota == 4)
                ensures (out == 7) decreases (left) {{
                    if (left < 3) {{ core.show(out == 7); }} else {{ core.use(rank(left - 3, page, flag, quota)); }}
                }}
            core.require(false, {first});
        %%end"#);
        let cases = [
            ("typed-bool-startup", startup, false, None),
            ("nested-binary-unit", units, false, None),
            ("nominal-arm-payload", nominal, false, None),
            ("named-real-invocation", named, false, None),
            ("ordinary-context-restore", context, false, None),
            ("affine-snapshot-actual", snapshots.replace("%%end", "inspect(1); %%end"), false, None),
            ("affine-snapshot-symbolic", snapshots.to_owned(), false, None),
            ("configured-transitive-dependencies", dep_app, true, None),
            ("contextual-guarded-debit", guarded_context.clone(), false, None),
            ("contextual-guarded-refutation", guarded_context.replace("core.old(core.context.credit) - 1", "core.old(core.context.credit) - 2"), false, None),
            ("mixed-contextual-named", mixed_context.clone(), false, Some(("credit.w", credit.to_owned()))),
            ("mixed-contextual-refutation", mixed_context, false, Some(("credit.w", credit.replace("core.old(core.context.credit) - 1", "core.old(core.context.credit) - 2")))),
            ("checked-loan-effects", checked_loan.clone(), false, None),
            ("checked-loan-refutation", checked_loan.replace("core.old(*p) +@ 1", "core.old(*p) +@ 2"), false, None),
            ("branching-array-proof", typed_array.clone(), false, None),
            ("array-rank-mixed", typed_array.replace("%%end", r#"
                u8(u8) descend = fn(value) { if (value < 3) { 4 } else { descend(value - 3) } };
                proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
                    if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
                }
                u8 reached = descend(8);
                core.require(reached == 4, { .stdout = "rank-incorrect"; .stderr = ""; .code = 29; .files = []; });
            %%end"#), false, None),
            ("qualified-nested-wide", qualified_nested, false, Some(("wide.w", wide))),
        ];
        let type_labels = |receipts: &[configured_check::ConfiguredCheckReceipt]| {
            use wosy_compiler::ScalarOrderedCheckerOutcome as Outcome;
            receipts.iter().map(|receipt| match &receipt.outcome {
                Ok(_) => "Ready", Err(Outcome::Unsupported) => "Unsupported",
                Err(Outcome::Invalid(_)) => "Invalid", Err(Outcome::NoChecks) => "NoChecks",
                Err(Outcome::Passed) => "Passed", Err(Outcome::Failed(_)) => "Failed",
            }).collect::<Vec<_>>()
        };
        fn authored_files(root: &Path, dir: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
            if !dir.is_dir() { return; }
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() { authored_files(root, &path, files); }
                else {
                    assert!(kind.is_file());
                    files.insert(path.strip_prefix(root).unwrap().to_owned(), fs::read(path).unwrap());
                }
            }
        }
        let package_roots = ["", "dep", "dep/leaf"];
        let snapshot = || {
            let mut files = BTreeMap::new();
            for package in package_roots {
                let path = root.join(package).join(".wosy");
                assert!(!path.join("artifacts").exists(), "Check must never build an artifact");
                authored_files(root, &path.join("outputs/check"), &mut files);
            }
            files
        };
        let reset = || for package in package_roots {
            let state = root.join(package).join(".wosy");
            if state.exists() { fs::remove_dir_all(state).unwrap(); }
        };
        let mut samples = 0;
        let mut classes = BTreeMap::<&str, usize>::new();
        for layout in ["native", "wasm-wasip1"] {
            for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
                for (name, program, dependencies, provider) in &cases {
                    let dependencies = *dependencies;
                    let mut primary = manifest("app", layout);
                    if dependencies { primary.push_str("\n[dependencies.dep]\npath = \"dep\"\n"); }
                    fs::create_dir_all(root.join("src")).unwrap();
                    fs::write(root.join("wosy.toml"), primary).unwrap();
                    fs::write(root.join("src/main.w"), program).unwrap();
                    for file in ["credit.w", "wide.w"] {
                        let path = root.join("src").join(file);
                        if path.exists() { fs::remove_file(path).unwrap(); }
                    }
                    if let Some((relative, source)) = provider {
                        fs::write(root.join("src").join(relative), source).unwrap();
                    }
                    if dependencies {
                        for (path, suffix, backend) in [("dep", "dep", "wasm-wasip1"), ("dep/leaf", "leaf", "native")] {
                            let package = root.join(path);
                            fs::create_dir_all(package.join("src")).unwrap();
                            let mut config = manifest(suffix, backend);
                            if suffix == "dep" { config.push_str("\n[dependencies.leaf]\npath = \"leaf\"\n"); }
                            fs::write(package.join("wosy.toml"), config).unwrap();
                            fs::write(package.join("src/main.w"), dependency_files(suffix)).unwrap();
                        }
                    }
                    reset();
                    let publish = |receipts: &[configured_check::ConfiguredCheckReceipt]| {
                        let mut stdout = Vec::new();
                        let mut stderr = Buffer::no_color();
                        let started = Instant::now();
                        let status = publish_configured_check_receipts(receipts, scope, dependencies, &mut stdout, &mut stderr);
                        let elapsed = started.elapsed();
                        ((status, stdout, stderr.as_slice().to_vec(), snapshot()), elapsed)
                    };
                    let started = Instant::now();
                    let incumbent = if dependencies {
                        configured_check::check_target_with_dependencies(root, None, "dev", scope)
                    } else if scope == ScalarCheckerScope::CurrentPackage {
                        configured_check::check_declared_package_target(root, None, "dev").map(|receipt| vec![receipt])
                    } else {
                        configured_check::check_target(root, None, "dev", scope, Vec::new()).map(|receipt| vec![receipt])
                    }.unwrap_or_else(|error| panic!("incumbent {name} {layout} {scope:?} load error: {:?}", std::mem::discriminant(&error)));
                    let old_load = started.elapsed();
                    let (baseline, old_publish) = publish(&incumbent);
                    let old_status = type_labels(&incumbent);
                    for cap in [1, 64] {
                        reset();
                        let start = Instant::now();
                        let cap_value = NonZeroUsize::new(cap).unwrap();
                        let candidate = if dependencies {
                            configured_check::check_targets_with_private_authored_limit(root, None, "dev", scope, cap_value)
                        } else {
                            configured_check::check_target_with_private_authored_limit(root, None, "dev", scope, cap_value)
                        }.unwrap_or_else(|error| panic!("candidate {name} {layout} {scope:?} load error: {:?}", std::mem::discriminant(&error)));
                        let load = start.elapsed();
                        let (observed, emit) = publish(&candidate);
                        let new_status = type_labels(&candidate);
                        let status_match = baseline.0 == observed.0;
                        let incumbent_refused = baseline.0.is_err() || old_status.iter().any(|label| matches!(*label, "Unsupported" | "Invalid"));
                        let candidate_refused = observed.0.is_err() || new_status.iter().any(|label| matches!(*label, "Unsupported" | "Invalid"));
                        if candidate_refused {
                            assert!(observed.1.is_empty() && observed.3.is_empty(), "unsupported result must be atomic");
                        }
                        // Two known terminal differences have stronger oracles than byte parity.
                        // The compiler tests independently authenticate the original branch/return
                        // histories; here bind the same source, ordered witnesses and publication
                        // outcomes through the actual configured owner.
                        let structural_contract = match (*name, scope) {
                            ("nested-binary-unit" | "affine-snapshot-symbolic", ScalarCheckerScope::MainFlowOnly) => {
                                assert_eq!(baseline, (Ok(0), Vec::new(), Vec::new(), BTreeMap::new()));
                                assert_eq!(observed, baseline, "a dormant callable cannot leak into main flow");
                                None
                            }
                            ("nested-binary-unit", ScalarCheckerScope::CurrentPackage) => {
                                let [old] = incumbent[0].outcome.as_ref().unwrap().directives.as_slice()
                                    else { panic!("incumbent merges the two histories"); };
                                let records = &candidate[0].outcome.as_ref().unwrap().directives;
                                assert_eq!(records.len(), if cap == 1 { 1 } else { 2 });
                                assert_eq!(baseline, (Ok(7), b"after".to_vec(), Vec::new(), BTreeMap::new()));
                                assert_eq!(observed, (Ok(7), b"after".repeat(records.len()), Vec::new(), BTreeMap::new()));
                                for (row, (lo, hi)) in records.iter().zip([(0, 127), (128, 255)]) {
                                    assert_eq!((&row.source, row.directive, row.condition), (&old.source, old.directive, old.condition));
                                    assert_eq!(&program[row.condition.start as usize..row.condition.end as usize], "b != 3");
                                    assert_eq!(row.witness.row_count(), 1);
                                    assert!(row.witness.intervals_at(0, 0).unwrap().eq([(&lo.into(), &hi.into())]));
                                    assert!(row.witness.intervals_at(0, 1).unwrap().eq([(&3.into(), &3.into())]));
                                }
                                (cap != 1).then_some("verified-distinct-second-history")
                            }
                            ("affine-snapshot-symbolic", ScalarCheckerScope::CurrentPackage) => {
                                let path = Path::new(".wosy/outputs/check/state.json");
                                let [old] = incumbent[0].outcome.as_ref().unwrap().directives.as_slice()
                                    else { panic!("incumbent merges the two affine selections"); };
                                let records = &candidate[0].outcome.as_ref().unwrap().directives;
                                assert_eq!(records.len(), if cap == 1 { 1 } else { 2 });
                                assert_eq!((&baseline.0, baseline.1.as_slice(), baseline.2.as_slice()),
                                    (&Ok(59), b"".as_slice(), b"construction".as_slice()));
                                assert_eq!(baseline.3.len(), 1);
                                let first_file = records[0].files()[0].text.to_vec();
                                assert_ne!(baseline.3[path], first_file);
                                for (row, value) in records.iter().zip([1, 3]) {
                                    assert_eq!((&row.source, row.directive, row.condition), (&old.source, old.directive, old.condition));
                                    assert_eq!(&program[row.condition.start as usize..row.condition.end as usize], "value != 7");
                                    assert_eq!(row.witness.axes().len(), 1);
                                    let [file] = row.files() else { panic!("one file in this history"); };
                                    assert_eq!(file.path.as_ref(), "state.json");
                                    let snapshot: serde_json::Value = serde_json::from_slice(&file.text).unwrap();
                                    assert_eq!(snapshot["value"]["plain"]["value"], value);
                                    assert_eq!(snapshot["value"]["raised"]["value"], value + 2);
                                    assert_eq!(snapshot["value"]["reflected"]["value"], 7 - value);
                                }
                                if cap == 1 {
                                    assert_eq!(observed, (Ok(59), Vec::new(), b"construction".to_vec(),
                                        BTreeMap::from([(path.to_owned(), first_file)])));
                                    Some("verified-first-affine-history")
                                } else {
                                    assert_eq!(observed, (Err("check: result requires unsupported output; no result published".to_owned()),
                                        Vec::new(), Vec::new(), BTreeMap::new()));
                                    assert!(!root.join(".wosy/outputs/check").exists());
                                    Some("verified-distinct-affine-duplicate-refusal")
                                }
                            }
                            ("ordinary-context-restore", ScalarCheckerScope::MainFlowOnly) => {
                                assert_eq!(baseline, (Ok(0), Vec::new(), Vec::new(), BTreeMap::new()));
                                assert_eq!(observed, baseline, "uninvoked caller is outside main flow");
                                None
                            }
                            ("ordinary-context-restore", ScalarCheckerScope::CurrentPackage) => {
                                let records = &candidate[0].outcome.as_ref().unwrap().directives;
                                let old = &incumbent[0].outcome.as_ref().unwrap().directives;
                                let [inner, restored] = old.as_slice() else { panic!("both original lexical paths"); };
                                assert_eq!(baseline, (Ok(51), b"innerrestored".to_vec(), Vec::new(), BTreeMap::new()));
                                assert_eq!(records.as_slice(), &old[..if cap == 1 { 1 } else { 2 }]);
                                let function = program.find("unit(i8) caller").unwrap();
                                let branch = program.find("if (input == 2)").unwrap();
                                let inner_decl = program.find("i8 core.context.count = core.context.count").unwrap();
                                let outer_decl = program.find("i8 core.context.count = input").unwrap();
                                let assert_context_axis = |row: &wosy_compiler::ScalarOrderedCheckerFailure,
                                    declaration: usize, value: i32| {
                                    let at = declaration as u32..(declaration + "i8 core.context.count".len()) as u32;
                                    let (axis, _) = row.witness.axes().enumerate().find(|(_, (binding, _))|
                                        binding.source == row.source && at.contains(&binding.declaration_span.start))
                                        .expect("exact contextual binding witness");
                                    assert!(row.witness.intervals_at(0, axis).unwrap().eq([(&value.into(), &value.into())]));
                                };
                                for (row, condition, code) in [(inner, "core.context.count == 2", 51), (restored, "core.context.count == 7", 52)] {
                                    assert_eq!(row.source.path, "src/main.w");
                                    assert!(function < branch && branch < row.condition.start as usize);
                                    assert_eq!(&program[row.condition.start as usize..row.condition.end as usize], condition);
                                    assert_eq!(row.code(), &code.into());
                                    assert_eq!(row.witness.row_count(), 1);
                                    assert!(row.witness.intervals_at(0, 0).unwrap().eq([(&2.into(), &2.into())]));
                                    assert_context_axis(row, outer_decl, 2);
                                }
                                assert_context_axis(inner, inner_decl, 7);
                                assert!(!restored.witness.axes().any(|(binding, _)|
                                    (inner_decl..inner_decl + "i8 core.context.count".len()).contains(&(binding.declaration_span.start as usize))));
                                if cap == 1 {
                                    assert_eq!(observed, (Ok(51), b"inner".to_vec(), Vec::new(), BTreeMap::new()));
                                    Some("verified-first-context-restore")
                                } else {
                                    assert_eq!(observed, baseline);
                                    None
                                }
                            }
                            _ => None,
                        };
                        // A known first authored path explains the terminal
                        // difference only if the formal diagnostics are also
                        // retained exactly and the omitted suffix is real.
                        let first_condition = match *name {
                            "typed-bool-startup" => Some("entry != 3"),
                            "configured-transitive-dependencies" | "mixed-contextual-named" | "mixed-contextual-refutation" => Some("false"),
                            "checked-loan-effects" => Some("local == 0"),
                            "branching-array-proof" | "array-rank-mixed" => Some("head == 8"),
                            _ => None,
                        };
                        let verified_cap1 = if cap == 1 && observed != baseline
                            && !incumbent_refused && !candidate_refused
                            && first_condition.is_some()
                        {
                            let report_records = |reports: &[configured_check::ConfiguredCheckReceipt]| {
                                reports.iter().map(|row| row.outcome.as_ref().unwrap().directives.len()).sum::<usize>()
                            };
                            let first = &candidate[0].outcome.as_ref().unwrap().directives;
                            let initial = &incumbent[0].outcome.as_ref().unwrap().directives;
                            let diagnostic_signature = |reports: &[configured_check::ConfiguredCheckReceipt]| {
                                reports.iter().flat_map(|row|
                                    ordered_check_diagnostics(row.outcome.as_ref().unwrap(), &row.texts).unwrap()
                                ).collect::<Vec<_>>()
                            };
                            let [selected] = first.as_slice() else { panic!("{name} cap1 must select one real path"); };
                            let expected_condition = first_condition.unwrap();
                            assert_eq!(selected.source.path, "src/main.w", "{name}");
                            assert_eq!(&program[selected.condition.start as usize..selected.condition.end as usize],
                                expected_condition, "{name}");
                            assert_eq!(selected, &initial[0], "{name} original typed witness/first output");
                            assert_eq!((selected.stdout(), selected.stderr(), selected.code()),
                                (b"startup".as_slice(), b"".as_slice(), &0.into()), "{name}");
                            assert_eq!(report_records(&candidate), 1, "{name}");
                            assert!(report_records(&incumbent) > 1, "{name} must have real omitted records");
                            let prior_diagnostics = diagnostic_signature(&incumbent);
                            assert_eq!(diagnostic_signature(&candidate), prior_diagnostics, "{name} formal proof diagnostics");
                            assert_eq!(observed.0, Ok(u8::from(!prior_diagnostics.is_empty())), "{name} authored zero and proof status");
                            assert_eq!(observed.1, b"startup", "{name} actual selected output");
                            assert!(observed.3.is_empty(), "{name} unselected files");
                            if dependencies {
                                assert_eq!(baseline.3, BTreeMap::from([
                                    (".wosy/outputs/check/dep.txt".into(), b"dep".to_vec()),
                                    (".wosy/outputs/check/leaf.txt".into(), b"leaf".to_vec()),
                                ]));
                            }
                            true
                        } else { false };
                        let label = if let Some(contract) = structural_contract {
                            contract
                        } else if verified_cap1 {
                            "verified-first-authored-cap1"
                        } else if incumbent_refused && candidate_refused {
                            "both-refused"
                        } else if incumbent_refused {
                            "incumbent-refused-candidate-open"
                        } else if candidate_refused {
                            "release-blocker"
                        } else if observed == baseline {
                            "observed-terminal-byte-parity"
                        } else if cap == 1 {
                            "cap1-terminal-difference"
                        } else {
                            "cap64-terminal-difference"
                        };
                        // These are terminal observations, not a proof of
                        // structural path parity or exhaustion at cap 64.
                        eprintln!("CHECK-MATRIX {name} {layout} {scope:?} cap={cap} {label} status_match={status_match} old={old_status:?}/{:?} new={new_status:?}/{:?} stdout={}/{} stderr={}/{} files={:?}/{:?} preparation_check_us={}/{} publication_us={}/{}",
                            baseline.0, observed.0, baseline.1.len(), observed.1.len(), baseline.2.len(), observed.2.len(),
                            baseline.3.iter().map(|(path, data)| (path, data.len())).collect::<Vec<_>>(),
                            observed.3.iter().map(|(path, data)| (path, data.len())).collect::<Vec<_>>(),
                            old_load.as_micros(), load.as_micros(), old_publish.as_micros(), emit.as_micros());
                        *classes.entry(label).or_default() += 1;
                        samples += 1;
                    }
                }
            }
        }
        assert_eq!(samples, 136);
        eprintln!("CHECK-MATRIX-SUMMARY {classes:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(feature = "internal-checker", target_os = "linux"))]
    #[test]
    fn private_configured_terminal_preserves_proofs_files_status_and_atomicity() {
        use std::num::NonZeroUsize;
        use codespan_reporting::term::termcolor::Buffer;
        let (fixture, _) = routing_test_project("private-configured-terminal", &[]);
        let root = &fixture.root;
        let manifest = |package: &str, output: &str| format!(r#"
[package]
name = "{package}"
default_target = "main"
source_root = "src"
[comments]
categories = []
[builders]
[runners]
[targets.main]
root = "src/main.w"
default_profile = "dev"
[targets.main.artifacts.main]
artifact = "RuntimeStdio"
[targets.main.artifacts.main.profiles.dev]
backend = "llvm"
output = "{output}"
builder = "never"
run_runner = "never"
[targets.main.profiles.dev]
main_artifact = "main"
"#);
        let authored = |code: u8, packet: &str| format!(r#"{{ .stdout = "{packet}"; .stderr = "[{packet}]"; .code = {code};
            .files = [{{ .path = "{packet}.txt"; .text = "file:{packet}"; }}]; }}"#);
        let first = authored(0, "first");
        let second = authored(19, "second");
        let ranked = format!(r#"%%start
            u8(u8) descend = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend(value - 3) }} }};
            u8(u8) echo = fn(value) {{ value }};
            proof live for echo(entry) -> (out) where (entry == 7) ensures (out == 8) {{}}
            proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {{
                if (entry < 3) {{ core.show(out == 4); }} else {{ core.use(rank(entry - 3)); }}
            }}
            u8 recursed = descend(8);
            u8 actual = echo(7);
            core.require(recursed == 5, {first});
            core.require(actual == 8, {second});
        %%end"#);
        let context = format!(r#"%%start
            unit(u8) consume = fn(charged) {{
                if (charged == 1) {{ core.context.credit = core.context.credit - 1; }}
                else {{ core.context.credit = core.context.credit - 1; }};
            }};
            unit() entry = fn {{
                u8 core.context.credit = 3;
                consume(1);
                core.require(false, {first});
                core.require(false, {second});
            }};
            proof debit for consume(cost) -> (null) where (cost <= 1)
                ensures (core.context.credit == core.old(core.context.credit) - 1) {{}}
            entry();
        %%end"#);
        fs::create_dir_all(root.join("src")).unwrap();
        let source = root.join("src/main.w");
        let state = root.join(".wosy");
        let reports_path = state.join("outputs/check");
        let terminal = |receipts: &[configured_check::ConfiguredCheckReceipt], scope, dependencies| {
            let (mut out, mut err) = (Vec::new(), Buffer::no_color());
            let code = publish_configured_check_receipts(receipts, scope, dependencies, &mut out, &mut err);
            (code, out, err.as_slice().to_vec())
        };
        let clean = || {
            if state.exists() { fs::remove_dir_all(&state).unwrap(); }
        };
        let files = || {
            let mut result = BTreeMap::new();
            if reports_path.exists() {
                for entry in fs::read_dir(&reports_path).unwrap() {
                    let entry = entry.unwrap();
                    result.insert(entry.file_name().into_string().unwrap(), fs::read(entry.path()).unwrap());
                }
            }
            result
        };
        let first_file = BTreeMap::from([("first.txt".to_owned(), b"file:first".to_vec())]);
        let both_files = BTreeMap::from([
            ("first.txt".to_owned(), b"file:first".to_vec()), ("second.txt".to_owned(), b"file:second".to_vec()),
        ]);
        for layout in ["native", "wasm-wasip1"] {
            fs::write(root.join("wosy.toml"), manifest("app", layout)).unwrap();
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let selected = |cap| configured_check::check_target_with_private_authored_limit(
                    root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
                ).ok().unwrap();
                let legacy = || match scope {
                    ScalarCheckerScope::CurrentPackage => configured_check::check_declared_package_target(root, None, "dev"),
                    ScalarCheckerScope::MainFlowOnly => configured_check::check_target(root, None, "dev", scope, Vec::new()),
                }.ok().unwrap();
                let named_ordinary = format!(r#"%%start
                    u8(u8) named = fn(value) {{ value }};
                    proof reached for named(entry) -> (out) where (entry == 3) ensures (out == 4) {{}}
                    unit(u8) packet = fn(entry) {{
                        if (entry == 7) {{
                            core.require(false, {{ .stdout = "packet"; .stderr = ""; .code = 23;
                                .files = [{{ .path = "packet.json"; .state = entry; }}]; }});
                        }};
                    }};
                    u8 actual = named(3);
                    core.require(actual == 4, {first});
                %%end"#);
                fs::write(&source, &named_ordinary).unwrap();
                let old = legacy();
                let old_printed = terminal(&[old], scope, false);
                let old_files = files();
                assert_eq!(old_files.get("first.txt"), first_file.get("first.txt"));
                let during = scope == ScalarCheckerScope::CurrentPackage;
                assert_eq!(old_files.contains_key("packet.json"), during);
                if during {
                    assert_eq!(old_files["packet.json"], b"{\"type\":{\"kind\":\"u8\"},\"value\":7}\n");
                }
                clean();
                let exact = selected(1);
                let report = exact[0].outcome.as_ref().unwrap();
                assert_eq!(report.directives.len(), 1);
                assert_eq!(report.diagnostics.len(), 1);
                let hit = report.diagnostics[0].invocation.call.range;
                assert_eq!(&named_ordinary[hit.start as usize..hit.end as usize], "named(3)");
                let first_only = terminal(&exact, scope, false);
                assert_eq!(first_only.1, b"first");
                assert_eq!(files(), first_file);
                clean();
                assert_eq!(terminal(&selected(4), scope, false), old_printed);
                assert_eq!(files(), old_files);
                clean();
                let duplicate = named_ordinary.replace("packet.json", "first.txt");
                fs::write(&source, duplicate).unwrap();
                if during {
                    let rejected = terminal(&selected(4), scope, false);
                    assert!(rejected.0.is_err() && rejected.1.is_empty());
                    assert!(!state.exists());
                    assert_eq!(terminal(&selected(1), scope, false), first_only);
                    assert_eq!(files(), first_file);
                    clean();
                }

                let graph_root = format!(r#"%%start
                    shared = namespace app "shared.w";
                    relay = namespace app "relay.w";
                    alias = namespace app "shared.w";
                    core.require(false, {second});
                %%end"#);
                let relay_output = authored(11, "relay");
                let unused_output = authored(23, "unused");
                let shared = format!(r#"%%start
                    u8(u8) echo = fn(input) {{ input }};
                    u8 actual = echo(7);
                    core.require(actual == 8, {first});
                    unit(u8) dormant = fn(input) {{ core.require(input == 0, {unused_output}); }};
                %%end"#);
                let relay = format!(r#"%%start
                    nested = namespace app "shared.w";
                    core.require(false, {relay_output});
                %%end"#);
                let shared_source = root.join("src/shared.w");
                let relay_source = root.join("src/relay.w");
                fs::write(&source, &graph_root).unwrap();
                fs::write(&shared_source, &shared).unwrap();
                fs::write(&relay_source, &relay).unwrap();
                let expected = if scope == ScalarCheckerScope::CurrentPackage {
                    vec!["src/shared.w", "src/relay.w", "src/main.w", "src/shared.w"]
                } else { vec!["src/shared.w", "src/relay.w", "src/main.w"] };
                let legacy_graph = terminal(&[legacy()], scope, false);
                assert_eq!(legacy_graph.0, Ok(11));
                clean();
                let clipped = terminal(&selected(1), scope, false);
                assert_eq!(clipped, (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                let full = selected(8);
                let directives = &full[0].outcome.as_ref().unwrap().directives;
                assert_eq!(directives.iter().map(|record| record.source.path.as_str()).collect::<Vec<_>>(), expected);
                assert_eq!(terminal(&full, scope, false), legacy_graph);
                clean();
                // The shared visit and original result are retained, but
                // an unsupported condition still cannot authorize full search.
                fs::write(&relay_source, relay.replace("core.require(false,", "core.require(false || true,")).unwrap();
                let unsupported = selected(8);
                assert!(matches!(unsupported[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());
                fs::remove_file(relay_source).unwrap();

                let qualified = format!(r#"%%start
                    lib = namespace app "shared.w";
                    u8(u8) relay = fn(input) {{ u8 forwarded = lib.echo(input); forwarded }};
                    u8 direct = lib.echo(7);
                    core.require(direct == 7, {unused_output});
                    core.require(direct == 8, {first});
                    u8 nested = relay(8);
                    core.require(nested == 8, {unused_output});
                    core.require(nested == 9, {second});
                %%end"#);
                fs::write(&source, &qualified).unwrap();
                fs::write(&shared_source, "%%start u8(u8) echo = fn(input) { input }; %%end").unwrap();
                assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                let complete = terminal(&selected(8), scope, false);
                assert_eq!(complete, (Ok(19), b"firstsecond".to_vec(), b"[first][second]".to_vec()));
                assert_eq!(files(), both_files);
                clean();
                // A reached qualified leaf with non-admitted ordinary
                // work cannot acquire authority through the lexical wrapper.
                fs::write(&shared_source, format!(r#"%%start
                    u8(u8) echo = fn(input) {{ core.require(input == 8, {relay_output}); input }};
                %%end"#)).unwrap();
                let refused = selected(8);
                assert!(matches!(refused[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(files().is_empty() && !state.exists());
                fs::write(&source, r#"%%start lib = namespace app "shared.w"; lib.report(7); %%end"#).unwrap();
                fs::write(&shared_source, format!(r#"%%start
                    unit(u8) report = fn(input) {{ core.require(input == 8, {first}); }};
                %%end"#)).unwrap();
                assert_eq!(terminal(&selected(8), scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                fs::remove_file(&shared_source).unwrap();
                let checked_cells = format!(r#"%%start
                    unit(*!u32) advance = fn(cell) {{ *cell = *cell +@ 1; }};
                    proof epoch for advance(p) -> () ensures (*p == core.old(*p) +@ 1) {{}}
                    unit() start = fn {{
                        u32 local = 4294967295;
                        advance(&!local);
                        advance(&!local);
                        core.require(local == 1, {unused_output});
                        core.require(local == 0, {first});
                        core.require(local == 2, {second});
                    }};
                    start();
                %%end"#);
                fs::write(&source, &checked_cells).unwrap();
                let old_pointer = terminal(&[legacy()], scope, false);
                assert_eq!(old_pointer.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                let first_pointer = selected(1);
                let report = first_pointer[0].outcome.as_ref().unwrap();
                let [record] = report.directives.as_slice() else { panic!("selection after the two authenticated updates"); };
                let range = record.condition;
                assert_eq!(&checked_cells[range.start as usize..range.end as usize], "local == 0");
                assert!(record.witness.intervals_at(0, 0).unwrap().eq([(&1.into(), &1.into())]));
                assert_eq!(terminal(&first_pointer, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                assert_eq!(terminal(&selected(8), scope, false), old_pointer);
                assert_eq!(files(), both_files);
                clean();
                let after_limit = checked_cells.replace("start();", &format!("core.require(false, {first}); start();"));
                fs::write(&source, &after_limit).unwrap();
                assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                // A false theorem over otherwise valid checked loans does
                // not justify either update, even after an earlier cap.
                fs::write(&source, after_limit.replace("core.old(*p) +@ 1", "core.old(*p) +@ 2")).unwrap();
                let refused = selected(1);
                assert!(matches!(refused[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());

                let qualified_rank = format!(r#"%%start
                    lib = namespace app "shared.w";
                    u8 base = lib.descend(2);
                    u8 recursed = lib.descend(8);
                    core.require(base == 4, {unused_output});
                    core.require(recursed == 4, {unused_output});
                    core.require(false, {second});
                %%end"#);
                let ranked_provider = format!(r#"%%start
                    u8(u8) descend = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend(value - 3) }} }};
                    proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {{
                        if (entry < 3) {{ core.show(out == 4); }} else {{ core.use(rank(entry - 3)); }}
                    }}
                    core.require(false, {first});
                %%end"#);
                fs::write(&source, &qualified_rank).unwrap();
                fs::write(&shared_source, ranked_provider).unwrap();
                let incumbent = terminal(&[legacy()], scope, false);
                assert_eq!(incumbent, (Ok(19), b"firstsecond".to_vec(), b"[first][second]".to_vec()));
                assert_eq!(files(), both_files);
                clean();
                for cap in [1, 2, 3] {
                    let receipts = selected(cap);
                    let report = receipts[0].outcome.as_ref().unwrap();
                    assert!(report.active && report.symbolic_diagnostic.is_none() && report.diagnostics.is_empty());
                    let paths = report.directives.iter().map(|record| record.source.path.as_str()).collect::<Vec<_>>();
                    assert_eq!(paths, ["src/shared.w", "src/main.w"][..cap.min(2)]);
                    if cap == 1 {
                        assert_eq!(terminal(&receipts, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                        assert_eq!(files(), first_file);
                    } else {
                        assert_eq!(terminal(&receipts, scope, false), incumbent);
                        assert_eq!(files(), both_files);
                    }
                    clean();
                }
                fs::write(&source, qualified_rank.replace("descend(8)", "descend(9)")).unwrap();
                let out_of_guard = selected(1);
                assert!(matches!(out_of_guard[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                let mut stdout = Vec::new();
                let mut stderr = Buffer::no_color();
                assert!(publish_configured_check_receipts(&out_of_guard, scope, false, &mut stdout, &mut stderr).is_err());
                assert!(stdout.is_empty() && stderr.as_slice().is_empty() && !state.exists());
                fs::remove_file(&shared_source).unwrap();

                let additional_source = root.join("src/extra.w");
                let roster_root = format!(r#"%%start
                    lib = namespace app "shared.w";
                    second_lib = namespace app "extra.w";
                    u8 actual_left = lib.echo(3);
                    u8 actual_right = second_lib.echo(5);
                    core.require(actual_left == 3, {unused_output});
                    core.require(actual_right == 5, {unused_output});
                    core.require(false, {{ .stdout = "root"; .stderr = "[root]"; .code = 23; .files = []; }});
                %%end"#);
                let provider = |output: &str| format!(r#"%%start
                    u8(u8) echo = fn(value) {{ value }};
                    proof exact for echo(entry) -> (out) where (entry <= 5) ensures (out == 8) {{}}
                    core.require(false, {output});
                %%end"#);
                fs::write(&source, &roster_root).unwrap();
                fs::write(&shared_source, provider(&first)).unwrap();
                fs::write(&additional_source, provider(&second)).unwrap();
                let incumbent = legacy();
                let expected = incumbent.outcome.as_ref().unwrap();
                assert_eq!(expected.directives.len(), 3);
                assert_eq!(expected.diagnostics.len(), 2);
                let complete = terminal(std::slice::from_ref(&incumbent), scope, false);
                assert_eq!(complete.0, Ok(19));
                assert_eq!(complete.1, b"firstsecondroot");
                assert_eq!(files(), both_files);
                clean();
                for cap in [1, 2, 3, 4] {
                    let receipts = selected(cap);
                    let actual = receipts[0].outcome.as_ref().unwrap();
                    assert_eq!(actual.directives, expected.directives[..cap.min(3)]);
                    assert_eq!(actual.directives.iter().map(|item| item.source.path.as_str()).collect::<Vec<_>>(),
                        ["src/shared.w", "src/extra.w", "src/main.w"][..cap.min(3)]);
                    assert_eq!(actual.diagnostics.len(), 2);
                    for (diagnostic, (path, argument, returned)) in actual.diagnostics.iter().zip([
                        ("src/shared.w", "lib.echo(3)", 3), ("src/extra.w", "second_lib.echo(5)", 5),
                    ]) {
                        assert_eq!(diagnostic.proof.source.path, path);
                        assert_eq!(diagnostic.invocation.call.source.path, "src/main.w");
                        let span = diagnostic.invocation.call.range;
                        assert_eq!(&roster_root[span.start as usize..span.end as usize], argument);
                        assert_eq!(diagnostic.invocation.returned, returned.into());
                    }
                    let observed = terminal(&receipts, scope, false);
                    match cap {
                        1 => { assert_eq!(observed.0, Ok(1)); assert_eq!(observed.1, b"first"); assert_eq!(files(), first_file); }
                        2 => { assert_eq!(observed.0, Ok(19)); assert_eq!(observed.1, b"firstsecond"); assert_eq!(files(), both_files); }
                        _ => { assert_eq!(observed, complete); assert_eq!(files(), both_files); }
                    }
                    clean();
                }
                fs::remove_file(&shared_source).unwrap();
                fs::remove_file(&additional_source).unwrap();

                // The incumbent's observer keeps one pure-affine
                // target at a time. Compare independently admitted
                // profiles rather than assume their arbitrary union.
                for (ty, implementation, contract, input, real_result, returned) in [
                    ("i8", "value", "where (entry >= -4) ensures (out >= -1)", "-2", "actual < 0", -2),
                    ("u8", "value", "where (entry == 5) ensures (out != 5)", "5", "actual == 5", 5),
                    ("u8", "u8 next = value + 1; next", "where (entry >= 1 && entry <= 8) ensures (out == core.old(entry))", "7", "actual == 8", 8),
                    ("u8", "u8 next = value + 1; next", "where (entry >= 1 && entry <= 8) ensures (out == core.old(entry) + 2)", "8", "actual == 9", 9),
                ] {
                    let declaration = format!(r#"
                        {ty}({ty}) calculate = fn(value) {{ {implementation} }};
                        proof result for calculate(entry) -> (out) {contract} {{}}
                    "#);
                    let scalar_relations = format!(r#"%%start
                        {declaration}
                        core.require(false, {first});
                        {ty} actual = calculate({input});
                        core.require({real_result}, {unused_output});
                        core.require(false, {second});
                    %%end"#);
                    fs::write(&source, &scalar_relations).unwrap();
                    let baseline = legacy();
                    let old = baseline.outcome.as_ref().unwrap_or_else(|_| panic!("incumbent literal/entry profile: {contract}"));
                    assert_eq!(old.directives.len(), 2);
                    assert_eq!(old.diagnostics.len(), 1);
                    let before = terminal(std::slice::from_ref(&baseline), scope, false);
                    assert_eq!(before.0, Ok(19));
                    assert_eq!(before.1, b"firstsecond");
                    assert_eq!(files(), both_files);
                    clean();
                    for cap in [1, 2, 3] {
                        let receipts = selected(cap);
                        let actual = receipts[0].outcome.as_ref().unwrap_or_else(|_| panic!("private literal/entry profile: {contract}"));
                        assert_eq!(actual.directives, old.directives[..cap.min(2)]);
                        let [new] = actual.diagnostics.as_slice() else { panic!("actual proof after cap: {contract}"); };
                        let prior = &old.diagnostics[0];
                        assert_eq!(new.proof, prior.proof);
                        assert_eq!(new.goal, prior.goal);
                        assert_eq!(new.invocation.call, prior.invocation.call);
                        assert_eq!(new.invocation.returned, returned.into());
                        let span = new.invocation.call.range;
                        assert_eq!(&scalar_relations[span.start as usize..span.end as usize], format!("calculate({input})"));
                        let printed = terminal(&receipts, scope, false);
                        if cap == 1 {
                            assert_eq!(printed.0, Ok(1));
                            assert_eq!(printed.1, b"first");
                            assert_eq!(files(), first_file);
                        } else {
                            assert_eq!(printed, before);
                            assert_eq!(files(), both_files);
                        }
                        clean();
                    }
                    let dormant = format!("%%start {declaration} core.require(false, {first}); core.require(false, {second}); %%end");
                    fs::write(&source, dormant).unwrap();
                    let baseline = legacy();
                    let old = baseline.outcome.as_ref().unwrap_or_else(|_| panic!("incumbent dormant profile: {contract}"));
                    for cap in [1, 3] {
                        let receipts = selected(cap);
                        let actual = receipts[0].outcome.as_ref().unwrap_or_else(|_| panic!("private dormant profile: {contract}"));
                        assert_eq!(actual.directives, old.directives[..cap.min(2)]);
                        assert!(actual.diagnostics.is_empty());
                        let observed = actual.symbolic_diagnostic.as_ref();
                        assert_eq!(observed.is_some(), scope == ScalarCheckerScope::CurrentPackage);
                        if let Some(observed) = observed {
                            let prior = old.symbolic_diagnostic.as_ref().unwrap();
                            assert_eq!((&observed.proof, &observed.goal, &observed.returned),
                                (&prior.proof, &prior.goal, &prior.returned));
                            assert_eq!(observed.directive_position, cap.min(2));
                        }
                    }
                }
                let sparse_exists = format!(r#"%%start
                    u8(u8) identity = fn(value) {{ value }};
                    proof sparse for identity(entry) -> (out) where (entry == 3)
                        ensures (exists (u8 chosen where chosen == 2 || chosen == 4) {{ out == chosen }}) {{}}
                    core.require(false, {first});
                    u8 actual = identity(3);
                    core.require(actual == 3, {unused_output});
                    core.require(false, {second});
                %%end"#);
                fs::write(&source, sparse_exists).unwrap();
                let baseline = legacy();
                let old = baseline.outcome.as_ref().unwrap();
                assert_eq!(old.directives.len(), 2);
                let complete = terminal(std::slice::from_ref(&baseline), scope, false);
                assert_eq!(complete.0, Ok(19));
                assert_eq!(complete.1, b"firstsecond");
                assert_eq!(files(), both_files);
                clean();
                for cap in [1, 2, 3] {
                    let receipts = selected(cap);
                    let report = receipts[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives, old.directives[..cap.min(2)]);
                    let [failure] = report.diagnostics.as_slice() else { panic!("one exact sparse existential refutation"); };
                    assert_eq!(failure.invocation.call, old.diagnostics[0].invocation.call);
                    assert_eq!(failure.invocation.returned, 3.into());
                    assert_eq!(failure.invocation.directive_position, 1);
                    let shown = terminal(&receipts, scope, false);
                    if cap == 1 {
                        assert_eq!((shown.0, shown.1), (Ok(1), b"first".to_vec()));
                        assert_eq!(files(), first_file);
                    } else {
                        assert_eq!(shown, complete);
                        assert_eq!(files(), both_files);
                    }
                    clean();
                }

                // Do not silently enable this newly enrolled goal in a provider catalog.
                fs::write(&source, &roster_root).unwrap();
                fs::write(&shared_source, provider(&first).replace("out == 8", "out >= 8")).unwrap();
                fs::write(&additional_source, provider(&second)).unwrap();
                let outside = selected(1);
                assert!(matches!(outside[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());
                fs::remove_file(&shared_source).unwrap();
                fs::remove_file(&additional_source).unwrap();

                let aggregate = format!(r#"%%start
                    u8[2]() make = fn {{ [7, 1] }};
                    proof cells for make() -> (items)
                        forall (u64 i where i == 0)
                        ensures (items[i] == 8)
                    {{ core.show(items[i] == 8); }}
                    u8[2] first_array = make();
                    u8 head = first_array[0];
                    core.require(head == 8, {first});
                    u8[2] second_array = make();
                    u8 tail = second_array[1];
                    core.require(tail == 2, {second});
                %%end"#);
                fs::write(&source, &aggregate).unwrap();
                let legacy_array = terminal(&[legacy()], scope, false);
                assert_eq!(legacy_array.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                let limited = selected(1);
                let report = limited[0].outcome.as_ref().unwrap();
                let [a, b] = report.aggregate_diagnostics.as_slice() else { panic!("both real fixed-array proof observations"); };
                assert!(a.call.as_ref().unwrap().range.start < b.call.as_ref().unwrap().range.start);
                for diagnostic in [a, b] {
                    let cell = diagnostic.cell.as_ref().unwrap();
                    assert_eq!((cell.index.to_string(), cell.value.to_string()), ("0".into(), "7".into()));
                }
                let clipped = terminal(&limited, scope, false);
                assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                assert_eq!(clipped.2.strip_prefix(b"[first]"), legacy_array.2.strip_prefix(b"[first][second]"));
                assert_eq!(files(), first_file);
                clean();
                assert_eq!(terminal(&selected(8), scope, false), legacy_array);
                assert_eq!(files(), both_files);
                clean();
                let proved = aggregate.replace("items[i] == 8", "items[i] == 7");
                fs::write(&source, proved).unwrap();
                let clipped = terminal(&selected(1), scope, false);
                assert_eq!(clipped, (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                // A non-admitted narrowing/opaque condition cannot
                // manufacture a missing adverse proof observation.
                fs::write(&source, aggregate.replace("u8[2] second_array = make();", &format!(
                    "core.narrow(false, {second}); u8[2] second_array = make();"))).unwrap();
                let refused = selected(1);
                assert!(matches!(refused[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());

                let branched = aggregate.replace("make = fn { [7, 1] };",
                    "make = fn(input) { if (input == 0) { [7, 1] } else { [1, 7] } };")
                    .replace("u8[2]() make", "u8[2](u8) make").replace("= make();", "= make(0);")
                    .replace("proof cells for make()", "proof cells for make(entry)")
                    .replace("second_array = make(0)", "second_array = make(1)");
                assert_ne!(branched, aggregate);
                fs::write(&source, &branched).unwrap();
                let old_coverage = terminal(&[legacy()], scope, false);
                assert_eq!(old_coverage.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                let clipped = selected(1);
                let report = clipped[0].outcome.as_ref().unwrap();
                let observed = report.aggregate_diagnostics.iter().map(|failure| {
                    let call = failure.call.as_ref().unwrap().range;
                    (&branched[call.start as usize..call.end as usize], failure.cell.as_ref().unwrap().value.to_string())
                }).collect::<Vec<_>>();
                assert_eq!(observed, [("make(0)", "7".into()), ("make(1)", "1".into())]);
                let clipped = terminal(&clipped, scope, false);
                assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                assert_eq!(clipped.2.strip_prefix(b"[first]"), old_coverage.2.strip_prefix(b"[first][second]"));
                assert_eq!(files(), first_file);
                clean();
                assert_eq!(terminal(&selected(8), scope, false), old_coverage);
                assert_eq!(files(), both_files);
                clean();

                let mixed_catalog = format!(r#"%%start
                    u8[2]() make = fn {{ [7, 1] }};
                    u8(u8) echo = fn(value) {{ value }};
                    proof cells for make() -> (items)
                        forall (u64 i where i == 0) ensures (items[i] == 8)
                    {{ core.show(items[i] == 8); }}
                    proof scalar for echo(entry) -> (out) where (entry == 7) ensures (out == 8) {{}}
                    u8[2] input = make();
                    u8 cell = input[0];
                    core.require(cell == 8, {first});
                    u8 returned = echo(7);
                    core.require(returned == 8, {second});
                %%end"#);
                fs::write(&source, mixed_catalog.replace("ensures (out == 8)", "ensures (out >= 8)")).unwrap();
                let excluded_relation = selected(1);
                assert!(matches!(excluded_relation[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());
                for proof_scope in [0, 1, 2] {
                    let both = match proof_scope {
                        1 => mixed_catalog.replace("u8 returned = echo(7);", "u8 returned = 7;"),
                        2 => mixed_catalog.replace("u8[2] input = make();", "").replace("u8 cell = input[0];", "u8 cell = 7;"),
                        _ => mixed_catalog.clone(),
                    };
                    fs::write(&source, both).unwrap();
                    let old_mixed = terminal(&[legacy()], scope, false);
                    assert_eq!(old_mixed.0, Ok(19));
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 1);
                    if proof_scope == 1 {
                        assert!(report.diagnostics.is_empty());
                        assert_eq!(report.symbolic_diagnostic.is_some(), scope == ScalarCheckerScope::CurrentPackage);
                    } else {
                        assert_eq!(report.diagnostics.len(), 1);
                    }
                    if proof_scope == 2 && scope == ScalarCheckerScope::MainFlowOnly {
                        assert!(report.aggregate_diagnostics.is_empty());
                    } else {
                        let [array] = report.aggregate_diagnostics.as_slice() else { panic!("one original aggregate claim"); };
                        assert_eq!(array.call.is_some(), proof_scope != 2);
                    }
                    let clipped = terminal(&limited, scope, false);
                    assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                    assert_eq!(clipped.2.strip_prefix(b"[first]"), old_mixed.2.strip_prefix(b"[first][second]"));
                    assert_eq!(files(), first_file);
                    clean();
                    assert_eq!(terminal(&selected(8), scope, false), old_mixed);
                    assert_eq!(files(), both_files);
                    clean();
                }
                // Both owned proof kinds can share the same real module
                // initialization, including a provider-produced array.
                let scalar_library = r#"%%start
                    u8(u8) echo = fn(value) { value };
                    proof scalar for echo(entry) -> (out) where (entry == 7) ensures (out == 8) {}
                %%end"#;
                let array_library = r#"
                    u8[2]() make = fn { [7, 1] };
                    proof cells for make() -> (items)
                        forall (u64 i where i == 0) ensures (items[i] == 8)
                    { core.show(items[i] == 8); }
                "#;
                for (array_in_provider, deferred) in [(false, false), (true, false), (true, true)] {
                    let provider_code = if array_in_provider {
                        scalar_library.replace("%%end", &format!("{array_library} %%end"))
                    } else { scalar_library.to_owned() };
                    let array_setup = match (array_in_provider, deferred) {
                        (_, true) => "u8 cell = 7;",
                        (true, false) => "u8[2] input = lib.make(); u8 cell = input[0];",
                        (false, false) => "u8[2] input = make(); u8 cell = input[0];",
                    };
                    let app_array = if array_in_provider { "" } else { array_library };
                    let app = format!(r#"%%start
                        lib = namespace app "shared.w";
                        {app_array}
                        {array_setup}
                        core.require(cell == 8, {first});
                        u8 returned = lib.echo(7);
                        core.require(returned == 8, {second});
                    %%end"#);
                    fs::write(&source, &app).unwrap();
                    fs::write(&shared_source, &provider_code).unwrap();
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old.0, Ok(19));
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 1);
                    let [scalar] = report.diagnostics.as_slice() else { panic!("actual qualified scalar proof"); };
                    assert_eq!(scalar.proof.source.path, "src/shared.w");
                    assert_eq!(scalar.invocation.call.source.path, "src/main.w");
                    let scalar_span = scalar.invocation.call.range;
                    assert_eq!(&app[scalar_span.start as usize..scalar_span.end as usize], "lib.echo(7)");
                    if deferred && scope == ScalarCheckerScope::MainFlowOnly {
                        assert!(report.aggregate_diagnostics.is_empty());
                    } else {
                        let [array] = report.aggregate_diagnostics.as_slice() else { panic!("original owned array proof"); };
                        assert_eq!(array.proof.source.path, if array_in_provider { "src/shared.w" } else { "src/main.w" });
                        assert_eq!(array.cell.as_ref().unwrap().value.to_string(), "7");
                        assert_eq!(array.call.is_none(), deferred);
                        if let Some(call) = &array.call {
                            let span = call.range;
                            assert_eq!(call.source.path, "src/main.w");
                            assert_eq!(&app[span.start as usize..span.end as usize], if array_in_provider { "lib.make()" } else { "make()" });
                        }
                    }
                    let clipped = terminal(&limited, scope, false);
                    assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                    assert_eq!(clipped.2.strip_prefix(b"[first]"), old.2.strip_prefix(b"[first][second]"));
                    assert_eq!(files(), first_file);
                    clean();
                    assert_eq!(terminal(&selected(8), scope, false), old);
                    assert_eq!(files(), both_files);
                    clean();
                    if array_in_provider && !deferred {
                        // A provider startup call is not an enrolled app
                        // activation, even when it resolves the same function.
                        fs::write(&shared_source, provider_code.replace("%%end",
                            &format!("u8[2] other = make(); u8 witness = other[0]; core.require(witness == 7, {unused_output}); %%end"))).unwrap();
                        let refused = selected(1);
                        assert!(matches!(refused[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                        assert!(!state.exists());
                    }
                }
                fs::remove_file(&shared_source).unwrap();
                // The array cohort keeps the exact owner for every bare
                // proof, even when an actual provider return and a local
                // return disagree with two independent claims after cap1.
                let multi_provider = scalar_library.replace("%%end", &format!("{array_library} %%end"));
                fs::write(&shared_source, &multi_provider).unwrap();
                for (dormant_local, dormant_array) in [(false, false), (true, false), (false, true)] {
                    let cell = if dormant_array { "u8 cell = 7;" }
                        else { "u8[2] input = lib.make(); u8 cell = input[0];" };
                    let local = if dormant_local { "u8 local_value = 5;" }
                        else { "u8 local_value = reflect(5);" };
                    let app = format!(r#"%%start
                        lib = namespace app "shared.w";
                        u8(u8) reflect = fn(value) {{ value }};
                        proof local_claim for reflect(entry) -> (out) where (entry == 5) ensures (out == 8) {{}}
                        {cell}
                        core.require(cell == 8, {first});
                        u8 external_value = lib.echo(7);
                        core.require(external_value == 7, {unused_output});
                        {local}
                        core.require(local_value == 8, {second});
                    %%end"#);
                    fs::write(&source, &app).unwrap();
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old.0, Ok(19), "legacy {layout} {scope:?} {dormant_local} {dormant_array}");
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().expect("mixed array and two scalar proofs");
                    assert_eq!(report.directives.len(), 1);
                    assert_eq!(report.diagnostics.len(), if dormant_local { 1 } else { 2 });
                    for (actual, expected) in report.diagnostics.iter().zip([("src/shared.w", "lib.echo(7)"), ("src/main.w", "reflect(5)")]) {
                        assert_eq!(actual.proof.source.path, expected.0);
                        assert_eq!(actual.invocation.call.source.path, "src/main.w");
                        let span = actual.invocation.call.range;
                        assert_eq!(&app[span.start as usize..span.end as usize], expected.1);
                    }
                    assert_eq!(report.symbolic_diagnostic.is_some(), dormant_local && scope == ScalarCheckerScope::CurrentPackage);
                    if dormant_array && scope == ScalarCheckerScope::MainFlowOnly {
                        assert!(report.aggregate_diagnostics.is_empty());
                    } else {
                        let [array] = report.aggregate_diagnostics.as_slice() else { panic!("original aggregate claim"); };
                        assert_eq!(array.proof.source.path, "src/shared.w");
                        assert_eq!(array.call.is_none(), dormant_array);
                    }
                    let clipped = terminal(&limited, scope, false);
                    assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                    assert_eq!(clipped.2.strip_prefix(b"[first]"), old.2.strip_prefix(b"[first][second]"));
                    assert_eq!(files(), first_file);
                    clean();
                    assert_eq!(terminal(&selected(8), scope, false), old);
                    assert_eq!(files(), both_files);
                    clean();
                    if !dormant_local && !dormant_array {
                        let duplicate = app.replace("second.txt", "first.txt");
                        assert_ne!(duplicate, app);
                        fs::write(&source, duplicate).unwrap();
                        let limited_duplicate = terminal(&selected(1), scope, false);
                        assert_eq!(limited_duplicate, (Ok(1), b"first".to_vec(), clipped.2));
                        assert_eq!(files(), first_file);
                        clean();
                        let refused = terminal(&selected(2), scope, false);
                        assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty());
                        assert!(!state.exists());
                    }
                }
                fs::remove_file(&shared_source).unwrap();
                // The configured app-owned scalar and a contextual debit
                // through the completed provider share one startup cursor.
                let mixed_context_provider = r#"%%start
                    unit() consume = fn { core.context.credit = core.context.credit - 1; };
                    unit() spend = fn { consume(); };
                    proof debit for consume() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 0) {}
                %%end"#;
                for scalar_first in [false, true] {
                    let score = "u8 scored = score(4);";
                    let (a, b) = if scalar_first { (score, "entry();") } else { ("entry();", score) };
                    let context_root = format!(r#"%%start
                        lib = namespace app "shared.w";
                        unit() entry = fn {{
                            u8 core.context.credit = 3;
                            lib.spend();
                        }};
                        u8(u8) score = fn(value) {{ value }};
                        proof scalar_value for score(entry) -> (out) where (entry == 4) ensures (out == 3) {{}}
                        core.require(false, {first});
                        {a} {b}
                        core.require(scored == 3, {second});
                    %%end"#);
                    fs::write(&source, &context_root).unwrap();
                    fs::write(&shared_source, mixed_context_provider).unwrap();
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old.0, Ok(19));
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 1);
                    assert_eq!(report.mixed_context_after_scalar.as_deref(), Some(&[usize::from(scalar_first)][..]));
                    let [ordinary] = report.diagnostics.as_slice() else { panic!("independent actual scalar"); };
                    assert_eq!(ordinary.invocation.returned, 4.into());
                    let context = report.contextual_diagnostic.as_ref().unwrap();
                    assert_eq!(context.proof.source.path, "src/shared.w");
                    assert_eq!(context.call.source.path, "src/shared.w");
                    assert_eq!(context.supplying_call.source.path, "src/main.w");
                    assert_eq!((&context.entry, &context.exit), (&3.into(), &2.into()));
                    let clipped = terminal(&limited, scope, false);
                    assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                    assert_eq!(clipped.2.strip_prefix(b"[first]"), old.2.strip_prefix(b"[first][second]"));
                    assert_eq!(files(), first_file);
                    clean();
                    assert_eq!(terminal(&selected(8), scope, false), old);
                    assert_eq!(files(), both_files);
                    clean();
                    fs::write(&source, context_root.replace("lib.spend();", &format!(
                        "lib.spend(); core.require(core.context.credit == 1, {unused_output});"))).unwrap();
                    let refused = selected(1);
                    assert!(matches!(refused[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                    assert!(!state.exists());

                    // The same real debit can support an ordinary Require
                    // on an authenticated immutable post-effect read.
                    let snapshot_root = context_root.replace(&format!("core.require(false, {first});"), "")
                        .replace("lib.spend();", &format!(
                            "lib.spend(); u8 observed_credit = core.context.credit; core.require(observed_credit == 1, {first});"))
                        .replace("ensures (out == 3)", "ensures (out == 4)");
                    assert_ne!(snapshot_root, context_root);
                    fs::write(&source, &snapshot_root).unwrap();
                    fs::write(&shared_source, mixed_context_provider.replace("credit) - 0", "credit) - 1")).unwrap();
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old.0, Ok(19));
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert!(report.contextual_diagnostic.is_none() && report.diagnostics.is_empty());
                    let [failure] = report.directives.as_slice() else { panic!("confirmed dependent Require consumes first slot"); };
                    let range = failure.condition;
                    assert_eq!(&snapshot_root[range.start as usize..range.end as usize], "observed_credit == 1");
                    let (axis, _) = failure.witness.axes().enumerate().find(|(_, (binding, _))|
                        &snapshot_root[binding.declaration_span.start as usize..binding.declaration_span.end as usize] == "observed_credit").unwrap();
                    assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(&2.into(), &2.into())]));
                    assert_eq!(terminal(&limited, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                    assert_eq!(files(), first_file);
                    clean();
                    assert_eq!(terminal(&selected(8), scope, false), old);
                    assert_eq!(files(), both_files);
                    clean();

                    // A refuted authority cannot spend the cap on that
                    // snapshot; the independent original failure is next.
                    fs::write(&shared_source, mixed_context_provider).unwrap();
                    let retained = selected(1);
                    let checked = retained[0].outcome.as_ref().unwrap();
                    assert!(checked.contextual_diagnostic.is_some());
                    let [failure] = checked.directives.as_slice() else { panic!("the independent Require still counts"); };
                    let range = failure.condition;
                    assert_eq!(&snapshot_root[range.start as usize..range.end as usize], "scored == 3");
                    let retained = terminal(&retained, scope, false);
                    assert_eq!((retained.0, retained.1), (Ok(19), b"second".to_vec()));
                    assert!(retained.2.starts_with(b"[second]"));
                    assert_eq!(files(), BTreeMap::from([("second.txt".to_owned(), b"file:second".to_vec())]));
                    clean();
                }
                fs::remove_file(&shared_source).unwrap();
                // A capped ordinary search cannot suppress an independent
                // authenticated authored citation graph.
                let cited = format!(r#"%%start
                    i8(i8) execute = fn(input) {{ if (input < 0) {{ -3 }} else {{ 7 }} }};
                    i8(i8) producer = fn(input) {{ i8 result = execute(input); result }};
                    proof top for producer(arg) -> (out) where (arg >= 0) ensures (out <= 8) {{ core.use(middle(arg)); }}
                    proof middle for producer(arg) -> (out) where (arg >= 0) ensures (out <= 7) {{ core.use(issued(arg)); }}
                    proof issued for producer(arg) -> (out) where (arg >= 0) ensures (out == 7) {{}}
                    core.require(false, {first});
                    core.require(false, {second});
                %%end"#);
                let show = cited.replace("core.use(middle(arg));", "core.use(issued(arg)); core.show(out > 7);");
                assert_ne!(show, cited);
                for (program, bad) in [(&cited, false), (&show, true)] {
                    fs::write(&source, program).unwrap();
                    let historical = legacy();
                    let historical_position = historical.outcome.as_ref().unwrap().symbolic_diagnostic.as_ref().map(|proof| proof.directive_position);
                    let old = terminal(&[historical], scope, false);
                    assert_eq!(old.0, Ok(19));
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 1);
                    assert!(report.diagnostics.is_empty());
                    if bad && scope == ScalarCheckerScope::CurrentPackage {
                        let proof = report.symbolic_diagnostic.as_ref().expect("required original cited show");
                        assert_eq!(proof.directive_position, 1);
                        assert_eq!(proof.returned, 7.into());
                        let step = proof.failed_step.as_ref().unwrap();
                        assert_eq!(step.source.path, "src/main.w");
                        let range = step.range;
                        assert!(program[range.start as usize..range.end as usize].contains("out > 7"));
                        assert!(proof.witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &127.into())]));
                    } else {
                        assert!(report.symbolic_diagnostic.is_none());
                    }
                    let clipped = terminal(&limited, scope, false);
                    assert_eq!((clipped.0, clipped.1), (Ok(if bad && scope == ScalarCheckerScope::CurrentPackage { 1 } else { 0 }), b"first".to_vec()));
                    assert_eq!(clipped.2.strip_prefix(b"[first]"), old.2.strip_prefix(b"[first][second]"));
                    assert_eq!(files(), first_file);
                    clean();
                    let exhausted = selected(8);
                    let position = exhausted[0].outcome.as_ref().unwrap().symbolic_diagnostic.as_ref().map(|proof| proof.directive_position);
                    let expected = (bad && scope == ScalarCheckerScope::CurrentPackage).then_some(2);
                    assert_eq!((position, historical_position), (expected, expected));
                    assert_eq!(terminal(&exhausted, scope, false), old);
                    assert_eq!(files(), both_files);
                    clean();
                }
                let reached_citation = show.replace(&format!("core.require(false, {second});"), &format!(r#"
                    i8 called = producer(3);
                    i8 repeated = producer(4);
                    core.require(called != 7, {second});
                    core.require(repeated == 7, {unused_output});
                "#));
                assert_ne!(reached_citation, show);
                fs::write(&source, &reached_citation).unwrap();
                let old = terminal(&[legacy()], scope, false);
                assert_eq!(old.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                for cap in [1, 8] {
                    let current = selected(cap);
                    let report = current[0].outcome.as_ref().unwrap();
                    let [first_call, second_call] = report.diagnostics.as_slice() else { panic!("two actual cited returns"); };
                    assert!(report.symbolic_diagnostic.is_none());
                    for (proof, snippet) in [(first_call, "producer(3)"), (second_call, "producer(4)")] {
                        assert_eq!(proof.invocation.returned, 7.into());
                        assert_eq!(proof.invocation.directive_position, 1);
                        let span = proof.invocation.call.range;
                        assert_eq!(proof.invocation.call.source.path, "src/main.w");
                        assert_eq!(&reached_citation[span.start as usize..span.end as usize], snippet);
                        let at = proof.failed_step.as_ref().unwrap().range;
                        assert!(reached_citation[at.start as usize..at.end as usize].contains("out > 7"));
                    }
                    let live = terminal(&current, scope, false);
                    if cap == 1 {
                        assert_eq!((live.0, live.1), (Ok(1), b"first".to_vec()));
                        assert_eq!(live.2.strip_prefix(b"[first]"), old.2.strip_prefix(b"[first][second]"));
                        assert_eq!(files(), first_file);
                    } else {
                        assert_eq!(live, old);
                        assert_eq!(files(), both_files);
                    }
                    clean();
                }
                let opaque = reached_citation.replace("i8 result = execute(input); result",
                    "i8 result = execute(input); i8 twice = execute(result); twice");
                assert_ne!(opaque, reached_citation);
                fs::write(&source, opaque).unwrap();
                assert!(matches!(selected(1)[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());
                if scope == ScalarCheckerScope::CurrentPackage {
                    for denied in [
                        cited.replace("core.use(issued(arg));", "core.use(top(arg));"),
                        cited.replace("ensures (out == 7)", "ensures (out == 6)"),
                        cited.replacen("arg >= 0", "arg >= -1", 1),
                    ] {
                        assert_ne!(denied, cited);
                        fs::write(&source, &denied).unwrap();
                        assert!(matches!(selected(1)[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                        assert!(!state.exists());
                    }
                }

                let projected = format!(r#"%%start
                    u8(u8) choose = fn(input) {{ if (input < 128) {{ 7 }} else {{ 7 }} }};
                    unit(u8) open = fn(value) {{
                        u8 result = choose(value);
                        core.require(result == 8, {first});
                    }};
                %%end"#);
                fs::write(&source, &projected).unwrap();
                if scope == ScalarCheckerScope::CurrentPackage {
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    let [row] = report.directives.as_slice() else { panic!("first structural child branch"); };
                    assert!(row.witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &127.into())]));
                    assert_eq!(terminal(&limited, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                    assert_eq!(files(), first_file);
                    clean();
                    let two = selected(2);
                    let report = two[0].outcome.as_ref().unwrap();
                    let [early, late] = report.directives.as_slice() else { panic!("distinct returned branches with equal value"); };
                    assert_eq!(early.directive, late.directive);
                    assert!(late.witness.intervals_at(0, 0).unwrap().eq([(&128.into(), &255.into())]));
                    let refused = terminal(&two, scope, false);
                    assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty());
                    assert!(!state.exists());
                } else {
                    assert!(selected(1)[0].outcome.as_ref().unwrap().directives.is_empty());
                    assert!(!state.exists());
                }

                let nested_projected = projected.replace("choose(value);", "forward(value);").replace(
                    "unit(u8) open", "u8(u8) forward = fn(input) { u8 original = choose(input); original }; unit(u8) open");
                assert!(nested_projected.contains("forward(value)") && nested_projected.contains("choose(input)"));
                fs::write(&source, &nested_projected).unwrap();
                if scope == ScalarCheckerScope::CurrentPackage {
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert!(report.directives[0].witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &127.into())]));
                    assert_eq!(terminal(&limited, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                    assert_eq!(files(), first_file);
                    clean();
                    let full = selected(2);
                    let report = full[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 2);
                    assert!(report.directives[1].witness.intervals_at(0, 0).unwrap().eq([(&128.into(), &255.into())]));
                    let refused = terminal(&full, scope, false);
                    assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty());
                    assert!(!state.exists());
                } else {
                    assert!(selected(1)[0].outcome.as_ref().unwrap().directives.is_empty());
                    assert!(!state.exists());
                }

                let qualified_projected = projected
                    .replace("u8(u8) choose = fn(input) { if (input < 128) { 7 } else { 7 } };",
                        r#"lib = namespace app "shared.w";"#)
                    .replace("choose(value)", "lib.choose(value)");
                assert!(!qualified_projected.contains("fn(input)") && qualified_projected.contains("lib.choose(value)"));
                fs::write(&source, &qualified_projected).unwrap();
                fs::write(&shared_source, "%%start u8(u8) choose = fn(input) { if (input < 128) { 7 } else { 7 } }; %%end").unwrap();
                if scope == ScalarCheckerScope::CurrentPackage {
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 1);
                    let witness = &report.directives[0].witness;
                    assert!(witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &127.into())]));
                    assert_eq!(terminal(&limited, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                    assert_eq!(files(), first_file);
                    clean();
                    let full = selected(2);
                    let report = full[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 2);
                    assert!(report.directives[1].witness.intervals_at(0, 0).unwrap().eq([(&128.into(), &255.into())]));
                    let refused = terminal(&full, scope, false);
                    assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty());
                    assert!(!state.exists());
                } else {
                    assert!(selected(1)[0].outcome.as_ref().unwrap().directives.is_empty());
                    assert!(!state.exists());
                }
                fs::remove_file(&shared_source).unwrap();

                let qualified_nested = qualified_projected.replace("lib.choose(value)", "lib.forward(value)");
                assert_ne!(qualified_nested, qualified_projected);
                fs::write(&source, &qualified_nested).unwrap();
                fs::write(&shared_source, r#"%%start
                    relay = namespace app "worker.w";
                    u8(u8) forward = fn(input) { u8 returned = relay.choose(input); returned };
                %%end"#).unwrap();
                let worker_source = root.join("src/worker.w");
                fs::write(&worker_source, r#"%%start
                    u8(u8) choose = fn(input) { if (input < 128) { 7 } else { 7 } };
                %%end"#).unwrap();
                if scope == ScalarCheckerScope::CurrentPackage {
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 1);
                    assert!(report.directives[0].witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &127.into())]));
                    assert_eq!(terminal(&limited, scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                    assert_eq!(files(), first_file);
                    clean();
                    let full = selected(2);
                    let report = full[0].outcome.as_ref().unwrap();
                    assert_eq!(report.directives.len(), 2);
                    assert!(report.directives[1].witness.intervals_at(0, 0).unwrap().eq([(&128.into(), &255.into())]));
                    let refused = terminal(&full, scope, false);
                    assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty());
                    assert!(!state.exists());
                } else {
                    assert!(selected(1)[0].outcome.as_ref().unwrap().directives.is_empty());
                    assert!(!state.exists());
                }
                fs::remove_file(&shared_source).unwrap();
                fs::remove_file(&worker_source).unwrap();

                let external_scalar = r#"
                    u8(u8) score = fn(value) { value };
                    proof scoped for score(entry) -> (out) where (entry == 4) ensures (out == 8) {}
                    u8(u8) other_score = fn(value) { value };
                    proof other for other_score(entry) -> (out) where (entry == 5) ensures (out == 8) {}
                "#;
                for separate in [false, true] {
                    let facts_source = root.join("src/facts.w");
                    let (namespace, qualifier) = if separate {
                        fs::write(&facts_source, format!("%%start {external_scalar} %%end")).unwrap();
                        fs::write(&shared_source, mixed_context_provider).unwrap();
                        (r#"facts = namespace app "facts.w";"#, "facts")
                    } else {
                        fs::write(&shared_source, mixed_context_provider.replace("%%end", &format!("{external_scalar} %%end"))).unwrap();
                        ("", "lib")
                    };
                    for (early, dormant) in [(4, false), (3, false), (4, true)] {
                        let before_context = if dormant { String::new() } else {
                            format!("u8 pre = {qualifier}.score({early}); core.require(pre == {early}, {unused_output});")
                        };
                        let after_context = if dormant { format!("core.require(false, {second});") } else {
                            format!("u8 post = {qualifier}.other_score(5); core.require(post == 8, {second});")
                        };
                        let program = format!(r#"%%start
                            lib = namespace app "shared.w";
                            {namespace}
                            unit() entry = fn {{
                                u8 core.context.credit = 3;
                                lib.spend();
                            }};
                            core.require(false, {first});
                            {before_context}
                            entry();
                            {after_context}
                        %%end"#);
                        fs::write(&source, &program).unwrap();
                        let old = terminal(&[legacy()], scope, false);
                        assert_eq!(old.0, Ok(19));
                        assert_eq!(files(), both_files);
                        clean();
                        let limited = selected(1);
                        let report = limited[0].outcome.as_ref().unwrap();
                        let preceding = usize::from(!dormant && early == 4);
                        assert_eq!(report.mixed_context_after_scalar.as_deref(), Some(&[preceding][..]));
                        assert_eq!(report.diagnostics.len(), if dormant { 0 } else { 1 + preceding });
                        for diagnostic in &report.diagnostics {
                            assert_eq!(diagnostic.invocation.call.source.path, "src/main.w");
                            assert_eq!(diagnostic.proof.source.path, if separate { "src/facts.w" } else { "src/shared.w" });
                        }
                        assert_eq!(report.contextual_diagnostic.as_ref().unwrap().proof.source.path, "src/shared.w");
                        assert_eq!(report.symbolic_diagnostic.is_some(), dormant && scope == ScalarCheckerScope::CurrentPackage);
                        let clipped = terminal(&limited, scope, false);
                        assert_eq!((clipped.0, clipped.1), (Ok(1), b"first".to_vec()));
                        assert_eq!(clipped.2.strip_prefix(b"[first]"), old.2.strip_prefix(b"[first][second]"));
                        assert_eq!(files(), first_file);
                        clean();
                        assert_eq!(terminal(&selected(8), scope, false), old);
                        assert_eq!(files(), both_files);
                        clean();
                    }
                    if separate { fs::remove_file(&facts_source).unwrap(); }
                }
                fs::remove_file(&shared_source).unwrap();

                let rank_definition = r#"
                    u8(u8) descend = fn(value) { if (value < 3) { 4 } else { descend(value - 3) } };
                    proof descent for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
                        if (entry < 3) { core.show(out == 4); } else { core.use(descent(entry - 3)); }
                    }
                "#;
                let ranked_mixed = mixed_catalog.replace("u8[2] input = make();",
                    &format!("{rank_definition} u8[2] input = make();"))
                    .replace("u8 returned = echo(7);", &format!(
                        "u8 recursed = descend(8); core.require(recursed == 4, {unused_output}); u8 returned = echo(7);"));
                let dormant_rank = ranked_mixed.replace(&format!("u8 recursed = descend(8); core.require(recursed == 4, {unused_output});"), "")
                    .replace("ensures (out == 4)", "ensures (out == 5)").replace("core.show(out == 4)", "core.show(out == 5)");
                for (catalog, deferred) in [(&ranked_mixed, false), (&dormant_rank, true)] {
                    fs::write(&source, catalog).unwrap();
                    let old_mixed = terminal(&[legacy()], scope, false);
                    assert_eq!(old_mixed.0, Ok(19));
                    assert_eq!(files(), both_files);
                    clean();
                    let limited = selected(1);
                    let report = limited[0].outcome.as_ref().unwrap();
                    assert_eq!(report.diagnostics.len(), 1);
                    assert_eq!(report.aggregate_diagnostics.len(), 1);
                    assert_eq!(report.symbolic_diagnostic.is_some(), deferred && scope == ScalarCheckerScope::CurrentPackage);
                    let output = terminal(&limited, scope, false);
                    assert_eq!((output.0, output.1), (Ok(1), b"first".to_vec()));
                    assert_eq!(output.2.strip_prefix(b"[first]"), old_mixed.2.strip_prefix(b"[first][second]"));
                    assert_eq!(files(), first_file);
                    clean();
                    assert_eq!(terminal(&selected(8), scope, false), old_mixed);
                    assert_eq!(files(), both_files);
                    clean();
                }
                fs::write(&source, &dormant_rank).unwrap();
                assert!(selected(1)[0].outcome.is_ok());
                fs::write(&source, dormant_rank.replace("ensures (out == 5)", "ensures (out <= 3)")).unwrap();
                assert!(matches!(selected(1)[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());

                fs::write(&source, ranked_mixed.replace("descend(8); core.require", "descend(9); core.require")).unwrap();
                let no_rank_receipt = selected(1);
                assert!(matches!(no_rank_receipt[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());

                let pruned = mixed_catalog.replace("u8 returned = echo(7);", &format!(
                    "core.narrow(false, {second}); u8 returned = echo(7);"));
                fs::write(&source, pruned).unwrap();
                let no_replay = selected(1);
                assert!(matches!(no_replay[0].outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(!state.exists());

                fs::write(&source, &ranked).unwrap();
                let legacy_render = terminal(&[legacy()], scope, false);
                assert_eq!(legacy_render.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                let cap_one = terminal(&selected(1), scope, false);
                assert_eq!(cap_one.0, Ok(1)); // code zero selected; independent formal error remains
                assert_eq!(cap_one.1, b"first");
                assert_eq!(files(), first_file);
                let suffix = legacy_render.2.strip_prefix(b"[first][second]").unwrap();
                assert!(String::from_utf8_lossy(suffix).contains("call returned 7"));
                assert_eq!(cap_one.2.strip_prefix(b"[first]"), Some(suffix));
                clean();
                assert_eq!(terminal(&selected(3), scope, false), legacy_render);
                assert_eq!(files(), both_files);
                clean();

                let repeated = format!(r#"%%start
                    u8(u8) echo = fn(value) {{ value }};
                    proof live for echo(entry) -> (out) where (entry == 7) ensures (out == 8) {{}}
                    u8 one = echo(7);
                    core.require(one == 8, {first});
                    u8 two = echo(7);
                    core.require(two == 8, {second});
                %%end"#);
                fs::write(&source, &repeated).unwrap();
                let both = terminal(&[legacy()], scope, false);
                assert_eq!(both.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                let selected_one = selected(1);
                let report = selected_one[0].outcome.as_ref().unwrap();
                let [first_call, second_call] = report.diagnostics.as_slice() else { panic!("two actual formal receipts"); };
                assert!(first_call.invocation.call.range.start < second_call.invocation.call.range.start);
                assert_eq!(first_call.invocation.returned, second_call.invocation.returned);
                let one = terminal(&selected_one, scope, false);
                assert_eq!((one.0, one.1), (Ok(1), b"first".to_vec()));
                assert_eq!(one.2.strip_prefix(b"[first]"), both.2.strip_prefix(b"[first][second]"));
                assert_eq!(files(), first_file);
                clean();
                assert_eq!(terminal(&selected(3), scope, false), both);
                assert_eq!(files(), both_files);
                clean();

                let mixed_roster = ranked.replace("core.require(actual == 8,",
                    "u8 rerank = descend(5); u8 more = echo(7); u8 last = echo(7); core.require(actual == 8,");
                fs::write(&source, &mixed_roster).unwrap();
                let mixed = selected(1);
                assert_eq!(mixed[0].outcome.as_ref().unwrap().diagnostics.len(), 3);
                let emitted = terminal(&mixed, scope, false);
                assert_eq!((emitted.0, emitted.1), (Ok(1), b"first".to_vec()));
                assert_eq!(files(), first_file);
                clean();

                // A real later rank outside the verified guard vetoes
                // the whole private outcome despite the earlier ordinary cap.
                fs::write(&source, mixed_roster.replace("rerank = descend(5)", "rerank = descend(9)")).unwrap();
                let refused = selected(1);
                let [refused] = refused.as_slice() else { panic!("one unsupported owner"); };
                assert!(matches!(refused.outcome, Err(wosy_compiler::ScalarOrderedCheckerOutcome::Unsupported)));
                assert!(files().is_empty() && !state.exists());

                fs::write(&source, context.replace(" where (cost <= 1)", "")).unwrap();
                let legacy_context = terminal(&[legacy()], scope, false);
                assert_eq!(legacy_context.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                fs::write(&source, &context).unwrap();
                assert_eq!(terminal(&selected(3), scope, false), legacy_context);
                assert_eq!(files(), both_files);
                clean();
                let wrong = context.replace("credit = core.context.credit - 1", "credit = core.context.credit - 0");
                fs::write(&source, &wrong).unwrap();
                let refuted = terminal(&selected(1), scope, false);
                assert_eq!(refuted.0, Ok(1));
                assert!(refuted.1.is_empty() && files().is_empty() && !state.exists());
                let diagnostic = String::from_utf8_lossy(&refuted.2);
                assert!(diagnostic.contains("proof obligation failed") && diagnostic.contains("explicit call entry: cost = 1")
                    && diagnostic.contains("contextual value at entry: 3; at exit: 3") && diagnostic.contains("consume(1)")
                    && !diagnostic.contains("[first]"));
                assert_eq!(terminal(&selected(3), scope, false), refuted);
                fs::write(&source, wrong.replace("consume(1)", "consume(2)")).unwrap();
                let excluded = terminal(&selected(1), scope, false);
                assert_eq!(excluded, (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();

                let roster = format!(r#"%%start
                    unit() consume = fn {{ core.context.credit = core.context.credit - 1; }};
                    unit() relay = fn {{ consume(); }};
                    unit() finish = fn {{ core.context.credit = core.context.credit - 1; }};
                    unit() entry = fn {{
                        u8 core.context.credit = 5;
                        core.require(false, {first});
                        relay();
                        core.require(false, {second});
                        relay();
                        finish();
                    }};
                    proof debit for consume() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 1) {{}}
                    proof final_debit for finish() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 1) {{}}
                    entry();
                %%end"#);
                fs::write(&source, &roster).unwrap();
                let old = terminal(&[legacy()], scope, false);
                assert_eq!(old.0, Ok(19));
                assert_eq!(files(), both_files);
                clean();
                assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"first".to_vec(), b"[first]".to_vec()));
                assert_eq!(files(), first_file);
                clean();
                for cap in [2, 8] {
                    assert_eq!(terminal(&selected(cap), scope, false), old);
                    assert_eq!(files(), both_files);
                    clean();
                }

                // A late theorem sees genuine state after two traversals
                // of the same original nested site, even after cap1.
                let wrong = roster.replace("proof final_debit for finish() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 1)",
                    "proof final_debit for finish() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 0)");
                assert_ne!(wrong, roster);
                fs::write(&source, &wrong).unwrap();
                let old = legacy();
                let prior = ordered_check_diagnostics(old.outcome.as_ref().unwrap(), &old.texts).unwrap();
                for cap in [1, 2, 8] {
                    let receipts = selected(cap);
                    let receipt = &receipts[0];
                    let report = receipt.outcome.as_ref().unwrap();
                    assert!(report.directives.is_empty() && report.additional_contextual_diagnostics.is_empty());
                    let failure = report.contextual_diagnostic.as_ref().expect("late contextual proof");
                    assert_eq!((&failure.entry, &failure.exit), (&3.into(), &2.into()));
                    let range = failure.call.range;
                    assert_eq!(&wrong[range.start as usize..range.end as usize], "finish()");
                    assert_eq!(ordered_check_diagnostics(report, &receipt.texts).unwrap(), prior);
                    let refused = terminal(&receipts, scope, false);
                    assert_eq!((refused.0, refused.1), (Ok(1), Vec::new()));
                    assert!(!refused.2.is_empty() && files().is_empty() && !state.exists());
                }

                let witness_output = r#"%%start
                    unit(u8) report = fn(value) {
                        core.require(value == 7, { .stdout = "witness"; .stderr = ""; .code = 0;
                            .files = [{ .path = "witness.json"; .state = value; }]; });
                        core.require(value != 7, { .stdout = "selected"; .stderr = ""; .code = value;
                            .files = [{ .path = "selected.json"; .state = value; }]; });
                    };
                %%end"#;
                fs::write(&source, witness_output).unwrap();
                if scope == ScalarCheckerScope::CurrentPackage {
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old, (Ok(7), b"witnessselected".to_vec(), Vec::new()));
                    let old_files = files();
                    clean();
                    let one = selected(1);
                    let [first] = one[0].outcome.as_ref().unwrap().directives.as_slice()
                        else { panic!("two disjoint alternatives count as one path"); };
                    assert!(first.witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &6.into()), (&8.into(), &255.into())]));
                    assert_eq!(terminal(&one, scope, false), (Ok(0), b"witness".to_vec(), Vec::new()));
                    assert_eq!(files().len(), 1);
                    assert_eq!(files()["witness.json"], old_files["witness.json"]);
                    clean();
                    for cap in [2, 8] {
                        assert_eq!(terminal(&selected(cap), scope, false), old);
                        assert_eq!(files(), old_files);
                        clean();
                    }

                    fs::write(&source, witness_output.replace("value != 7,", "value == 7,")).unwrap();
                    assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"witness".to_vec(), Vec::new()));
                    assert_eq!(files().len(), 1);
                    clean();
                    let refused = terminal(&selected(2), scope, false);
                    assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty() && !state.exists());
                } else {
                    assert_eq!(terminal(&selected(2), scope, false), terminal(&[legacy()], scope, false));
                    assert!(!state.exists());
                }

                if scope == ScalarCheckerScope::CurrentPackage {
                    // The same authored Require site is reached via two
                    // genuinely distinct short-circuit decisions.
                    let compound = r#"%%start
                        unit(u8, u8) report = fn(left, right) {
                            if (left == 0 || right == 0) {
                                core.require(false, { .stdout = "true"; .stderr = ""; .code = 0; .files = []; });
                            } else {
                                core.require(false, { .stdout = "false"; .stderr = ""; .code = 9; .files = []; });
                            };
                        };
                    %%end"#;
                    fs::write(&source, compound).unwrap();
                    assert_eq!(terminal(&[legacy()], scope, false), (Ok(9), b"truefalse".to_vec(), Vec::new()));
                    for (cap, expected, status) in [(1, "true", 0), (2, "truetrue", 0), (3, "truetruefalse", 9), (8, "truetruefalse", 9)] {
                        let selected = selected(cap);
                        let report = selected[0].outcome.as_ref().unwrap();
                        assert_eq!(report.directives.len(), cap.min(3));
                        if cap > 1 { assert_eq!(report.directives[0].directive, report.directives[1].directive); }
                        assert_eq!(terminal(&selected, scope, false), (Ok(status), expected.as_bytes().to_vec(), Vec::new()));
                    }
                    let files_case = compound.replace(".code = 0; .files = [];", r#".code = left;
                        .files = [{ .path = "short.json"; .state = left; }];"#);
                    assert_ne!(files_case, compound);
                    fs::write(&source, &files_case).unwrap();
                    assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"true".to_vec(), Vec::new()));
                    assert_eq!(files()["short.json"], b"{\"type\":{\"kind\":\"u8\"},\"value\":0}\n");
                    clean();
                    let refused = terminal(&selected(2), scope, false);
                    assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty() && !state.exists());

                    fs::write(&source, compound.replace(".code = 0; .files = [];", r#".code = 0;
                        .files = [{ .path = "short.json"; .state = left; }];"#)).unwrap();
                    assert_eq!(terminal(&selected(1), scope, false).0, Ok(0));
                    assert_eq!(files().len(), 1);
                    clean();
                    let duplicate = terminal(&selected(2), scope, false);
                    assert!(duplicate.0.is_err() && duplicate.1.is_empty() && duplicate.2.is_empty() && !state.exists());
                }

                if scope == ScalarCheckerScope::CurrentPackage {
                    let compared = r#"%%start
                        unit(u8, u8) report = fn(left, right) {
                            core.require(left != 0 && right != 0,
                                { .stdout = "require"; .stderr = ""; .code = 0; .files = []; });
                            core.require(false, { .stdout = "after"; .stderr = ""; .code = 9; .files = []; });
                        };
                    %%end"#;
                    fs::write(&source, compared).unwrap();
                    assert_eq!(terminal(&[legacy()], scope, false), (Ok(9), b"requireafter".to_vec(), Vec::new()));
                    let observed = selected(3);
                    let report = observed[0].outcome.as_ref().unwrap();
                    let [left, right, after] = report.directives.as_slice() else { panic!("two false operand paths and one original continuation"); };
                    assert_eq!(left.directive, right.directive);
                    assert_ne!(right.directive, after.directive);
                    let bounds = |failure: &wosy_compiler::ScalarOrderedCheckerFailure, axis| {
                        failure.witness.intervals_at(0, axis).unwrap()
                            .map(|(lo, hi)| (lo.to_string(), hi.to_string())).collect::<Vec<_>>()
                    };
                    assert_eq!(bounds(left, 0), [("0".to_owned(), "0".to_owned())]);
                    assert_eq!(bounds(right, 0), [("1".to_owned(), "255".to_owned())]);
                    assert_eq!(bounds(right, 1), [("0".to_owned(), "0".to_owned())]);
                    assert_eq!(bounds(after, 0), [("0".to_owned(), "255".to_owned())]);
                    assert_eq!(bounds(after, 1), [("0".to_owned(), "255".to_owned())]);
                    for (cap, expected, status) in [(1, "require", 0), (2, "requirerequire", 0), (3, "requirerequireafter", 9)] {
                        assert_eq!(terminal(&selected(cap), scope, false), (Ok(status), expected.as_bytes().to_vec(), Vec::new()));
                    }

                    let narrowed = compared.replace("core.require(left != 0 && right != 0,", "core.narrow(left == 0 || right == 0,");
                    fs::write(&source, narrowed).unwrap();
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old, (Ok(9), b"after".to_vec(), Vec::new()));
                    let survived = selected(3);
                    let report = survived[0].outcome.as_ref().unwrap();
                    let [left, right] = report.directives.as_slice() else { panic!("two genuine passing narrowing decisions"); };
                    assert_eq!(left.directive, right.directive);
                    assert_eq!(bounds(left, 0), [("0".to_owned(), "0".to_owned())]);
                    assert_eq!(bounds(left, 1), [("0".to_owned(), "255".to_owned())]);
                    assert_eq!(bounds(right, 0), [("1".to_owned(), "255".to_owned())]);
                    assert_eq!(bounds(right, 1), [("0".to_owned(), "0".to_owned())]);
                    assert_eq!(terminal(&survived, scope, false), (Ok(9), b"afterafter".to_vec(), Vec::new()));

                    let empty = r#"%%start
                        unit(u8, u8) report = fn(left, right) {
                            core.narrow(right != 0, { .stdout = "unexpected"; .stderr = ""; .code = 1; .files = []; });
                            core.narrow(left == 0 && right == 0, {
                                .stdout = "empty"; .stderr = ""; .code = 7;
                                .files = [{ .path = "before.json"; .state = left; }];
                            });
                            core.require(false, { .stdout = "unreachable"; .stderr = ""; .code = 9; .files = []; });
                        };
                    %%end"#;
                    fs::write(&source, empty).unwrap();
                    let old = terminal(&[legacy()], scope, false);
                    assert_eq!(old, (Ok(7), b"empty".to_vec(), Vec::new()));
                    let old_files = files();
                    clean();
                    for cap in [1, 3] {
                        let refused = selected(cap);
                        let report = refused[0].outcome.as_ref().unwrap();
                        let [failure] = report.directives.as_slice() else { panic!("one failed incoming structural history"); };
                        assert_eq!(bounds(failure, 0), [("0".to_owned(), "255".to_owned())]);
                        assert_eq!(bounds(failure, 1), [("1".to_owned(), "255".to_owned())]);
                        assert_eq!(terminal(&refused, scope, false), old);
                        assert_eq!(files(), old_files);
                        clean();
                    }
                }

                // A Unit return may carry two real surviving branches.
                // The parent must not flatten them into one scalar domain.
                let branch_leaf = r#"
                    unit(u8) leaf = fn(value) {
                        if (value < 128) {
                            core.require(true, { .stdout = "unexpected"; .stderr = ""; .code = 7; .files = []; });
                        } else {
                            core.narrow(value > 128, { .stdout = "unexpected"; .stderr = ""; .code = 8; .files = []; });
                        };
                    };
                "#;
                for qualified in [false, true] {
                    let (declarations, leaf) = if qualified {
                        fs::write(&shared_source, format!("%%start {branch_leaf} %%end")).unwrap();
                        (r#"lib = namespace app "shared.w";"#, "lib.leaf")
                    } else { (branch_leaf, "leaf") };
                    let main = format!(r#"%%start
                        {declarations}
                        unit(u8) second = fn(value) {{ {leaf}(value); }};
                        unit(u8) first = fn(value) {{ second(value); }};
                        unit(u8) report = fn(value) {{
                            first(value);
                            core.require(false, {{ .stdout = "after"; .stderr = ""; .code = 9; .files = []; }});
                        }};
                    %%end"#);
                    fs::write(&source, &main).unwrap();
                    if scope == ScalarCheckerScope::CurrentPackage {
                        let observed = selected(3);
                        let report = observed[0].outcome.as_ref().unwrap();
                        let [left, right] = report.directives.as_slice() else { panic!("two original branch continuations, no Narrow failure"); };
                        assert_eq!(left.directive, right.directive);
                        assert!(left.witness.intervals_at(0, 0).unwrap().eq([(&0.into(), &127.into())]));
                        assert!(right.witness.intervals_at(0, 0).unwrap().eq([(&129.into(), &255.into())]));
                        for (cap, stream) in [(1, &b"after"[..]), (2, &b"afterafter"[..]), (8, &b"afterafter"[..])] {
                            assert_eq!(terminal(&selected(cap), scope, false), (Ok(9), stream.to_vec(), Vec::new()));
                            assert!(!state.exists());
                        }
                    } else {
                        assert_eq!(terminal(&selected(8), scope, false), terminal(&[legacy()], scope, false));
                    }
                    if qualified { fs::remove_file(&shared_source).unwrap(); }
                }

                let exact_typed = r#"%%start
                    bool(bool, i8) choose = fn(flag, entry) {
                        core.require(entry != 3, { .stdout = "producer"; .stderr = ""; .code = 0; .files = []; });
                        if (flag) { true } else { false }
                    };
                    bool initialized = true;
                    bool ready = choose(initialized, 3);
                    i8 status = if (ready) {
                        core.require(false, { .stdout = "yes"; .stderr = ""; .code = 5; .files = []; });
                        7
                    } else {
                        core.require(false, { .stdout = "no"; .stderr = ""; .code = 6; .files = []; });
                        8
                    };
                    core.require(status < 7, { .stdout = "after"; .stderr = ""; .code = status;
                        .files = [{ .path = "typed.json"; .state = status; }]; });
                %%end"#;
                for (flag, stream, status, value) in [(true, "yes", 5, 7), (false, "no", 6, 8)] {
                    for direct in [false, true] {
                        let mut actual = exact_typed.replace("bool initialized = true;", &format!("bool initialized = {flag};"));
                        if direct {
                            actual = actual.replace("bool ready = choose(initialized, 3);", "")
                                .replace("if (ready)", "if (choose(initialized, 3))");
                        }
                        fs::write(&source, &actual).unwrap();
                        let old = terminal(&[legacy()], scope, false);
                        let full = format!("producer{stream}after").into_bytes();
                        assert_eq!(old, (Ok(status), full, Vec::new()));
                        let old_files = files();
                        assert_eq!(old_files["typed.json"], format!("{{\"type\":{{\"kind\":\"i8\"}},\"value\":{value}}}\n").as_bytes());
                        clean();
                        assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"producer".to_vec(), Vec::new()));
                        assert!(!state.exists());
                        assert_eq!(terminal(&selected(2), scope, false),
                            (Ok(status), format!("producer{stream}").into_bytes(), Vec::new()));
                        assert!(!state.exists());
                        assert_eq!(terminal(&selected(8), scope, false), old);
                        assert_eq!(files(), old_files);
                        clean();
                    }
                }
                let unsupported = exact_typed.replace("if (ready)", "if (ready || false)");
                fs::write(&source, unsupported).unwrap();
                let refused = terminal(&selected(8), scope, false);
                assert!(refused.0.is_err() && refused.1.is_empty() && refused.2.is_empty() && !state.exists());

                let nullary = r#"%%start
                    i8() code = fn {
                        core.require(false, { .stdout = "nullary"; .stderr = ""; .code = 0; .files = []; });
                        7
                    };
                    i8 observed = code();
                    core.require(observed != 7, { .stdout = "after"; .stderr = ""; .code = observed; .files = []; });
                %%end"#;
                fs::write(&source, nullary).unwrap();
                let old = terminal(&[legacy()], scope, false);
                assert_eq!(old, (Ok(7), b"nullaryafter".to_vec(), Vec::new()));
                assert_eq!(terminal(&selected(1), scope, false), (Ok(0), b"nullary".to_vec(), Vec::new()));
                assert_eq!(terminal(&selected(8), scope, false), old);
                assert!(!state.exists());

                let startup_witness = r#"%%start
                    i8 status = -37;
                    bool ready = true;
                    core.require(false, { .stdout = "startup"; .stderr = ""; .code = -status;
                        .files = [{ .path = "status.json"; .state = -status; },
                                  { .path = "ready.json"; .state = !ready; }]; });
                %%end"#;
                fs::write(&source, startup_witness).unwrap();
                let old = terminal(&[legacy()], scope, false);
                let old_files = files();
                assert_eq!(old.0, Ok(37));
                assert_eq!(old_files["ready.json"], b"{\"type\":{\"kind\":\"bool\"},\"value\":false}\n");
                assert_eq!(old_files["status.json"], b"{\"type\":{\"kind\":\"i8\"},\"value\":37}\n");
                clean();
                assert_eq!(terminal(&selected(1), scope, false), old);
                assert_eq!(files(), old_files);
                clean();
            }
        }

        // Real sibling dependency publications: only selected filenames
        // conflict, but later selected-input typing is always completed.
        let package = |name: &str, output: &str| {
            let child = root.join(name);
            fs::create_dir_all(child.join("src")).unwrap();
            fs::write(child.join("wosy.toml"), manifest(name, output)).unwrap();
            child.join("src/main.w")
        };
        let a = package("a", "native");
        let b = package("b", "wasm-wasip1");
        let same_file = |mark: &str, code: u8| format!(r#"%%start
core.require(false, {{ .stdout = "{mark}"; .stderr = ""; .code = {code}; .files = [{{ .path = "same.txt"; .text = "{mark}"; }}]; }});
%%end"#);
        fs::write(&a, same_file("a", 7)).unwrap();
        fs::write(&b, same_file("b", 8)).unwrap();
        fs::write(root.join("wosy.toml"), manifest("app", "native") + "\n[dependencies.a]\npath = \"a\"\n[dependencies.b]\npath = \"b\"\n").unwrap();
        fs::write(&source, "%%start core.require(true, { .stdout = \"\"; .stderr = \"\"; .code = 0; .files = []; }); %%end").unwrap();
        let scope = ScalarCheckerScope::CurrentPackage;
        let batch = |cap| configured_check::check_targets_with_private_authored_limit(
            root, None, "dev", scope, NonZeroUsize::new(cap).unwrap(),
        ).ok().unwrap();
        assert_eq!(terminal(&batch(1), scope, true), (Ok(7), b"a".to_vec(), Vec::new()));
        assert_eq!(files(), BTreeMap::from([("same.txt".to_owned(), b"a".to_vec())]));
        clean();
        let duplicate = terminal(&batch(2), scope, true);
        assert!(duplicate.0.unwrap_err().contains("no result published"));
        assert!(duplicate.1.is_empty() && duplicate.2.is_empty() && !state.exists());
        fs::write(&b, "%%start u8 bad = true; %%end").unwrap();
        let invalid = terminal(&batch(1), scope, true);
        assert_eq!(invalid.0, Ok(2));
        assert!(invalid.1.is_empty() && String::from_utf8_lossy(&invalid.2).contains("b::src/main.w") && !state.exists());
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
        let default = Arguments::try_parse_from(["wosy", "check"]).unwrap();
        assert!(matches!(default.command, Command::Check {
            target: None, main_flow_only: false, declared_package: false, ..
        }));
        assert!(Arguments::try_parse_from(["wosy", "check", "--main-flow-only", "--declared-package"]).is_err());
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
    fn internal_checker_adapter_and_opt_in_cli_preserve_main_flow_scope() {
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
        let selected = Arguments::try_parse_from(["wosy", "check", "app", "--main-flow-only"]).unwrap();
        assert!(matches!(selected.command, Command::Check {
            target: Some(target), profile, main_flow_only: true, declared_package: false, dependencies: false, ..
        } if target == "app" && profile == "dev"));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn configured_declared_package_scope_uses_exact_membership_without_dormant_initializers() {
        use configured_check::{check_declared_package_target, ConfiguredCheckLoadError};
        let root = snapshot_test_path("declared-scope").with_extension("");
        let manifest = r#"
[package]
name = "app"
default_target = "app"
source_root = "src"

[comments]
categories = []
[builders]
[runners]

[targets.app]
root = "src/main.w"
[targets.app.artifacts.main]
artifact = "RuntimeStdio"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "never"
run_runner = "never"
[targets.app.profiles.dev]
main_artifact = "main"
"#;
        let main = r#"%%start
            imported = namespace app "live.w";
            u8 actual = imported.run(7);
        %%end"#;
        let dormant = r#"%%start
            core.require(false, { .stdout = "forbidden initializer"; .stderr = ""; .code = 19; .files = []; });
            u8(u8) dormant = fn(value) { value };
            proof checks for dormant(entry) -> (out) ensures (out != 7) {}
        %%end"#;
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        for (path, content) in [
            ("wosy.toml", manifest), ("src/main.w", main),
            ("src/live.w", "%%start u8(u8) run = fn(value) { value }; %%end"),
            ("src/features/dormant.w", dormant),
            ("stray.w", "intentionally invalid outside the declared tree"),
            ("src/other_package/wosy.toml", r#"[package]
name = "other"
source_root = "src"
"#),
            ("src/other_package/src/broken.w", "intentionally invalid in the nested package"),
            ("src/.wosy/broken.w", "intentionally invalid in generated state"),
        ] {
            write(path, content);
        }
        let checked = check_declared_package_target(&root, None, "dev")
            .unwrap_or_else(|_| panic!("configured tree should close"));
        let report = checked.outcome.unwrap();
        assert!(report.active && report.directives.is_empty() && report.diagnostics.is_empty());
        let failure = report.symbolic_diagnostic.unwrap();
        assert_eq!(failure.proof.source.path, "src/features/dormant.w");
        assert_eq!(failure.returned.to_string(), "7");
        assert!(report.additional_symbolic_diagnostics.is_empty());
        assert_eq!(checked.texts.len(), 3);
        assert!(!root.join(".wosy").exists());

        write("wosy.toml", &manifest.replace("source_root = \"src\"\n", ""));
        assert!(matches!(check_declared_package_target(&root, None, "dev"),
            Err(ConfiguredCheckLoadError::PackageSourceRoot(_))));
        write("wosy.toml", manifest);
        write("src/main.w", &main.replace("live.w", "other_package/src/broken.w"));
        assert!(matches!(check_declared_package_target(&root, None, "dev"),
            Err(ConfiguredCheckLoadError::Canonical(CanonicalLoadFailure::Input(message)))
                if message.contains("outside the declared package source tree")));
        write("src/main.w", main);
        let nested = "src/other_package/wosy.toml";
        write(nested, "[package]\nname =");
        assert!(matches!(check_declared_package_target(&root, None, "dev"),
            Err(ConfiguredCheckLoadError::Dormant(CanonicalLoadFailure::Input(message)))
                if message.contains("invalid nested package identity")));
        fs::remove_file(root.join(nested)).unwrap();
        fs::create_dir(root.join(nested)).unwrap();
        assert!(matches!(check_declared_package_target(&root, None, "dev"),
            Err(ConfiguredCheckLoadError::Dormant(CanonicalLoadFailure::Input(message)))
                if message.contains("nested package manifest is not a non-aliased regular file")));
        fs::remove_dir(root.join(nested)).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("wosy.toml"), root.join(nested)).unwrap();
            assert!(matches!(check_declared_package_target(&root, None, "dev"),
                Err(ConfiguredCheckLoadError::Dormant(CanonicalLoadFailure::Input(message)))
                    if message.contains("nested package manifest is not a non-aliased regular file")));
            fs::remove_file(root.join(nested)).unwrap();
        }
        write(nested, "[package]\nname = \"other\"\nsource_root = \"src\"\n");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("stray.w"), root.join("src/alias.w")).unwrap();
            assert!(matches!(check_declared_package_target(&root, None, "dev"),
                Err(ConfiguredCheckLoadError::Dormant(CanonicalLoadFailure::Input(message)))
                    if message.contains("package source path is aliased")));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(feature = "internal-checker"))]
    #[test]
    fn build_only_cli_does_not_offer_the_checker() {
        assert!(Arguments::try_parse_from(["wosy", "check", "app", "--main-flow-only"]).is_err());
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
            git_checkout: None,
            git_pin_identity: None,
            patch_stack_id: None,
            local_root_identity: Some(dependency),
            dependencies: Vec::new(),
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
            compile_project_text(
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
            compile_project_text(
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
