//! Render a verified diagnostic response against its immutable local source.
use super::diagnostics::{diagnostic_deadline, require_diagnostic_time, within_diagnostic_deadline, CommandExchange};
use super::{
    checked_file_key, load_usage, read_selection, AcknowledgedSourceGeneration, CapturedBuildDiagnostics, DiagnosticExchange,
    RequiredSourceReplay, Result, SourceReplay, SqliteSourceState, StoredNodes,
};
use crate::{
    artifact_inputs::{expected_artifact_compiler_identity, pending_builder_inputs, BuilderInputPaths, ObservedBuilderBasis, PendingBuilderInputs},
    project_sources::LiveSourceReader, source_preparation::verify_live_source,
};
use codespan_reporting::term::termcolor::WriteColor;
use rusqlite::TransactionBehavior;
use std::{collections::{BTreeMap, HashMap}, fs::File, future::{poll_fn, Future}, io::Seek, path::{Path, PathBuf}, pin::pin,
    task::Poll, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use wosy_compiler::Diagnostic;
use wosy_project::PackageRelativePath;
use wosy_protocol::{
    walk_cst, BuildDiagnosticResponse, BuildSessionRefusal, CstEvent, CstExpansionBudget,
    BuildSessionReply, BuildSessionRequest, BuildSourceSelection, FileEntry, FileKey, NodeId, OrdinaryLlvmProjection,
    MAX_ORDINARY_LLVM_PROFILE_BYTES, MAX_SESSION_FRAME_BYTES, ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION, ORDINARY_LLVM_BUILDER_INPUT_VERSION,
    ORDINARY_LLVM_PROJECTION_VERSION,
};
use wosy_syntax::SourceIdentity;

pub enum ProjectDiagnosticExchange {
    StaleSelection,
    ReplayRequired(RequiredSourceReplay),
    Refused(BuildSessionRefusal),
    Report(RootedDiagnosticReport),
}

pub enum AcknowledgedBuildDiagnostics {
    NotAcknowledged,
    StaleSelection,
    WorkspaceChanged,
    ProfileNotSelected,
    ReplayNotAccepted,
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    NativeReplayRequired,
    Refused(BuildSessionRefusal),
    Report(RootedDiagnosticReport),
}

/// A fresh verified LLVM partition for the still-acknowledged generation.
/// Do not treat these bytes as artifact/build permission or a verdict.
pub struct AcknowledgedLlvmPartition {
    captured: AcknowledgedSourceGeneration,
    projection: OrdinaryLlvmProjection,
}

impl AcknowledgedLlvmPartition {
    pub fn source(&self) -> &AcknowledgedSourceGeneration { &self.captured }
    pub fn projection(&self) -> &OrdinaryLlvmProjection { &self.projection }
}

pub enum AcknowledgedOrdinaryLlvm<T = AcknowledgedLlvmPartition> {
    NotAcknowledged,
    StaleSelection,
    WorkspaceChanged,
    ProfileNotSelected,
    ReplayNotAccepted,
    Refused(BuildSessionRefusal),
    Partition(T),
}

/// Expected bytes of the executable already opened by the trusted local
/// activator. No reply field can create it. This hash does not authenticate
/// an OS peer or associate an arbitrary stream with that executable.
pub struct AuthorizedOrdinaryBuilderPeer(NodeId);

impl AuthorizedOrdinaryBuilderPeer {
    pub fn from_open_executable(mut file: File) -> Result<Self> {
        if !file.metadata()?.is_file() { return Err("ordinary builder peer needs a regular executable file".into()); }
        file.rewind()?;
        Ok(Self(wosy_protocol::compiler_image_digest(file)?))
    }
}

/// Exact pending ordinary no-cache builder inputs. This type deliberately
/// has no write, spawn, artifact selection or publication method.
pub struct AcknowledgedOrdinaryBuilderInputs {
    source: AcknowledgedSourceGeneration,
    project: PathBuf,
    paths: BuilderInputPaths,
    inputs: PendingBuilderInputs,
    partition: String,
    runtime: &'static str,
    executable_partition_digest: Option<NodeId>,
    #[cfg(target_os = "linux")]
    profile: String,
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    origin: BuilderPlanOrigin,
}

#[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
#[derive(Clone, Copy, Eq, PartialEq)]
enum BuilderPlanOrigin { LocalAdapter, OwnedNativeChild }

/// Exact staged invocation inputs, not custody of the driver image, workspace
/// mutations, or permission to execute. A later controller must supply them.
#[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
struct PreparedNativeBuildInvocation {
    plan: AcknowledgedOrdinaryBuilderInputs,
    stage: wosy_project::StagedArtifactGeneration,
}

#[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
impl PreparedNativeBuildInvocation {
    fn prepare(
        plan: AcknowledgedOrdinaryBuilderInputs,
        stage: wosy_project::StagedArtifactGeneration,
    ) -> Result<Option<Self>> {
        if plan.origin != BuilderPlanOrigin::OwnedNativeChild
            || plan.executable_partition_digest.is_none()
            || plan.inputs.manifest.identity.output != "wasm-wasip1"
            || plan.paths.output != stage.output_dir()
        {
            return Ok(None);
        }
        stage.materialize_llvm_inputs(plan.partition.as_bytes(), plan.runtime.as_bytes())?;
        Ok(Some(Self { plan, stage }))
    }
}

impl AcknowledgedOrdinaryBuilderInputs {
    pub fn source(&self) -> &AcknowledgedSourceGeneration { &self.source }
    pub fn project(&self) -> &Path { &self.project }
    pub fn paths(&self) -> &BuilderInputPaths { &self.paths }
    pub fn inputs(&self) -> &PendingBuilderInputs { &self.inputs }
    pub fn partition(&self) -> &str { &self.partition }
    pub fn runtime(&self) -> &'static str { self.runtime }
    /// Comparison data for explicitly requested executable LLVM, not an artifact reuse key.
    pub fn executable_partition_digest(&self) -> Option<NodeId> { self.executable_partition_digest }
}

/// Selected-source compiler result with its actual final comptime input
/// identity. This object is inert; artifact Build owns invocation/publication.
#[cfg(target_os = "linux")]
pub struct SelectedPairOrdinaryInputs {
    source: AcknowledgedSourceGeneration,
    result_root: wosy_protocol::ComptimeResultRoot,
    inputs: PendingBuilderInputs,
    partition: Vec<u8>,
    runtime: &'static str,
}

#[cfg(target_os = "linux")]
impl SelectedPairOrdinaryInputs {
    pub fn source(&self) -> &AcknowledgedSourceGeneration { &self.source }
    pub fn result_root(&self) -> wosy_protocol::ComptimeResultRoot { self.result_root }
    pub fn inputs(&self) -> &PendingBuilderInputs { &self.inputs }
    pub fn partition(&self) -> &[u8] { &self.partition }
    pub fn runtime(&self) -> &'static str { self.runtime }
    pub fn into_builder_inputs(self) -> (AcknowledgedSourceGeneration, PendingBuilderInputs, Vec<u8>, &'static str) {
        (self.source, self.inputs, self.partition, self.runtime)
    }
}

#[cfg(target_os = "linux")]
pub struct SelectedPairToolBudget {
    pub watch_limits: wosy_project::packages::ComptimeWatchLimits,
    pub evaluator_ir_bytes: usize,
    pub evaluator_module_bytes: u64,
    pub timeout: Duration,
}

struct LiveCapture {
    captured: CapturedBuildDiagnostics,
    builder: Option<ObservedBuilderBasis>,
}

enum LiveCaptureRefusal { NotAcknowledged, StaleSelection, WorkspaceChanged, ProfileNotSelected }

impl From<LiveCaptureRefusal> for AcknowledgedBuildDiagnostics {
    fn from(refusal: LiveCaptureRefusal) -> Self {
        match refusal {
            LiveCaptureRefusal::NotAcknowledged => Self::NotAcknowledged,
            LiveCaptureRefusal::StaleSelection => Self::StaleSelection,
            LiveCaptureRefusal::WorkspaceChanged => Self::WorkspaceChanged,
            LiveCaptureRefusal::ProfileNotSelected => Self::ProfileNotSelected,
        }
    }
}

impl<T> From<LiveCaptureRefusal> for AcknowledgedOrdinaryLlvm<T> {
    fn from(refusal: LiveCaptureRefusal) -> Self {
        match refusal {
            LiveCaptureRefusal::NotAcknowledged => Self::NotAcknowledged,
            LiveCaptureRefusal::StaleSelection => Self::StaleSelection,
            LiveCaptureRefusal::WorkspaceChanged => Self::WorkspaceChanged,
            LiveCaptureRefusal::ProfileNotSelected => Self::ProfileNotSelected,
        }
    }
}

enum Recovery {
    LocalAdapter,
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    NativeKnown,
}

#[derive(Clone, Copy)]
enum OrdinaryLlvmMode {
    Projection,
    KnownBuilderInputs,
    KnownExecutableBuilderInputs,
}

impl SqliteSourceState {
    /// Analyze the acknowledged source, using a trusted adapter to open a
    /// fresh, exclusively owned stream for its exact project/target/root.
    /// After a terminal replay request it may open one more stream with the
    /// same captured SQLite generation's bounded canonical source. A later
    /// source (even identical bytes newly acknowledged) is never substituted.
    ///
    /// One deadline covers both openings and framed exchanges. The adapter
    /// must cooperate with asynchronous cancellation; a synchronous call
    /// or a running poll cannot be preempted. Replay and authored projection
    /// retain their separate read budgets. This neither advances the
    /// selection nor emits artifacts.
    pub async fn acknowledged_build_diagnostics<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, budget: CstExpansionBudget, open: Open,
    ) -> Result<AcknowledgedBuildDiagnostics>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        use AcknowledgedBuildDiagnostics as Outcome;
        let deadline = diagnostic_deadline(timeout)?;
        let Some(captured) = self.capture_build_diagnostics(target, profile)? else {
            return Ok(Outcome::NotAcknowledged);
        };
        self.diagnostics_for_capture(captured, deadline, budget, open, Recovery::LocalAdapter).await
    }

    /// Opt in only for a command observing the live workspace. A fresh
    /// ordinary source observation must match the captured acknowledged
    /// root and profile before any stream is opened. Changed files are not
    /// parsed here and no pending or historical source is substituted.
    ///
    /// This uses the same single observation cut as ordinary source loading,
    /// not a filesystem lock or a promise that worktree bytes cannot change
    /// afterward. The same captured SQLite generation owns the entire
    /// exchange. The authored source byte cap is aggregate across reachable
    /// files, separate from SQLite and diagnostic projection limits. It does
    /// not limit manifest, lockfile or installed Git metadata reads by the
    /// ordinary resolver. Live local files use nonblocking Linux open flags
    /// and are checked on the opened handle; unsupported platforms refuse.
    /// The deadline is checked between synchronous phases and source reads,
    /// not a way to preempt an in-progress filesystem syscall.
    ///
    /// Recovery may replay the exact acknowledged root into a missing daemon
    /// cache. This does not stage, publish a new observation, advance the
    /// selection, emit an artifact or produce a verdict.
    pub async fn live_build_diagnostics<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, max_authored_source_bytes: usize,
        budget: CstExpansionBudget, open: Open,
    ) -> Result<AcknowledgedBuildDiagnostics>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let deadline = diagnostic_deadline(timeout)?;
        let captured = match self.capture_live(target, profile, deadline, max_authored_source_bytes, false)? {
            Ok(capture) => capture.captured,
            Err(outcome) => return Ok(outcome.into()),
        };
        self.diagnostics_for_capture(captured, deadline, budget, open, Recovery::LocalAdapter).await
    }

    /// Use the already acknowledged live source and a protected one-shot
    /// Linux Unix endpoint. The endpoint's trust is the local activator and
    /// same effective uid. This never sends replay data, stages source, or
    /// changes the ordinary CLI; a missing daemon root is explicit.
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    pub async fn native_known_build_diagnostics(
        &mut self, directory: &std::path::Path, target: &str, profile: &str, timeout: Duration,
        max_authored_source_bytes: usize, budget: CstExpansionBudget,
    ) -> Result<AcknowledgedBuildDiagnostics> {
        let deadline = diagnostic_deadline(timeout)?;
        let captured = match self.capture_live(target, profile, deadline, max_authored_source_bytes, false)? {
            Ok(capture) => capture.captured,
            Err(outcome) => return Ok(outcome.into()),
        };
        self.diagnostics_for_capture(captured, deadline, budget, |_, _| async move {
            Ok(wosy_protocol::native_diagnostics::connect(directory).await?)
        }, Recovery::NativeKnown).await
    }

    /// Explicit native V3 observation through the project activator's own
    /// direct child. The pinned selection comes from activation, not the
    /// socket; an executable digest alone never grants this route. No
    /// artifact write, builder run, cache reuse or default CLI change.
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    pub async fn native_owned_executable_builder_inputs(
        &mut self, directory: &std::path::Path, child: &mut std::process::Child, executable: &File,
        selected: &BuildSourceSelection, profile: &str, timeout: Duration, max_authored_source_bytes: usize,
        max_partition_bytes: usize, output: PathBuf,
    ) -> Result<AcknowledgedOrdinaryLlvm<AcknowledgedOrdinaryBuilderInputs>> {
        let deadline = llvm_deadline(profile, timeout)?;
        let expected_peer = AuthorizedOrdinaryBuilderPeer::from_open_executable(executable.try_clone()?)?;
        require_diagnostic_time(deadline)?;
        let mut child = Some(child);
        let result = self.live_executable_builder_inputs(&selected.target, profile, deadline.saturating_duration_since(Instant::now()),
            max_authored_source_bytes, max_partition_bytes, &expected_peer, output,
            |actual| {
                let child = child.take();
                async move {
                    if &actual != selected {
                        return Err("native V3 source differs from the activated selection".into());
                    }
                    let child = child.ok_or("native V3 activation can only open one connection")?;
                    Ok(wosy_protocol::native_diagnostics::connect_owned_child(directory, child, executable).await?)
                }
            }).await?;
        Ok(match result {
            AcknowledgedOrdinaryLlvm::Partition(mut plan) => {
                plan.origin = BuilderPlanOrigin::OwnedNativeChild;
                AcknowledgedOrdinaryLlvm::Partition(plan)
            }
            other => other,
        })
    }

    /// Prepare only inputs for the caller-owned generational workspace and
    /// the genuine owned compiler child; the next step is not implicit.
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    async fn prepare_native_owned_build_invocation(
        &mut self, directory: &Path, child: &mut std::process::Child, executable: &File,
        selected: &BuildSourceSelection, profile: &str, timeout: Duration,
        max_authored_source_bytes: usize, max_partition_bytes: usize,
        stage: wosy_project::StagedArtifactGeneration,
    ) -> Result<Option<PreparedNativeBuildInvocation>> {
        // A stage rooted at the project directory would hold its advisory
        // writer lock while the live capture attempts a project read lock.
        if !stage.is_workspace_subdirectory(&self.project)? {
            return Ok(None);
        }
        let output = stage.output_dir();
        let result = self.native_owned_executable_builder_inputs(
            directory, child, executable, selected, profile, timeout,
            max_authored_source_bytes, max_partition_bytes, output,
        ).await?;
        let AcknowledgedOrdinaryLlvm::Partition(plan) = result else { return Ok(None); };
        PreparedNativeBuildInvocation::prepare(plan, stage)
    }

    /// Explicit local-adapter request for ordinary authored LLVM (not an
    /// incremental Build verdict). The adapter owns its dedicated stream
    /// and grants the separate server capability for exactly this source.
    /// One expiry covers live capture, one optional replay, and close.
    /// Memory read/framing budgets are distinct from the partition byte
    /// limit. Recovery can restore the already selected source only. This
    /// never stages new source, writes an artifact, or builds or runs code.
    pub async fn live_ordinary_llvm<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, max_authored_source_bytes: usize,
        max_partition_bytes: usize, open: Open,
    ) -> Result<AcknowledgedOrdinaryLlvm>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let deadline = llvm_deadline(profile, timeout)?;
        let captured = match self.capture_live(target, profile, deadline, max_authored_source_bytes, false)? {
            Ok(capture) => capture.captured,
            Err(outcome) => return Ok(outcome.into()),
        };
        self.live_partition_for_capture(captured, deadline, max_partition_bytes, OrdinaryLlvmMode::Projection,
            open, |captured, projection| Ok(AcknowledgedLlvmPartition { captured, projection }),
        ).await
    }

    /// Compare the ordinary authored partition against the selected daemon
    /// result. A cache miss or an unverified source is not a build error.
    /// The caller owns the peer transport; the image digest is a consistency
    /// check and does not authenticate a network peer.
    pub async fn compare_selected_ordinary_llvm<Open, Opening, Stream>(
        &mut self,
        target: &str,
        profile: &str,
        timeout: Duration,
        max_authored_source_bytes: usize,
        max_partition_bytes: usize,
        expected_peer: &AuthorizedOrdinaryBuilderPeer,
        ordinary: &PendingBuilderInputs,
        open: Open,
    ) -> Result<Option<AcknowledgedSourceGeneration>>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let deadline = llvm_deadline(profile, timeout)?;
        let Ok(initial) = self.capture_live(target, profile, deadline, max_authored_source_bytes, true)? else {
            return Ok(None);
        };
        if !matches_ordinary_builder(initial.builder, target, profile, ordinary)? {
            return Ok(None);
        }
        let answer = self.live_partition_for_capture(
            initial.captured, deadline, max_partition_bytes, OrdinaryLlvmMode::Projection,
            open, |captured, projection| Ok(AcknowledgedLlvmPartition { captured, projection }),
        ).await?;
        let AcknowledgedOrdinaryLlvm::Partition(partition) = answer else {
            return Ok(None);
        };
        let projected = partition.projection();
        if projected.compiler_image != expected_peer.0
            || projected.digest.hex() != ordinary.manifest.identity.llvm_input_identity
        {
            return Ok(None);
        }
        let Ok(current) = self.capture_live(target, profile, deadline, max_authored_source_bytes, true)? else {
            return Ok(None);
        };
        if current.captured.source.generation() != partition.captured.generation()
            || current.captured.selection() != partition.captured.selection()
            || !matches_ordinary_builder(current.builder, target, profile, ordinary)?
        {
            return Ok(None);
        }
        Ok(Some(partition.captured))
    }

    /// Plan the exact ordinary builder inputs without writing, invoking or
    /// caching artifacts. The trusted local activator binds the dedicated
    /// stream to the executable it independently expects. The image
    /// comparison checks consistency, not OS peer authentication. A
    /// missing known root is refused without preparing or sending replay.
    /// The local manifest, canonical graph and build/runtime choices come
    /// only from the same verified live source cut. This is not an
    /// ArtifactKey, and does not freeze builder files, environment or mounts.
    pub async fn live_ordinary_builder_inputs<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, max_authored_source_bytes: usize,
        max_partition_bytes: usize, expected_peer: &AuthorizedOrdinaryBuilderPeer, output: PathBuf, open: Open,
    ) -> Result<AcknowledgedOrdinaryLlvm<AcknowledgedOrdinaryBuilderInputs>>
    where
        Open: FnMut(BuildSourceSelection) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        self.live_builder_inputs(target, profile, timeout, max_authored_source_bytes, max_partition_bytes,
            expected_peer, output, open, OrdinaryLlvmMode::KnownBuilderInputs).await
    }

    /// Explicit normalized projection. Its bytes can differ from ordinary Build;
    /// the result is still inert and never authorizes a builder or reuse.
    pub async fn live_executable_builder_inputs<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, max_authored_source_bytes: usize,
        max_partition_bytes: usize, expected_peer: &AuthorizedOrdinaryBuilderPeer, output: PathBuf, open: Open,
    ) -> Result<AcknowledgedOrdinaryLlvm<AcknowledgedOrdinaryBuilderInputs>>
    where
        Open: FnMut(BuildSourceSelection) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        self.live_builder_inputs(target, profile, timeout, max_authored_source_bytes, max_partition_bytes,
            expected_peer, output, open, OrdinaryLlvmMode::KnownExecutableBuilderInputs).await
    }

    async fn live_builder_inputs<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, max_authored_source_bytes: usize,
        max_partition_bytes: usize, expected_peer: &AuthorizedOrdinaryBuilderPeer, output: PathBuf, open: Open, mode: OrdinaryLlvmMode,
    ) -> Result<AcknowledgedOrdinaryLlvm<AcknowledgedOrdinaryBuilderInputs>>
    where
        Open: FnMut(BuildSourceSelection) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let deadline = llvm_deadline(profile, timeout)?;
        let LiveCapture { captured, builder } = match self.capture_live(target, profile, deadline, max_authored_source_bytes, true)? {
            Ok(capture) => capture,
            Err(outcome) => return Ok(outcome.into()),
        };
        let builder = builder.ok_or("current source lacks an ordinary builder observation")?;
        let mut open = open;
        let answer = self.live_partition_for_capture(captured, deadline, max_partition_bytes, mode,
            |selected, _| open(selected), |source, projection| bind_builder_inputs(source, builder, projection, expected_peer.0, output),
        ).await?;
        let AcknowledgedOrdinaryLlvm::Partition(plan) = &answer else { return Ok(answer); };
        // This is a second observation, not an atomic snapshot. Each pass
        // admits at most the same authored graph byte budget; both share one
        // deadline. The plan still cannot authorize a later write or run.
        match self.capture_live(target, profile, deadline, max_authored_source_bytes, false)? {
            Ok(current) if current.captured.source.generation() == plan.source().generation()
                && current.captured.selection() == plan.source().selection() => Ok(answer),
            Ok(_) | Err(LiveCaptureRefusal::NotAcknowledged | LiveCaptureRefusal::StaleSelection) =>
                Ok(AcknowledgedOrdinaryLlvm::StaleSelection),
            Err(reason) => Ok(reason.into()),
        }
    }

    async fn live_partition_for_capture<Open, Opening, Stream, Answer>(
        &mut self, captured: CapturedBuildDiagnostics, deadline: Instant, max_partition_bytes: usize,
        mode: OrdinaryLlvmMode, open: Open, bind: impl FnOnce(AcknowledgedSourceGeneration, OrdinaryLlvmProjection) -> Result<Answer>,
    ) -> Result<AcknowledgedOrdinaryLlvm<Answer>>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        use AcknowledgedOrdinaryLlvm as Outcome;
        let output = max_partition_bytes.min(MAX_SESSION_FRAME_BYTES);
        let (version, allow_replay) = match mode {
            OrdinaryLlvmMode::Projection => (ORDINARY_LLVM_PROJECTION_VERSION, true),
            OrdinaryLlvmMode::KnownBuilderInputs => (ORDINARY_LLVM_BUILDER_INPUT_VERSION, false),
            OrdinaryLlvmMode::KnownExecutableBuilderInputs => (ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION, false),
        };
        let answer = self.exchange_for_capture(&captured, deadline, open, allow_replay,
            |capture| capture.ordinary_llvm_request(version),
            |capture, reply| capture.accept_ordinary_llvm(reply, version, output),
        ).await?;
        require_diagnostic_time(deadline)?;
        let source = captured.source.clone();
        let answer = match answer {
            CommandExchange::StaleSelection => Outcome::StaleSelection,
            CommandExchange::ReplayRequired(_) => Outcome::ReplayNotAccepted,
            CommandExchange::Refused(reason) => Outcome::Refused(reason),
            CommandExchange::Answer(projection) => Outcome::Partition(bind(source, projection)?),
        };
        let current = captured.is_current(self)?;
        require_diagnostic_time(deadline)?;
        if !current { return Ok(Outcome::StaleSelection); }
        Ok(answer)
    }

    fn capture_live(
        &mut self, target: &str, profile: &str, deadline: Instant, max_authored_source_bytes: usize,
        observe_builder: bool,
    ) -> Result<std::result::Result<LiveCapture, LiveCaptureRefusal>> {
        use LiveCaptureRefusal as Outcome;
        require_diagnostic_time(deadline)?;
        let Some(captured) = self.capture_build_diagnostics(target, profile)? else {
            return Ok(Err(Outcome::NotAcknowledged));
        };
        let selected = captured.selection();
        let (files, profile_selected) = {
            let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
            if !captured.source.matches(read_selection(&tx, target)?) { return Ok(Err(Outcome::StaleSelection)); }
            let mut read = StoredNodes::new(&tx)?;
            // Bound selected metadata before decoding or copying SQLite
            // values. This is not a bound on the ordinary live file reader.
            let mut remaining = self.limits;
            let mut record = |id| read.charged_record(id, false, |bytes| {
                remaining.nodes = remaining.nodes.checked_sub(1).ok_or("live source index exceeds the record budget")?;
                remaining.encoded_bytes = remaining.encoded_bytes.checked_sub(bytes).ok_or("live source index exceeds the byte budget")?;
                Ok(())
            });
            let (config, entries) = Self::header_using(&self.project, target, selected.root, &mut record)?;
            let profile_selected = config.profiles.contains_key(profile);
            let files = StoredNodes::file_index_using(&config, entries, record)?;
            drop(read);
            tx.commit()?;
            (files, profile_selected)
        };
        require_diagnostic_time(deadline)?;
        let probe = verify_live_source(&self.project, target, &files, selected.root,
            LiveSourceReader::new(max_authored_source_bytes, deadline), observe_builder && profile_selected);
        require_diagnostic_time(deadline)?;
        let Some(verified) = probe? else { return Ok(Err(Outcome::WorkspaceChanged)); };
        if !captured.is_current(self)? { return Ok(Err(Outcome::StaleSelection)); }
        if !profile_selected { return Ok(Err(Outcome::ProfileNotSelected)); }
        Ok(Ok(LiveCapture { captured, builder: verified.builder }))
    }

    async fn diagnostics_for_capture<Open, Opening, Stream>(
        &mut self, captured: CapturedBuildDiagnostics, deadline: Instant, budget: CstExpansionBudget, open: Open,
        recovery: Recovery,
    ) -> Result<AcknowledgedBuildDiagnostics>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        use AcknowledgedBuildDiagnostics as Outcome;
        let replay = match recovery {
            Recovery::LocalAdapter => true,
            #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
            Recovery::NativeKnown => false,
        };
        let answer = self.exchange_for_capture(&captured, deadline, open, replay,
            CapturedBuildDiagnostics::analysis_request, CapturedBuildDiagnostics::accept_analysis,
        ).await?;
        Ok(match captured.finish_project_exchange(self, answer.into(), budget)? {
            ProjectDiagnosticExchange::StaleSelection => Outcome::StaleSelection,
            ProjectDiagnosticExchange::ReplayRequired(_) => match recovery {
                Recovery::LocalAdapter => Outcome::ReplayNotAccepted,
                #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
                Recovery::NativeKnown => Outcome::NativeReplayRequired,
            },
            ProjectDiagnosticExchange::Refused(reason) => Outcome::Refused(reason),
            ProjectDiagnosticExchange::Report(report) => Outcome::Report(report),
        })
    }
    async fn exchange_for_capture<Open, Opening, Stream, Answer>(
        &mut self, captured: &CapturedBuildDiagnostics, deadline: Instant, mut open: Open, allow_replay: bool,
        command: impl Fn(&CapturedBuildDiagnostics) -> BuildSessionRequest + Copy,
        verify: impl Fn(&CapturedBuildDiagnostics, BuildSessionReply<Diagnostic>) -> Result<Answer> + Copy,
    ) -> Result<CommandExchange<Answer>>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        within_diagnostic_deadline(deadline, async {
            let first = open_stream(captured, deadline, None, &mut open).await?;
            let first = captured.converse_command(self, first, deadline, command, verify).await?;
            let CommandExchange::ReplayRequired(challenge) = first else { return Ok(first); };
            if !allow_replay { return Ok(CommandExchange::ReplayRequired(challenge)); }
            let Some(replay) = challenge.prepare(self)? else { return Ok(CommandExchange::StaleSelection); };
            let stream = open_stream(captured, deadline, Some(replay), &mut open).await?;
            captured.converse_command(self, stream, deadline, command, verify).await
        }).await
    }
}

