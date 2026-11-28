//! Opt-in ordinary Build→Run through selected source and retained generations.
use super::*;
use std::{num::NonZeroUsize, os::unix::fs::{DirBuilderExt, PermissionsExt}, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_cli::{
    local_build_daemon::{compare_owned_child, establish_local_build_source, stage_local_build_source, SourceStoreCustody},
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
    let comparison = match selected_source(root, target, profile, Some(&pending), false) {
        Ok(comparison) => comparison,
        Err(error) => {
            eprintln!("incremental comparison unavailable; running ordinary builder: {error}");
            None
        }
    };
    publish_generation(root, target, profile, ordinary_paths, pending, &partition, runtime_ir, comparison, false, None)
}

struct SelectedOrdinarySource {
    state: SqliteSourceState,
    acknowledged: AcknowledgedSourceGeneration,
    compiler_partition: Option<blake3::Hash>,
}

fn compiler_partition_key(
    store: &SourceStore, source: &AcknowledgedSourceGeneration, profile: &str,
) -> Result<Option<blake3::Hash>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(view) = store.snapshot(source.selection().root)? else { return Ok(None); };
    let Some(inputs) = view.ordinary_authored_llvm_inputs(profile)? else { return Ok(None); };
    let image = wosy_protocol::compiler_image_digest(fs::File::open("/proc/self/exe")?)?;
    // A positive receipt is seeded only by exact daemon/ordinary LLVM
    // comparison plus proven no-comptime. It covers the compiler input
    // projection, not the whole artifact configuration or any native tool.
    let mut key = blake3::Hasher::new_derive_key("Wosy ordinary authored no-comptime compiler result v3");
    key.update(&image.0).update(&inputs.0);
    Ok(Some(key.finalize()))
}

pub(super) fn try_retained_partition(
    root: &Path, target: &str, profile: &str, paths: &BuilderInputPaths,
    config: &ProjectConfiguration, config_identity: &str, artifact: &ArtifactConfig,
    artifact_profile: &ArtifactProfile,
) -> Result<bool, String> {
    let candidate = (|| -> Result<_, Box<dyn std::error::Error + Send + Sync>> {
        let Some(previous) = select_for_run(&paths.output.join("artifact-manifest.json"))? else { return Ok(None); };
        let manifest = previous.manifest();
        if manifest.comptime.is_some() { return Ok(None); }
        let configuration_changed = manifest.identity.configuration_identity != config_identity;
        let Some(selected) = selected_source(root, target, profile, None, configuration_changed)? else { return Ok(None); };
        let Some(key) = selected.compiler_partition else { return Ok(None); };
        let Some(bytes) = previous.retained_compiler_partition(&key)? else { return Ok(None); };
        let pending = pending_builder_inputs(config, config_identity.into(), target, profile, artifact,
            artifact_profile, manifest.source_observations.clone(), manifest.identity.llvm_input_identity.clone(), &paths.output)?;
        Ok(Some((pending, bytes, selected)))
    })();
    let Ok(Some((pending, bytes, selected))) = candidate else { return Ok(false); };
    let runtime = wosy_compiler::core_runtime(&artifact_profile.backend, &artifact_profile.output)?;
    publish_generation(root, target, profile, paths, pending, &bytes, runtime, Some(selected), true, None)
}

