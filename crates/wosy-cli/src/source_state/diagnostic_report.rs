//! Render a verified diagnostic response against its immutable local source.
use super::diagnostics::{diagnostic_deadline, require_diagnostic_time, within_diagnostic_deadline, CommandExchange};
use super::{
    checked_file_key, load_usage, read_selection, AcknowledgedSourceGeneration, CapturedBuildDiagnostics, DiagnosticExchange,
    RequiredSourceReplay, Result, SourceReplay, SqliteSourceState, StoredNodes,
};
use crate::{
    artifact_inputs::{pending_builder_inputs, BuilderInputPaths, ObservedBuilderBasis, PendingBuilderInputs},
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
        partition: projection.partition, runtime, executable_partition_digest })
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