fn llvm_deadline(profile: &str, timeout: Duration) -> Result<Instant> {
    if profile.len() > MAX_ORDINARY_LLVM_PROFILE_BYTES { return Err("ordinary LLVM profile exceeds the byte budget".into()); }
    diagnostic_deadline(timeout)
}

fn matches_ordinary_builder(
    observed: Option<ObservedBuilderBasis>, target: &str, profile: &str, ordinary: &PendingBuilderInputs,
) -> Result<bool> {
    let Some(observed) = observed else { return Ok(false); };
    let (_, artifact, profile_config, _) = observed.configuration.artifact_profile(target, profile)?;
    let output = ordinary.manifest.executable.parent().ok_or("ordinary builder output has no directory")?;
    let mut recomputed = pending_builder_inputs(
        &observed.configuration, observed.configuration_identity, target, profile,
        artifact, profile_config, observed.source, ordinary.manifest.identity.llvm_input_identity.clone(), output,
    )?;
    if let Some(observation) = &ordinary.manifest.comptime {
        #[cfg(target_os = "linux")]
        if !observation.matches_current(&observed.project, target, profile) {
            return Ok(false);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = observation;
            return Ok(false);
        }
        recomputed.manifest.identity.compiler_identity = expected_artifact_compiler_identity(Some(observation));
        recomputed.manifest.comptime = Some(observation.clone());
    }
    Ok(recomputed.manifest == ordinary.manifest && recomputed.builder == ordinary.builder)
}