pub(super) fn try_selected_comptime(
    root: &Path, target: &str, profile: &str, paths: &BuilderInputPaths,
    config_identity: &str, artifact_identity_name: &str,
) -> Result<bool, String> {
    use wosy_cli::source_state::SelectedPairToolBudget;
    use wosy_daemon::{SelectedComptimePreparation, SourceStorageWorker};
    type BoxError = Box<dyn std::error::Error + Send + Sync>;
    const LIFETIME: Duration = Duration::from_secs(120);

    if wosy_project::SelectedComptimeTools::load(root, target, profile)?.is_none() {
        return Ok(false);
    }
    let opened = (|| -> Result<_, BoxError> {
        let Some(custody) = SourceStoreCustody::try_acquire(root)? else { return Ok(None); };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build()?;
        let store_path = custody.store_path();
        let store = runtime.block_on(async { custody.open_store() }).map(Arc::new)?;
        Ok(Some((custody, runtime, store_path, store)))
    })();
    let (custody, runtime, store_path, store) = match opened {
        Ok(Some(opened)) => opened,
        Ok(None) => return Ok(false),
        Err(error) => {
            eprintln!("incremental comptime preparation unavailable; trying ordinary Build: {error}");
            return Ok(false);
        }
    };
    let mut admitted = false;
    let attempt = runtime.block_on(async {
        let mut state = SqliteSourceState::open(root, ClientSourceLimits { nodes: usize::MAX, encoded_bytes: usize::MAX })?;
        if !establish_local_build_source(&mut state, &store, target).await? { return Ok(None); }
        let Some(selected) = state.acknowledged_build_source(target)? else { return Ok(None); };
        let one = NonZeroUsize::MIN;
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one)?;
        let Some(session) = worker.open_build_diagnostics(selected, LIFETIME).await? else { return Ok(None); };
        let limits = BaseCstAdmissionLimits {
            records: usize::MAX, record_bytes: usize::MAX, graph_edges: usize::MAX,
            expanded_record_bytes: usize::MAX, cst_occurrences: usize::MAX, cst_depth: usize::MAX,
            authored_bytes: usize::MAX,
        };
        let budget = SelectedPairToolBudget {
            watch_limits: wosy_project::packages::ComptimeWatchLimits {
                entries: usize::MAX, files: usize::MAX, content_bytes: u64::MAX,
            },
            evaluator_ir_bytes: usize::MAX, evaluator_module_bytes: u64::MAX, timeout: LIFETIME,
        };
        let result = match session.prepare_selected_wasip1_comptime(profile.to_owned(), limits, usize::MAX).await? {
            SelectedComptimePreparation::Pair(owner) => {
                admitted = true;
                state.prepare_selected_pair_ordinary_inputs(&owner, &store_path, &paths.output, budget, || false)?
            }
            SelectedComptimePreparation::Ordered(owner) => {
                admitted = true;
                state.prepare_selected_ordered_ordinary_inputs(&owner, &store_path, &paths.output, budget, || false)?
            }
            SelectedComptimePreparation::Exports(owner) => {
                admitted = true;
                state.prepare_selected_exports_ordinary_inputs(&owner, &store_path, &paths.output, budget, || false)?
            }
            SelectedComptimePreparation::Refused(_) => return Ok(None),
        };
        Ok::<_, BoxError>(result.map(|result| (state, result)))
    });
    let close = match Arc::into_inner(store) {
        Some(store) => runtime.block_on(store.close()).map_err(|error| error.to_string()),
        None => Err("incremental comptime worker retained the source store".into()),
    };
    // Join any remaining store work before another process can reserve it.
    drop(runtime);
    drop(custody);
    if let Err(error) = close {
        if attempt.is_ok() {
            if admitted { return Err(format!("incremental comptime source store close failed: {error}")); }
            eprintln!("incremental source store close failed: {error}");
            return Ok(false);
        }
    }
    let (state, result) = match attempt {
        Ok(Some(result)) => result,
        Ok(None) => return Ok(false),
        Err(error) if admitted => return Err(format!("selected comptime tool execution did not complete: {error}")),
        Err(error) => {
            eprintln!("incremental comptime preparation unavailable; trying ordinary Build: {error}");
            return Ok(false);
        }
    };
    let pending = result.inputs();
    let identity = &pending.manifest.identity;
    if identity.configuration_identity != config_identity
        || identity.artifact_identity != format!("{target}:{profile}:{artifact_identity_name}")
    {
        return Ok(false);
    }
    let (source, pending, partition, runtime, compiler_partition) = result.into_builder_inputs();
    let selected = SelectedOrdinarySource { state, acknowledged: source, compiler_partition: None };
    publish_generation(root, target, profile, paths, pending, &partition, runtime, Some(selected),
        false, Some(compiler_partition))
}

