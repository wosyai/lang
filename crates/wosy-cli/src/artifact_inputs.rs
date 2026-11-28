//! Exactly the ordinary no-cache builder inputs. No invocation or publication.
use std::{borrow::Cow, path::{Path, PathBuf}};
use wosy_project::{ArtifactConfig, ArtifactIdentity, ArtifactManifest, ArtifactProfile, BuilderConfig,
    ProjectConfiguration, SourceObservation};

pub struct BuilderInputPaths {
    pub output: PathBuf,
    pub partition: PathBuf,
    pub runtime: PathBuf,
    pub pending_manifest: PathBuf,
}

impl BuilderInputPaths {
    pub fn new(output: PathBuf) -> Self {
        Self {
            partition: output.join("partition.ll"),
            runtime: output.join("runtime.ll"),
            pending_manifest: output.join("artifact-manifest.pending.json"),
            output,
        }
    }

    pub fn arguments(&self) -> [Cow<'_, str>; 4] {
        [&self.partition, &self.output, &self.pending_manifest, &self.runtime].map(|path| path.to_string_lossy())
    }
}

/// The ordinary Build's manifest and literal command configuration; no
/// codegen cache key, materialization, or authority to skip a builder.
pub struct PendingBuilderInputs {
    pub manifest: ArtifactManifest,
    pub builder: BuilderConfig,
}

pub fn pending_builder_inputs(
    configuration: &ProjectConfiguration, configuration_identity: String, target: &str, profile: &str,
    artifact: &ArtifactConfig, artifact_profile: &ArtifactProfile, source_observations: Vec<SourceObservation>,
    llvm_input_identity: String, output: &Path,
) -> Result<PendingBuilderInputs, String> {
    let source_identity = wosy_project::source_identity(&source_observations);
    let (identity, builder) = artifact_identity(configuration_identity, configuration, target, profile,
        artifact, artifact_profile, source_identity, llvm_input_identity)?;
    Ok(PendingBuilderInputs {
        manifest: ArtifactManifest { identity, executable: output.join("main"), source_observations, comptime: None },
        builder: builder.clone(),
    })
}

/// Ordinary configuration, literal command and canonical source evidence
/// taken from the same exact live observation, never from the remote reply.
pub(crate) struct ObservedBuilderBasis {
    pub project: PathBuf,
    pub configuration: ProjectConfiguration,
    pub configuration_identity: String,
    pub source: Vec<SourceObservation>,
}

pub fn project_configuration_identity(
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

fn artifact_identity<'a>(
    configuration_identity: String,
    configuration: &'a ProjectConfiguration,
    target: &str,
    profile: &str,
    artifact: &wosy_project::ArtifactConfig,
    artifact_profile: &wosy_project::ArtifactProfile,
    source_identity: String,
    llvm_input_identity: String,
) -> Result<(ArtifactIdentity, &'a BuilderConfig), String> {
    let builder = configuration
        .builders
        .get(&artifact_profile.builder)
        .ok_or_else(|| format!("builder {} is missing", artifact_profile.builder))?;
    let runner = configuration
        .runners
        .get(&artifact_profile.run_runner)
        .ok_or_else(|| format!("runner {} is missing", artifact_profile.run_runner))?;
    Ok((ArtifactIdentity {
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
    }, builder))
}

/// Older CLI builds ignored unknown manifest fields. Force them to reject
/// the multi-site generation instead of running after checking only site 1.
pub fn expected_artifact_compiler_identity(comptime: Option<&wosy_project::ArtifactComptimeObservation>) -> String {
    let mut identity = compiler_identity();
    if comptime.is_some_and(|observed| !observed.additional_sites.is_empty()) {
        identity.push_str(":ordered-comptime-v1");
    }
    identity
}

pub fn compiler_identity() -> String {
    wosy_compiler::COMPILER_IDENTITY.to_owned()
}

pub fn llvm_identity() -> String {
    wosy_compiler::LLVM_ADAPTER_IDENTITY.to_owned()
}