fn bind_builder_inputs(
    source: AcknowledgedSourceGeneration, observed: ObservedBuilderBasis, projection: OrdinaryLlvmProjection,
    expected_peer_image: NodeId, output: PathBuf,
) -> Result<AcknowledgedOrdinaryBuilderInputs> {
    if projection.compiler_image != expected_peer_image {
        return Err("ordinary builder inputs came from a different compiler image".into());
    }
    let witness = projection.builder.expect("v2 builder evidence checked in the conversation");
    let target = &source.selection().target;
    let (_, artifact, profile, _) = observed.configuration.artifact_profile(target, &projection.profile)?;
    let runtime = wosy_compiler::core_runtime(&profile.backend, &profile.output)?;
    if witness.compiler != wosy_compiler::COMPILER_IDENTITY || witness.llvm_adapter != wosy_compiler::LLVM_ADAPTER_IDENTITY
        || witness.linked_llvm != wosy_compiler::linked_llvm_version()
        || witness.backend != profile.backend || witness.output != profile.output
        || blake3::hash(runtime.as_bytes()).as_bytes() != &witness.runtime_digest.0 {
        return Err("ordinary builder inputs have a different compiler/runtime identity".into());
    }
    let paths = BuilderInputPaths::new(output);
    let inputs = pending_builder_inputs(&observed.configuration, observed.configuration_identity, target, &projection.profile,
        artifact, profile, observed.source, projection.digest.hex(), &paths.output)?;
    let executable_partition_digest = (projection.version == ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION).then_some(projection.digest);
    Ok(AcknowledgedOrdinaryBuilderInputs { project: observed.project, source, paths, inputs,
        partition: projection.partition, runtime, executable_partition_digest,
        #[cfg(target_os = "linux")]
        profile: projection.profile,
        #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
        origin: BuilderPlanOrigin::LocalAdapter })
}

