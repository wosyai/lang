//! Opt-in ordinary Build→Run through selected source and retained generations.
use super::*;
use std::{os::unix::fs::{DirBuilderExt, PermissionsExt}, path::Path, sync::Arc, time::Duration};
use wosy_cli::{
    local_build_daemon::{compare_local_build, establish_local_build_source},
    source_recovery::ClientSourceLimits,
    source_state::{AcknowledgedSourceGeneration, SqliteSourceState},
};
use wosy_daemon::{BaseCstAdmissionLimits, SourceStore};
use wosy_project::{ArtifactConfig, ArtifactProfile, SelectedArtifactGeneration, StagedArtifactGeneration};

pub(super) fn enabled() -> bool {
    env::var_os("WOSY_INCREMENTAL").is_some_and(|value| value == "1")
}

fn require_private_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(format!("{} is not a private incremental directory", path.display()));
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<(), String> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    require_private_directory(path)
}

fn sibling_store(artifact: &Path) -> std::path::PathBuf {
    artifact.join("incremental")
}

pub(super) fn select_for_run(manifest: &Path) -> Result<Option<SelectedArtifactGeneration>, String> {
    if !enabled() { return Ok(None); }
    let store = sibling_store(manifest.parent().ok_or("artifact manifest has no directory")?);
    match fs::symlink_metadata(&store) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            require_private_directory(&store)?;
            SelectedArtifactGeneration::open(&store).map_err(|e| e.to_string())
        }
    }
}

pub(super) fn build(
    root: &Path, target: &str, profile: &str, ordinary_paths: &BuilderInputPaths,
    config: &ProjectConfiguration, config_identity: &str, artifact: &ArtifactConfig,
    artifact_profile: &ArtifactProfile, observations: &[SourceObservation], llvm: &LlvmPartition,
) -> Result<bool, String> {
    let mut partition = Vec::new();
    llvm.write_text(&mut partition).map_err(|e| e.to_string())?;
    let pending = pending_builder_inputs(config, config_identity.into(), target, profile, artifact,
        artifact_profile, observations.to_vec(), blake3::hash(&partition).to_hex().to_string(), &ordinary_paths.output)?;
    let runtime_ir = wosy_compiler::core_runtime(&artifact_profile.backend, &artifact_profile.output)?;
    let comparison = match selected_source(root, target, profile, Some(&pending)) {
        Ok(comparison) => comparison,
        Err(error) => {
            eprintln!("incremental comparison unavailable; running ordinary builder: {error}");
            None
        }
    };
    publish_generation(root, target, profile, ordinary_paths, pending, &partition, runtime_ir, comparison, false)
}

fn compiler_partition_key(
    source: &AcknowledgedSourceGeneration, profile: &str, configuration: &str,
) -> std::io::Result<blake3::Hash> {
    let image = wosy_protocol::compiler_image_digest(fs::File::open("/proc/self/exe")?)?;
    let mut key = blake3::Hasher::new_derive_key("Wosy ordinary authored compiler result v1");
    for component in [
        source.selection().project.as_bytes(), source.selection().target.as_bytes(),
        &source.selection().root.0, profile.as_bytes(), configuration.as_bytes(), &image.0,
    ] {
        key.update(&(component.len() as u64).to_le_bytes());
        key.update(component);
    }
    Ok(key.finalize())
}

pub(super) fn try_retained_partition(
    root: &Path, target: &str, profile: &str, paths: &BuilderInputPaths,
    config: &ProjectConfiguration, config_identity: &str, artifact: &ArtifactConfig,
    artifact_profile: &ArtifactProfile,
) -> Result<bool, String> {
    let candidate = (|| -> Result<_, Box<dyn std::error::Error + Send + Sync>> {
        let Some(previous) = select_for_run(&paths.output.join("artifact-manifest.json"))? else { return Ok(None); };
        let manifest = previous.manifest();
        if manifest.identity.configuration_identity != config_identity { return Ok(None); }
        validate_current_generation(root, target, profile, previous.manifest_path())?;
        let Some((state, source)) = selected_source(root, target, profile, None)? else { return Ok(None); };
        let key = compiler_partition_key(&source, profile, config_identity)?;
        let Some(bytes) = previous.retained_compiler_partition(&key)? else { return Ok(None); };
        let pending = pending_builder_inputs(config, config_identity.into(), target, profile, artifact,
            artifact_profile, manifest.source_observations.clone(), manifest.identity.llvm_input_identity.clone(), &paths.output)?;
        if pending.manifest.identity != manifest.identity { return Ok(None); }
        Ok(Some((pending, bytes, state, source)))
    })();
    let Ok(Some((pending, bytes, state, source))) = candidate else { return Ok(false); };
    let runtime = wosy_compiler::core_runtime(&artifact_profile.backend, &artifact_profile.output)?;
    publish_generation(root, target, profile, paths, pending, &bytes, runtime, Some((state, source)), true)
}

