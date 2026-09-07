use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic as Report, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
use std::env;
use std::fs;
use std::process::ExitCode;
use wosy_compiler::{
    derive_scalar_program_from_cst, emit_scalar_llvm_text, parse_source, publication,
    serialize_publication,
};
use wosy_project::{
    artifact_manifest_path, discover_root, load_artifact_manifest, publish_artifact,
    ArtifactIdentity, ProjectConfiguration, ProjectContext,
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
    let (target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(target_name, profile)?;
    let output = parse_project(&root, &project)?;
    let canonical_path = persist_cst(&root, &output)?;
    let canonical = load_canonical_cst(&canonical_path)?;
    render_diagnostics(
        &output.result.text,
        &output.diagnostics,
        &output.result.source,
    );
    if !output.diagnostics.is_empty() {
        return Err("build rejected by syntax diagnostics".to_owned());
    }
    let validation = derive_scalar_program_from_cst(&canonical);
    render_diagnostics(
        &output.result.text,
        &validation.diagnostics,
        &output.result.source,
    );
    if !validation.diagnostics.is_empty() {
        return Err("build rejected by semantic diagnostics".to_owned());
    }
    let llvm = emit_scalar_llvm_text(&validation)?;
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
        &project,
        &configuration,
        target_name,
        profile,
        artifact,
        artifact_profile,
        &target_config.root,
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
    let (target_config, artifact, artifact_profile, target_profile) =
        configuration.artifact_profile(target_name, profile)?;
    let manifest_path =
        artifact_manifest_path(&root, target_name, profile, &target_profile.main_artifact);
    let manifest = load_artifact_manifest(&manifest_path)?;
    let llvm = fs::read_to_string(
        manifest
            .executable
            .parent()
            .ok_or_else(|| "artifact executable has no parent".to_owned())?
            .join("partition.ll"),
    )
    .map_err(|error| error.to_string())?;
    let expected = artifact_identity(
        &root,
        &project,
        &configuration,
        target_name,
        profile,
        artifact,
        artifact_profile,
        &target_config.root,
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

fn parse_project(
    root: &std::path::Path,
    project: &ProjectContext,
) -> Result<wosy_compiler::ParseOutput, String> {
    let source_path = project.source_path();
    let text = fs::read_to_string(&source_path)
        .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
    let relative = source_path
        .strip_prefix(root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let revision = blake3::hash(text.as_bytes()).to_hex().to_string();
    let source = SourceIdentity::new(
        root.to_string_lossy().into_owned(),
        project.package.clone(),
        relative,
        revision,
    );
    Ok(parse_source(source, text, &project.comment_categories))
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

fn artifact_identity(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    target: &str,
    profile: &str,
    artifact: &wosy_project::ArtifactConfig,
    artifact_profile: &wosy_project::ArtifactProfile,
    source_root: &std::path::Path,
    llvm: &str,
) -> Result<ArtifactIdentity, String> {
    let source = fs::read_to_string(root.join(source_root)).map_err(|error| error.to_string())?;
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
        source_identity: format!(
            "{}:{}",
            project.package,
            blake3::hash(source.as_bytes()).to_hex()
        ),
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