#[cfg(target_os = "linux")]
impl SqliteSourceState {
    /// Execute only the acknowledged selected plan and prepare its ordinary
    /// builder inputs. The real compiler result and all declared inputs are
    /// checked again on the current source before returning. Not an artifact
    /// cache or permission to select a different builder.
    pub fn prepare_selected_pair_ordinary_inputs(
        &mut self, owner: &wosy_daemon::SelectedPairComptimeOwner<'_>,
        private_parent: &Path, output: &Path, budget: SelectedPairToolBudget,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<SelectedPairOrdinaryInputs>> {
        use wosy_project::packages::{observe_owned_local_comptime_inputs, ComptimeExecutionAttempt, LocalComptimeInputScope};
        let Some(plan) = owner.plan() else { return Ok(None); };
        let Some(evaluators) = plan.wasip1_output() else { return Ok(None); };
        let deadline = llvm_deadline(plan.profile(), budget.timeout)?;
        let Ok(initial) = self.capture_live(plan.target(), plan.profile(), deadline, usize::MAX, true)? else { return Ok(None); };
        let source = initial.captured.source;
        if source.selection().root != plan.source_root()
            || source.selection().project != plan.evidence().site().source.project
            || initial.builder.is_none() || cancelled()
        {
            return Ok(None);
        }
        let watch_scope = || LocalComptimeInputScope {
            target: plan.target(), profile: plan.profile(), site: plan.id(), watch_limits: budget.watch_limits,
        };
        let execution = crate::comptime_fragment::execute_selected_pair_wasip1(
            evaluators, &self.project, watch_scope(), owner.configuration_inputs().0,
            ComptimeExecutionAttempt {
                seal: owner.execution_seal().clone(), root: plan.source_root().0,
                generation: source.generation(), authored_cst: plan.authored_cst().0,
            }, private_parent, budget.evaluator_ir_bytes, budget.evaluator_module_bytes, deadline.into_std(), &cancelled,
        )?;
        let Some(emitted) = owner.emit_owned_result(source.generation(), &execution)? else { return Ok(None); };
        let Ok(current) = self.capture_live(plan.target(), plan.profile(), deadline, usize::MAX, true)? else { return Ok(None); };
        if current.captured.selection() != source.selection()
            || current.captured.source.generation() != source.generation()
            || cancelled() || emitted.source_root != source.selection().root
        {
            return Ok(None);
        }
        let watches = observe_owned_local_comptime_inputs(&self.project, watch_scope(), owner.configuration_inputs().0)?;
        if watches.watches() != execution.observed().watches() || !current.captured.is_current(self)?
            || owner.plan().is_none()
        {
            return Ok(None);
        }
        require_diagnostic_time(deadline)?;
        let observed = current.builder.ok_or("selected comptime artifact has no live builder inputs")?;
        let (_, artifact, profile, _) = observed.configuration.artifact_profile(plan.target(), plan.profile())?;
        let runtime = wosy_compiler::core_runtime(&profile.backend, &profile.output)?;
        let mut partition = Vec::new();
        emitted.partition.write_text(&mut partition)?;
        let mut inputs = pending_builder_inputs(
            &observed.configuration, observed.configuration_identity, plan.target(), plan.profile(),
            artifact, profile, observed.source, blake3::hash(&partition).to_hex().to_string(), output,
        )?;
        inputs.manifest.comptime = Some(wosy_project::ArtifactComptimeObservation {
            source_root: source.selection().root.0,
            result_root: emitted.result_root.identity().0,
            configuration_inputs: owner.configuration_inputs().0,
            site: plan.id().to_owned(),
            watches: watches.watches().to_vec(),
            additional_sites: Vec::new(),
        });
        Ok(Some(SelectedPairOrdinaryInputs { source, result_root: emitted.result_root, inputs, partition, runtime }))
    }

    /// Execute all closed sites before one ordinary continuation. Observe
    /// every site before the first child and again after the last child.
    pub fn prepare_selected_ordered_ordinary_inputs(
        &mut self, owner: &wosy_daemon::SelectedOrderedComptimeOwner<'_>,
        private_parent: &Path, output: &Path, budget: SelectedPairToolBudget,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<SelectedPairOrdinaryInputs>> {
        use wosy_project::{ArtifactComptimeObservation, ArtifactComptimeSiteObservation};
        use wosy_project::packages::{observe_owned_local_comptime_watch_batch, ComptimeExecutionAttempt, LocalComptimeInputScope};
        let Some(plan) = owner.plan() else { return Ok(None); };
        let deadline = llvm_deadline(plan.profile(), budget.timeout)?;
        let Ok(initial) = self.capture_live(plan.target(), plan.profile(), deadline, usize::MAX, true)? else {
            return Ok(None);
        };
        let source = initial.captured.source;
        let configuration_inputs = owner.configuration_inputs().0;
        if source.selection().root != plan.source_root() || source.selection().project != plan.project()
            || self.project != Path::new(plan.project()) || initial.builder.is_none() || cancelled()
        {
            return Ok(None);
        }
        let scope = |ordinal: usize| LocalComptimeInputScope {
            target: plan.target(), profile: plan.profile(), site: plan.evaluators()[ordinal].id(),
            watch_limits: budget.watch_limits,
        };
        let mut remaining_ir = budget.evaluator_ir_bytes;
        for evaluator in plan.evaluators() {
            remaining_ir = remaining_ir.checked_sub(evaluator.ir().len()).ok_or("selected evaluator IR exceeds requested budget")?;
        }
        require_diagnostic_time(deadline)?;
        let sites = plan.evaluators().iter().map(|item| item.id()).collect::<Vec<_>>();
        let (declarations, watch_sets) = observe_owned_local_comptime_watch_batch(
            &self.project, plan.target(), plan.profile(), &sites, budget.watch_limits, configuration_inputs,
        )?;
        if !declarations.parameters().is_empty() {
            // This corridor admits only closed values, not parameterized tool manifests.
            return Ok(None);
        }
        let initial_watches = sites.into_iter().zip(watch_sets).map(|(site, watches)| {
            ArtifactComptimeSiteObservation { site: site.to_owned(), watches }
        }).collect::<Vec<_>>();
        let mut executions = Vec::with_capacity(plan.evaluators().len());
        for (ordinal, (evaluator, before)) in plan.evaluators().iter().zip(&initial_watches).enumerate() {
            if cancelled() || owner.plan().is_none() { return Ok(None); }
            executions.push(crate::comptime_fragment::execute_selected_closed_wasip1(
                evaluator, &self.project, scope(ordinal), configuration_inputs,
                ComptimeExecutionAttempt {
                    seal: owner.execution_seal().clone(), root: plan.source_root().0,
                    generation: source.generation(), authored_cst: plan.authored_cst().0,
                }, &before.watches, private_parent, budget.evaluator_module_bytes, deadline.into_std(), &cancelled,
            )?);
        }
        let Some(emitted) = owner.emit_owned_results(source.generation(), &executions)? else { return Ok(None); };
        let Ok(current) = self.capture_live(plan.target(), plan.profile(), deadline, usize::MAX, true)? else { return Ok(None); };
        if current.captured.selection() != source.selection()
            || current.captured.source.generation() != source.generation()
            || emitted.source_root != source.selection().root || cancelled()
        {
            return Ok(None);
        }
        let mut watches = initial_watches.into_iter();
        let first = watches.next().ok_or("selected comptime batch has no site")?;
        let observation = ArtifactComptimeObservation {
            source_root: emitted.source_root.0, result_root: emitted.result_root.identity().0, configuration_inputs,
            site: first.site, watches: first.watches, additional_sites: watches.collect(),
        };
        if !observation.matches_current(&self.project, plan.target(), plan.profile())
            || !current.captured.is_current(self)? || owner.plan().is_none()
        {
            return Ok(None);
        }
        require_diagnostic_time(deadline)?;
        let observed = current.builder.ok_or("selected comptime artifact has no live builder inputs")?;
        let (_, artifact, profile, _) = observed.configuration.artifact_profile(plan.target(), plan.profile())?;
        let runtime = wosy_compiler::core_runtime(&profile.backend, &profile.output)?;
        let mut partition = Vec::new();
        emitted.partition.write_text(&mut partition)?;
        let mut inputs = pending_builder_inputs(
            &observed.configuration, observed.configuration_identity, plan.target(), plan.profile(), artifact, profile,
            observed.source, blake3::hash(&partition).to_hex().to_string(), output,
        )?;
        inputs.manifest.identity.compiler_identity = expected_artifact_compiler_identity(Some(&observation));
        inputs.manifest.comptime = Some(observation);
        Ok(Some(SelectedPairOrdinaryInputs { source, result_root: emitted.result_root, inputs, partition, runtime }))
    }

    /// Reobserve exact ordinary inputs while the caller holds its artifact
    /// writer reservation. This does not exclude non-participating editors.
    pub fn revalidate_ordinary_build_inputs(
        &mut self, source: &AcknowledgedSourceGeneration, profile: &str,
        ordinary: &PendingBuilderInputs, timeout: Duration,
    ) -> Result<bool> {
        let deadline = llvm_deadline(profile, timeout)?;
        let selection = source.selection();
        if ordinary.manifest.comptime.as_ref().is_some_and(|item| item.source_root != selection.root.0) {
            return Ok(false);
        }
        let Ok(live) = self.capture_live(&selection.target, profile, deadline, usize::MAX, true)? else {
            return Ok(false);
        };
        Ok(live.captured.selection() == selection && live.captured.source.generation() == source.generation()
            && matches_ordinary_builder(live.builder, &selection.target, profile, ordinary)?)
    }

    /// Publish the completed ordinary builder in a new generation, without
    /// invoking a driver or moving the prior reader's artifact. The caller
    /// owns the selected local store and its ordinary builder invocation.
    pub fn publish_ordinary_build_generation(
        &mut self,
        source: &AcknowledgedSourceGeneration,
        profile: &str,
        ordinary: &PendingBuilderInputs,
        stage: wosy_project::StagedArtifactGeneration,
        timeout: Duration,
        max_authored_source_bytes: usize,
    ) -> Result<Option<PathBuf>> {
        let deadline = llvm_deadline(profile, timeout)?;
        let selection = source.selection();
        if ordinary.manifest.comptime.as_ref().is_some_and(|item| item.source_root != selection.root.0)
            || ordinary.manifest.executable != stage.output_dir().join("main")
            || !stage.is_workspace_subdirectory(&self.project)?
        {
            return Ok(None);
        }
        let prepared = stage.prepare(&ordinary.manifest)?;
        let Ok(live) = self.capture_live(&selection.target, profile, deadline, max_authored_source_bytes, true)? else {
            return Ok(None);
        };
        if live.captured.selection() != selection || live.captured.source.generation() != source.generation()
            || !matches_ordinary_builder(live.builder, &selection.target, profile, ordinary)?
        {
            return Ok(None);
        }
        let Some(_held) = self.hold_publication_selection(source)? else { return Ok(None); };
        require_diagnostic_time(deadline)?;
        Ok(Some(prepared.publish()?))
    }

    /// Local storage completion only. The caller must already own a
    /// verified controlled driver, output custody and workspace write
    /// coordination. The receipt alone cannot grant any of those.
    ///
    /// The live reobservation checks current authored bytes/configuration;
    /// the final SQLite reservation excludes a competing acknowledgement,
    /// not an editor bypassing the caller's workspace coordination.
    fn complete_controlled_generation_storage(
        &mut self,
        plan: AcknowledgedOrdinaryBuilderInputs,
        stage: wosy_project::StagedArtifactGeneration,
        timeout: std::time::Duration,
        max_authored_source_bytes: usize,
    ) -> Result<Option<PathBuf>> {
        let Some(digest) = plan.executable_partition_digest else { return Ok(None); };
        if plan.inputs.manifest.identity.output != "wasm-wasip1"
            || stage.output_dir() != plan.paths.output {
            return Ok(None);
        }
        let prepared = stage.prepare(&plan.inputs.manifest)?;
        let deadline = llvm_deadline(&plan.profile, timeout)?;
        let target = &plan.source.selection().target;
        let Ok(live) = self.capture_live(target, &plan.profile, deadline, max_authored_source_bytes, true)? else {
            return Ok(None);
        };
        if live.captured.selection() != plan.source.selection() || live.captured.generation() != plan.source.generation() {
            return Ok(None);
        }
        let observed = live.builder.ok_or("verified live capture did not retain builder inputs")?;
        let (_, artifact, profile, _) = observed.configuration.artifact_profile(target, &plan.profile)?;
        let current = pending_builder_inputs(
            &observed.configuration, observed.configuration_identity, target, &plan.profile,
            artifact, profile, observed.source, digest.hex(), &plan.paths.output,
        )?;
        if current.manifest != plan.inputs.manifest || current.builder != plan.inputs.builder {
            return Ok(None);
        }
        let Some(_held) = self.hold_publication_selection(&plan.source)? else { return Ok(None); };
        require_diagnostic_time(deadline)?;
        Ok(Some(prepared.publish()?))
    }
}

async fn open_stream<Open, Opening, Stream>(
    captured: &CapturedBuildDiagnostics, deadline: Instant,
    replay: Option<SourceReplay>, open: &mut Open,
) -> Result<Stream>
where
    Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
    Opening: Future<Output = Result<Stream>>,
    Stream: AsyncRead + AsyncWrite + Unpin + 'static,
{
    require_diagnostic_time(deadline)?;
    let mut opening = pin!(open(captured.selection().clone(), replay));
    let stream = poll_fn(|cx| match require_diagnostic_time(deadline) {
        Ok(()) => opening.as_mut().poll(cx),
        Err(expired) => Poll::Ready(Err(expired)),
    }).await?;
    require_diagnostic_time(deadline)?;
    Ok(stream)
}

/// Renderable diagnostics from one opened source-only session. The owned
/// snippets describe the authenticated revision, not later workspace edits.
/// No artifact, Build verdict, or Check result is implied.
pub struct RootedDiagnosticReport {
    response: BuildDiagnosticResponse<Diagnostic>,
    sources: HashMap<SourceIdentity, String>,
}

impl RootedDiagnosticReport {
    pub fn response(&self) -> &BuildDiagnosticResponse<Diagnostic> { &self.response }

    pub fn emit_to(&self, output: &mut impl WriteColor) -> Result<()> {
        let BuildDiagnosticResponse::BuildDiagnostics { diagnostics, .. } = &self.response;
        for diagnostic in diagnostics {
            crate::diagnostic_output::emit_project_diagnostic(output, diagnostic, |source| {
                self.sources.get(source).map(String::as_str)
            })?;
        }
        Ok(())
    }
}

impl CapturedBuildDiagnostics {
    /// Use the existing typed leased conversation and the ordinary project
    /// renderer. All labels are checked before a report can reach the writer.
    /// The caller supplies a budget for expanded authored CST occurrences
    /// and bytes; the frame timeout only bounds the asynchronous exchange.
    pub async fn exchange_project_diagnostics<T: AsyncRead + AsyncWrite + Unpin + 'static>(
        &self, local: &mut SqliteSourceState, stream: T, timeout: Duration, budget: CstExpansionBudget,
    ) -> Result<ProjectDiagnosticExchange> {
        let answer = self.exchange_unprojected(local, stream, timeout).await?;
        self.finish_project_exchange(local, answer, budget)
    }

    fn finish_project_exchange(
        &self, local: &mut SqliteSourceState, answer: DiagnosticExchange, budget: CstExpansionBudget,
    ) -> Result<ProjectDiagnosticExchange> {
        use ProjectDiagnosticExchange as Outcome;
        match answer {
            DiagnosticExchange::StaleSelection => Ok(Outcome::StaleSelection),
            DiagnosticExchange::Diagnostics(response) =>
                Ok(self.project_report(local, response, budget)?.map_or(Outcome::StaleSelection, Outcome::Report)),
            DiagnosticExchange::ReplayRequired(replay) if self.is_current(local)? => Ok(Outcome::ReplayRequired(replay)),
            DiagnosticExchange::Refused(reason) if self.is_current(local)? => Ok(Outcome::Refused(reason)),
            _ => Ok(Outcome::StaleSelection),
        }
    }

    fn project_report(
        &self, local: &mut SqliteSourceState, response: BuildDiagnosticResponse<Diagnostic>, mut budget: CstExpansionBudget,
    ) -> Result<Option<RootedDiagnosticReport>> {
        let tx = local.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let selected = self.selection();
        if !self.source.matches(read_selection(&tx, &selected.target)?) {
            return Ok(None);
        }
        let BuildDiagnosticResponse::BuildDiagnostics { diagnostics, profile, .. } = &response;
        if let Some(usage) = load_usage(&tx, selected.root)? { usage.admit(local.limits)?; }
        let mut read = StoredNodes::new(&tx)?;
        let (config, files) = SqliteSourceState::header_using(&local.project, &selected.target, selected.root,
            |id| read.charged_record(id, false, |bytes| budget.read(bytes)))?;
        if !config.profiles.contains_key(profile) { return Err("diagnostic profile is not in the selected source".into()); }
        let mut requested = BTreeMap::new();
        for label in diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels) {
            let source = &label.span.source;
            let key = FileKey {
                package: source.package.clone(),
                path: PackageRelativePath::new(source.path.clone().into())?,
            };
            if requested.insert(key, source).is_some_and(|old| old != source) {
                return Err("diagnostic labels requested conflicting source revisions".into());
            }
        }

        let mut sources = HashMap::new();
        if !requested.is_empty() {
            if files.len() > local.limits.nodes { return Err("diagnostic source file index exceeds the node budget".into()); }
            let parser = config.parser.id();
            for id in files {
                let node = read.charged_record(id, false, |bytes| budget.read(bytes))?;
                let file = FileEntry::from_node(id, node)?;
                let key = checked_file_key(&config, parser, &file)?;
                let Some(source) = requested.get(&key) else { continue; };
                if file.metadata.source.project != source.project || file.metadata.source.package != source.package
                    || file.metadata.source.path.as_path().to_str() != Some(source.path.as_str())
                    || file.metadata.revision.hex() != source.revision
                {
                    return Err("diagnostic source does not match the authored file identity".into());
                }
                if sources.contains_key(*source) { return Err("duplicate diagnostic file in source index".into()); }
                let text = materialize_file(&mut read, &mut budget, &file)?;
                sources.insert((*source).clone(), text);
            }
        }

        for label in diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels) {
            let text = sources.get(&label.span.source).ok_or("diagnostic label is outside the selected source")?;
            let span = label.span.range;
            if span.start > span.end || !text.is_char_boundary(span.start as usize) || !text.is_char_boundary(span.end as usize) {
                return Err("diagnostic label is outside authored UTF-8 boundaries".into());
            }
        }
        drop(read);
        tx.commit()?;
        Ok(Some(RootedDiagnosticReport { response, sources }))
    }
}