fn publish_generation(
    root: &Path, target: &str, profile: &str, ordinary_paths: &BuilderInputPaths,
    mut pending: PendingBuilderInputs, partition: &[u8], runtime_ir: &str,
    mut comparison: Option<SelectedOrdinarySource>, warm: bool,
    comptime_partition: Option<wosy_cli::source_state::SelectedComptimePartitionReceipt>,
) -> Result<bool, String> {
    fs::create_dir_all(&ordinary_paths.output).map_err(|e| e.to_string())?;
    let destination = sibling_store(&ordinary_paths.output);
    private_directory(&destination)?;
    let mut stage = StagedArtifactGeneration::create(&destination).map_err(|e| e.to_string())?;
    let paths = BuilderInputPaths::new(stage.output_dir());
    pending.manifest.executable = paths.output.join("main");
    let selected_comptime = pending.manifest.comptime.is_some();
    if warm || selected_comptime {
        let Some(selected) = comparison.as_mut() else { return Ok(false); };
        if !selected.state.revalidate_ordinary_build_inputs(&selected.acknowledged, profile, &pending, Duration::from_secs(120)).unwrap_or(false) {
            return Ok(false);
        }
        if warm {
            eprintln!("incremental build reused compiler LLVM for {}", selected.acknowledged.selection().root.hex());
        }
    }
    stage.materialize_llvm_inputs(partition, runtime_ir.as_bytes()).map_err(|e| e.to_string())?;
    let builder_started = Instant::now();
    invoke_unpublished_builder(root, &paths, &pending)?;
    let builder_elapsed = builder_started.elapsed();
    fs::remove_file(&paths.pending_manifest).map_err(|e| e.to_string())?;
    match comparison {
        Some(SelectedOrdinarySource { mut state, acknowledged: source, compiler_partition }) => {
            let key = if selected_comptime {
                comptime_partition.as_ref().and_then(|partition| partition.key())
            } else {
                compiler_partition
            };
            if let Some(key) = key {
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
    if let Some(partition) = comptime_partition {
        eprintln!("incremental build {} compiler LLVM after executing current comptime (LLVM {} us; evaluator tools {} us; final builder {} us)",
            if partition.reused() { "reused" } else { "compiled" }, partition.llvm_elapsed().as_micros(),
            partition.tools_elapsed().as_micros(), builder_elapsed.as_micros());
    }
    Ok(true)
}

fn validate_current_generation(root: &Path, target: &str, profile: &str, manifest: &Path) -> Result<(), String> {
    let (current, document) = ProjectConfiguration::load_with_manifest(root)?;
    let identity = project_configuration_identity(document.raw().as_bytes(), &current)?;
    let (_, artifact, artifact_profile, _) = current.artifact_profile(target, profile)?;
    validate_run_manifest(root, manifest, &current, &identity, target, profile, artifact, artifact_profile)?;
    Ok(())
}

fn selected_source(
    root: &Path, target: &str, profile: &str, pending: Option<&PendingBuilderInputs>, refresh: bool,
) -> Result<Option<SelectedOrdinarySource>, Box<dyn std::error::Error + Send + Sync>> {
    let mut runtime = tokio::runtime::Builder::new_current_thread();
    runtime.enable_time();
    if pending.is_some() { runtime.enable_io(); }
    let session = runtime.build()?;
    let Some(custody) = SourceStoreCustody::try_acquire(root)? else { return Ok(None); };
    session.block_on(async {
        let source_started = Instant::now();
        let mut state = SqliteSourceState::open(root, ClientSourceLimits { nodes: usize::MAX, encoded_bytes: usize::MAX })?;
        let store = custody.open_store()?;
        let result = async {
            let source = if pending.is_some() || refresh {
                stage_local_build_source(&mut state, &store, target).await?
            } else {
                state.capture_acknowledged_source(target)?
            };
            let Some(source) = source else { return Ok(None); };
            // The selected immutable graph never grants permission to use
            // stale workspace files. Publication still matches the live
            // selected generation and current manifest/builder configuration.
            let key = compiler_partition_key(&store, &source, profile).ok().flatten();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Some((source, key)))
        }.await;
        // SurrealKV's owner moves to the private child serially. Nothing
        // in the parent keeps this store open across process activation.
        store.close().await?;
        let Some((staged, mut key)) = result? else { return Ok(None); };
        let Some(pending) = pending else {
            return Ok(Some(SelectedOrdinarySource { state, acknowledged: staged, compiler_partition: key }));
        };
        let source_elapsed = source_started.elapsed();
        let child_started = Instant::now();
        let Some((source, replayed)) = compare_owned_child(&mut state, &custody, target, profile, pending).await? else { return Ok(None); };
        if source.selection() != staged.selection() || source.generation() != staged.generation() { return Ok(None); }
        if key.is_none() && replayed {
            // The exact graph did not exist when the source cut was staged.
            // Replay has closed; read its immutable root, never a newer cut.
            if let Ok(store) = custody.open_store() {
                key = compiler_partition_key(&store, &source, profile).ok().flatten();
                store.close().await?;
            }
        }
        eprintln!("incremental compiler LLVM compared over local Unix socket for {target}:{profile} in an owned compiler process{} (source store {} us; owned comparison {} us)",
            if replayed { " after source replay" } else { "" },
            source_elapsed.as_micros(), child_started.elapsed().as_micros());
        Ok(Some(SelectedOrdinarySource { state, acknowledged: source, compiler_partition: key }))
    })
}
