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
        Command::Check { target, profile, main_flow_only, dependencies, .. } => {
            // Clap rejects both scope flags. An omitted scope is exactly the explicit package scope.
            let scope = if main_flow_only {
                ScalarCheckerScope::MainFlowOnly
            } else {
                ScalarCheckerScope::CurrentPackage
            };
            match check_command(target.as_deref(), &profile, scope, dependencies) {
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
        let stream = StandardStream::stderr(ColorChoice::Auto);
        let mut stream_lock = stream.lock();
        let _ = term::emit(&mut stream_lock, &config, &files, &report);
    }
}

#[cfg(feature = "internal-checker")]
fn check_command(
    target: Option<&str>, profile: &str, scope: ScalarCheckerScope, dependencies: bool,
) -> Result<u8, String> {
    use configured_check::{CheckPublicationError, CheckPublicationPreflightError, ConfiguredCheckLoadError as LoadError};
    use wosy_compiler::ScalarOrderedCheckerOutcome;

    let current = env::current_dir().map_err(|error| error.to_string())?;
    let root = discover_root(&current)?;
    let receipts = if dependencies {
        configured_check::check_target_with_dependencies(&root, target, profile, scope)
    } else {
        match scope {
            ScalarCheckerScope::MainFlowOnly => configured_check::check_target(&root, target, profile, scope, Vec::new()),
            ScalarCheckerScope::CurrentPackage => configured_check::check_declared_package_target(&root, target, profile),
        }.map(|receipt| vec![receipt])
    }.map_err(|failure| match failure {
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
    let unsupported_output = || "check: result requires unsupported output; no result published".to_owned();
    let primary = receipts.first().ok_or_else(unsupported_output)?;
    let checked = match checked_publication_reports(&receipts) {
        Ok(checked) => checked,
        Err(receipt) => match &receipt.outcome {
            Err(ScalarOrderedCheckerOutcome::Invalid(diagnostics)) => {
                render_project_diagnostics_with_packages(diagnostics, &receipt.texts, dependencies);
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
    let diagnostics = checked.iter().zip(&receipts)
        .map(|(report, receipt)| ordered_check_diagnostics(report, &receipt.texts))
        .collect::<Result<Vec<_>, _>>()?;
    let authored = publication.prepare(&primary.root).map_err(|error| match error {
        CheckPublicationPreflightError::UnsupportedFilePublication => unsupported_output(),
        CheckPublicationPreflightError::File(error) =>
            format!("check: could not prepare authored files; no result published: {error}"),
        CheckPublicationPreflightError::ExitStatusOutsideByteRange =>
            "check: selected exit code is outside 0..=255; no result published".to_owned(),
    })?;
    let authored_status = authored.publish(&mut std::io::stdout().lock(), &mut std::io::stderr().lock())
        .map_err(|error| format!("check: could not complete authored output; earlier records may already be published: {error}"))?;
    let mut has_failure = false;
    for (diagnostics, receipt) in diagnostics.iter().zip(&receipts) {
        has_failure |= !diagnostics.is_empty();
        render_project_diagnostics_with_packages(diagnostics, &receipt.texts, dependencies);
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
            target: Some(target), profile, main_flow_only: true, declared_package: false, dependencies: false,
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