fn materialize_file(read: &mut StoredNodes<'_>, budget: &mut CstExpansionBudget, file: &FileEntry) -> Result<String> {
    let bytes = file.metadata.bytes as usize;
    budget.file(bytes)?;
    let mut state = (read, budget, String::with_capacity(bytes));
    walk_cst(file.authored, &mut state, |(read, budget, _), id| {
        read.charged_record(id, true, |bytes| budget.read(bytes))
    }, |(_, budget, text), event| {
        match event {
            CstEvent::Start { depth, children, .. } => budget.children(depth, children)?,
            CstEvent::Token { text: token, .. } => {
                if token.len() > bytes - text.len() { return Err("diagnostic file exceeds its authored extent".into()); }
                text.push_str(token);
            }
            CstEvent::Finish => {}
        }
        Ok(())
    })?;
    let text = state.2;
    if text.len() != bytes || blake3::hash(text.as_bytes()).as_bytes() != &file.metadata.revision.0 {
        return Err("diagnostic text does not match its authored revision".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_recovery::ClientSourceLimits;
    use wosy_compiler::{DiagnosticLabel, DiagnosticLabelKind, DiagnosticSeverity};
    use wosy_protocol::{BuildAnalysisOrigin, BuildAnalysisPhase, NodeId, BUILD_DIAGNOSTIC_VERSION};
    use wosy_syntax::{ByteSpan, SourceSpan};

    #[test]
    #[ignore = "requires a disposable SQLite project with a selected app root and a UTF-8 source comment"]
    fn verify_report_admission_before_a_writer_is_available() {
        let project = std::path::PathBuf::from(std::env::var_os("WOSY_REPORT_PROJECT").unwrap());
        let mut local = SqliteSourceState::open(&project, ClientSourceLimits {
            nodes: 25_000, encoded_bytes: 16 << 20,
        }).unwrap();
        let capture = local.capture_build_diagnostics("app", "dev").unwrap().unwrap();
        {
            let mut records = StoredNodes::new(&local.db).unwrap();
            let root = capture.selection().root;
            let mut limited = CstExpansionBudget::new(0, 1, 1, 0);
            assert!(records.charged_record(root, true, |bytes| limited.read(bytes)).is_err());
            assert!(records.checked.nodes.is_empty());
            assert_eq!(records.count, 1);
            let mut enough = CstExpansionBudget::new(1024, 1, 1, 0);
            records.charged_record(root, true, |bytes| enough.read(bytes)).unwrap();
            assert!(records.checked.nodes.contains_key(&root));
            assert_eq!(records.count, 2);
            assert!(records.charged_record(root, true, |bytes| limited.read(bytes)).is_err());
            assert_eq!(records.count, 2);
        }
        let text = std::fs::read_to_string(project.join("src/main.w")).unwrap();
        let unicode = text.find('☕').unwrap() as u32;
        let source = SourceIdentity::new(capture.selection().project.clone(), "app".into(), "src/main.w".into(),
            NodeId(*blake3::hash(text.as_bytes()).as_bytes()).hex());
        let report = |source: SourceIdentity, range: ByteSpan| BuildDiagnosticResponse::BuildDiagnostics {
            version: BUILD_DIAGNOSTIC_VERSION, source_root: capture.selection().root,
            profile: "dev".into(), preparation: NodeId([12; 32]), receipt: NodeId([13; 32]),
            phase: BuildAnalysisPhase::Analyzed, origin: BuildAnalysisOrigin::Derived,
            diagnostics: vec![Diagnostic {
                code: "fixture".into(), severity: DiagnosticSeverity::Error, message: "fixture".into(),
                notes: vec![], labels: vec![DiagnosticLabel { kind: DiagnosticLabelKind::Primary,
                    span: SourceSpan::new(source, range), message: "exact authored span".into(),
                }],
            }],
        };
        let budget = || CstExpansionBudget::new(16 << 20, 20_000, 1024, 8 << 20);
        let range = ByteSpan::new(unicode, unicode + 3);
        assert!(capture.project_report(&mut local, report(source.clone(), range),
            CstExpansionBudget::new(0, 20_000, 1024, 8 << 20)).is_err());
        assert!(capture.project_report(&mut local, report(source.clone(), range), budget()).unwrap().is_some());
        let mut wrong = source.clone();
        wrong.revision = NodeId([99; 32]).hex();
        assert!(capture.project_report(&mut local, report(wrong, range), budget()).is_err());
        let mut wrong = source.clone();
        wrong.path = "src/ordinary-but-unseen.w".into();
        assert!(capture.project_report(&mut local, report(wrong, range), budget()).is_err());
        let mut wrong = source.clone();
        wrong.project.push_str("-different");
        assert!(capture.project_report(&mut local, report(wrong, range), budget()).is_err());
        assert!(capture.project_report(&mut local, report(source, ByteSpan::new(unicode + 1, unicode + 3)),
            budget()).is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod controlled_generation_storage_tests {
    use super::*;
    use crate::source_recovery::ClientSourceLimits;
    use std::{fs, os::unix::fs::DirBuilderExt, path::Path, time::{Duration, SystemTime, UNIX_EPOCH}};
    use wosy_project::{SelectedArtifactGeneration, StagedArtifactGeneration};

    const TIME: Duration = Duration::from_secs(5);
    const BYTES: usize = 1 << 20;
    const CONFIG: &str = r#"
[package]
name = "app"
default_target = "app"
[comments]
categories = []
[builders.fixture]
command = ["must-not-invoke"]
toolchain_identity = "fixture-only"
[runners.fixture]
command = ["must-not-run"]
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "Main"
[targets.app.artifacts.main.profiles.wasm]
backend = "llvm"
output = "wasm-wasip1"
builder = "fixture"
run_runner = "fixture"
[targets.app.profiles.wasm]
main_artifact = "main"
"#;

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    fn make_plan(
        source: &mut SqliteSourceState, store: &Path,
    ) -> (AcknowledgedOrdinaryBuilderInputs, StagedArtifactGeneration) {
        let stage = StagedArtifactGeneration::create(store).unwrap();
        let paths = BuilderInputPaths::new(stage.output_dir());
        let LiveCapture { captured, builder } = source.capture_live("app", "wasm",
            llvm_deadline("wasm", TIME).unwrap(), BYTES, true).unwrap().ok().unwrap();
        let observed = builder.unwrap();
        let (_, artifact, profile, _) = observed.configuration.artifact_profile("app", "wasm").unwrap();
        // This creates only a disposable storage fixture, not a native plan
        // authorization or a fixture driver that the product may execute.
        let digest = NodeId([1; 32]);
        let inputs = pending_builder_inputs(&observed.configuration, observed.configuration_identity, "app", "wasm",
            artifact, profile, observed.source, digest.hex(), &paths.output).unwrap();
        fs::write(&inputs.manifest.executable, b"storage-only fixture bytes").unwrap();
        (AcknowledgedOrdinaryBuilderInputs {
            source: captured.source, project: observed.project, paths, inputs,
            partition: String::new(), runtime: "", executable_partition_digest: Some(digest), profile: "wasm".into(),
            #[cfg(feature = "native-diagnostics")]
            origin: BuilderPlanOrigin::LocalAdapter,
        }, stage)
    }

    #[cfg(feature = "native-diagnostics")]
    #[test]
    fn local_adapter_plan_cannot_prepare_a_native_invocation() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let scratch = Scratch(std::env::temp_dir().join(format!("wosy-local-plan-not-native-{}-{now}", std::process::id())));
        fs::DirBuilder::new().mode(0o700).create(&scratch.0).unwrap();
        fs::write(scratch.0.join("wosy.toml"), CONFIG).unwrap();
        fs::write(scratch.0.join("main.w"), "%%start\ni32 value = 42;\n%%end\n").unwrap();
        let mut source = SqliteSourceState::open(&scratch.0, ClientSourceLimits { nodes: 1000, encoded_bytes: BYTES }).unwrap();
        let staged = source.stage("app").unwrap();
        assert!(source.acknowledge("app", staged.pending.unwrap()).unwrap());
        let store = scratch.0.join("controlled-fixture-store");
        fs::DirBuilder::new().mode(0o700).create(&store).unwrap();
        let (plan, stage) = make_plan(&mut source, &store);
        assert!(stage.is_workspace_subdirectory(&scratch.0).unwrap());
        let candidate = stage.output_dir();
        assert!(PreparedNativeBuildInvocation::prepare(plan, stage).unwrap().is_none());
        assert!(!candidate.exists());
        assert!(!store.join("artifact-manifest.json").exists());
    }

    #[test]
    fn only_current_observed_inputs_commit_and_old_reader_is_retained() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let scratch = Scratch(std::env::temp_dir().join(format!("wosy-complete-generation-{}-{now}", std::process::id())));
        fs::DirBuilder::new().mode(0o700).create(&scratch.0).unwrap();
        let store = scratch.0.join("controlled-fixture-store");
        fs::DirBuilder::new().mode(0o700).create(&store).unwrap();
        let config = scratch.0.join("wosy.toml");
        fs::write(&config, CONFIG).unwrap();
        let entry = scratch.0.join("main.w");
        let first = "%%start\ni32 value = 42;\n%%end\n";
        let changed = "%%start\ni32 value = 43;\n%%end\n";
        fs::write(&entry, first).unwrap();
        let mut source = SqliteSourceState::open(&scratch.0, ClientSourceLimits { nodes: 1000, encoded_bytes: BYTES }).unwrap();
        let staged = source.stage("app").unwrap();
        assert!(source.acknowledge("app", staged.pending.unwrap()).unwrap());
        let (plan, candidate) = make_plan(&mut source, &store);
        let prior = source.complete_controlled_generation_storage(plan, candidate, TIME, BYTES).unwrap().unwrap();
        let reader = SelectedArtifactGeneration::open(&store).unwrap().unwrap();
        assert_eq!(reader.manifest_path(), prior);

        let (plan, candidate) = make_plan(&mut source, &store);
        let refused = candidate.output_dir();
        fs::write(&entry, changed).unwrap();
        assert!(source.complete_controlled_generation_storage(plan, candidate, TIME, BYTES).unwrap().is_none());
        assert!(!refused.exists());
        assert_eq!(SelectedArtifactGeneration::open(&store).unwrap().unwrap().manifest_path(), prior);

        fs::write(&entry, first).unwrap();
        let (plan, candidate) = make_plan(&mut source, &store);
        fs::write(&config, CONFIG.replace("fixture-only", "changed-toolchain")).unwrap();
        assert!(source.complete_controlled_generation_storage(plan, candidate, TIME, BYTES).unwrap().is_none());
        assert_eq!(SelectedArtifactGeneration::open(&store).unwrap().unwrap().manifest_path(), prior);

        fs::write(&config, CONFIG).unwrap();
        let (plan, candidate) = make_plan(&mut source, &store);
        let next = source.complete_controlled_generation_storage(plan, candidate, TIME, BYTES).unwrap().unwrap();
        assert_ne!(prior, next);
        assert_eq!(SelectedArtifactGeneration::open(&store).unwrap().unwrap().manifest_path(), next);
        assert_eq!(reader.manifest_path(), prior);
        assert_eq!(fs::read(&reader.manifest().executable).unwrap(), b"storage-only fixture bytes");
    }
}