fn publish_generation(
    root: &Path, target: &str, profile: &str, ordinary_paths: &BuilderInputPaths,
    mut pending: PendingBuilderInputs, partition: &[u8], runtime_ir: &str,
    mut comparison: Option<(SqliteSourceState, AcknowledgedSourceGeneration)>, warm: bool,
) -> Result<bool, String> {
    fs::create_dir_all(&ordinary_paths.output).map_err(|e| e.to_string())?;
    let destination = sibling_store(&ordinary_paths.output);
    private_directory(&destination)?;
    let mut stage = StagedArtifactGeneration::create(&destination).map_err(|e| e.to_string())?;
    let paths = BuilderInputPaths::new(stage.output_dir());
    pending.manifest.executable = paths.output.join("main");
    if warm {
        let Some((state, source)) = comparison.as_mut() else { return Ok(false); };
        if !state.revalidate_ordinary_build_inputs(source, profile, &pending, Duration::from_secs(120)).unwrap_or(false) {
            return Ok(false);
        }
        eprintln!("incremental build reused compiler LLVM for {}", source.selection().root.hex());
    }
    stage.materialize_llvm_inputs(partition, runtime_ir.as_bytes()).map_err(|e| e.to_string())?;
    invoke_unpublished_builder(root, &paths, &pending)?;
    fs::remove_file(&paths.pending_manifest).map_err(|e| e.to_string())?;
    match comparison {
        Some((mut state, source)) => {
            if let Ok(key) = compiler_partition_key(&source, profile, &pending.manifest.identity.configuration_identity) {
                let _ = stage.record_compiler_partition(&key, &pending.manifest.identity.llvm_input_identity);
            }
            if state.publish_ordinary_build_generation(&source, profile, &pending, stage,
                Duration::from_secs(120), usize::MAX).map_err(|e| e.to_string())?.is_none() {
                return Err("workspace changed during incremental build".into());
            }
        }
        None => {
            // Same publication slot and writer reservation as a verified
            // build. A comparison miss never grants reuse of old binaries.
            let prepared = stage.prepare(&pending.manifest).map_err(|e| e.to_string())?;
            validate_current_generation(root, target, profile, &paths.output.join("artifact-manifest.json"))?;
            prepared.publish().map_err(|e| e.to_string())?;
        }
    }
    Ok(true)
}

fn validate_current_generation(root: &Path, target: &str, profile: &str, manifest: &Path) -> Result<(), String> {
    let (current, document) = ProjectConfiguration::load_with_manifest(root)?;
    let identity = project_configuration_identity(document.raw().as_bytes(), &current)?;
    let (_, artifact, artifact_profile, _) = current.artifact_profile(target, profile)?;
    validate_run_manifest(manifest, &current, &identity, target, profile, artifact, artifact_profile)?;
    Ok(())
}

fn selected_source(
    root: &Path, target: &str, profile: &str, pending: Option<&PendingBuilderInputs>,
) -> Result<Option<(SqliteSourceState, wosy_cli::source_state::AcknowledgedSourceGeneration)>, Box<dyn std::error::Error + Send + Sync>> {
    let session = tokio::runtime::Builder::new_current_thread().enable_time().build()?;
    let owner = root.join(".wosy");
    let source_store = owner.join("incremental-daemon");
    fs::create_dir_all(&owner)?;
    private_directory(&source_store)?;
    let store = session.block_on(async { SourceStore::open(&source_store, 8 << 20) }).map(Arc::new)?;
    let comparison = session.block_on(async {
        let mut state = SqliteSourceState::open(root, ClientSourceLimits { nodes: usize::MAX, encoded_bytes: usize::MAX })?;
        if !establish_local_build_source(&mut state, &store, target).await? { return Ok(None); }
        if pending.is_none() {
            let Some(source) = state.capture_acknowledged_source(target)? else { return Ok(None); };
            let Some(view) = store.snapshot(source.selection().root)? else { return Ok(None); };
            if !store.ordinary_view_has_no_comptime(&view)? { return Ok(None); }
            return Ok(Some((state, source)));
        }
        let pending = pending.expect("cold comparison includes ordinary inputs");
        let limits = BaseCstAdmissionLimits {
            records: usize::MAX, record_bytes: usize::MAX, graph_edges: usize::MAX,
            expanded_record_bytes: usize::MAX, cst_occurrences: usize::MAX, cst_depth: usize::MAX,
            authored_bytes: usize::MAX,
        };
        let source = compare_local_build(&mut state, Arc::clone(&store), target, profile, pending,
            limits, wosy_protocol::MAX_SESSION_FRAME_BYTES, Duration::from_secs(120)).await?;
        let Some(source) = source else { return Ok(None); };
        let Some(view) = store.snapshot(source.selection().root)? else { return Ok(None); };
        if !store.ordinary_view_has_no_comptime(&view)? { return Ok(None); }
        Ok(Some((state, source)))
    });
    let comparison: Result<_, Box<dyn std::error::Error + Send + Sync>> = comparison;
    if let Some(store) = Arc::into_inner(store) {
        session.block_on(store.close())?;
    }
    comparison
}
