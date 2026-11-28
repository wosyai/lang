//! Route one bounded byte stream through the existing diagnostic session.
use super::{
    build_session::BuildRequestRejection, encode_session_frame, BaseCstAdmissionLimits,
    BuildDiagnosticSession, BuildSourceOpening, PreparedBaseDeclarations, Result, RootedLlvmProjection,
    SessionFrameError, SessionFrameReader,
    SourceStorageWorker, StorageAdmissionError,
};
use std::time::{Duration, Instant};
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio::time::{sleep_until, Instant as TokioInstant};
use wosy_compiler::Diagnostic;
use wosy_protocol::{
    BaseCstProposal, BaseCstProposalBinding, BaseDeclarationProjection, BuildDiagnosticRequest,
    BuildDiagnosticResponse, BuildSessionCommand as Command, BuildSessionCommandId as CommandId,
    BuildSessionRefusal as Refusal, BuildSessionReply, BuildSessionRequest as Request,
    write_encoded_session_frame, BuildSessionResponse, BuildSourceSelection, Delta, BASE_DECLARATION_PROJECTION_VERSION,
    OrdinaryLlvmBuilderWitness, OrdinaryLlvmProjection, BUILD_DIAGNOSTIC_VERSION, BUILD_RECONCILIATION_VERSION,
    MAX_ORDINARY_LLVM_PROFILE_BYTES, MAX_SESSION_FRAME_BYTES, ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION, ORDINARY_LLVM_BUILDER_INPUT_VERSION, ORDINARY_LLVM_PROJECTION_VERSION,
};

type Reply = BuildSessionReply<Diagnostic>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildSessionEnd {
    PeerClosed,
    Disconnected,
    Expired,
    Refused,
    ReplayRequired,
}

enum Close {
    Peer,
    Deadline,
    Request(CommandId),
}

struct ProjectionScope {
    binding: BaseCstProposalBinding,
    admission_limits: BaseCstAdmissionLimits,
    declaration_bytes: usize,
}

#[derive(Clone)]
struct OrdinaryLlvmScope {
    limits: BaseCstAdmissionLimits,
    bytes: usize,
    version: u16,
    pinned_profile: Option<String>,
}

enum Opening {
    Ordinary(Option<ProjectionScope>),
    Recoverable { replay: Option<Delta>, limits: BaseCstAdmissionLimits, ordinary_llvm: Option<OrdinaryLlvmScope> },
}

enum Opened<'a> {
    Session(BuildDiagnosticSession<'a>),
    Unavailable,
    ReplayRequired,
}

enum Operation<'a> {
    Analyze(BuildDiagnosticRequest),
    Project(&'a ProjectionScope, BaseCstProposal),
    OrdinaryLlvm(OrdinaryLlvmScope, u16, String),
}

enum Completed {
    Analysis(BuildDiagnosticResponse<Diagnostic>),
    Projection(PreparedBaseDeclarations),
    OrdinaryLlvm(u16, RootedLlvmProjection, Option<OrdinaryLlvmBuilderWitness>),
}

impl SourceStorageWorker {
    /// Serve one host-selected connection on a Tokio runtime with its time
    /// driver enabled. The adapter supplies the acknowledged selection;
    /// wire commands never change roots. This does not provide a listener.
    pub async fn serve_build_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        self.serve_selected_session(stream, selected, lifetime, Opening::Ordinary(None), None).await
    }

    /// Only the trusted local adapter may bind a pending source attempt to
    /// this connection. The same lease still selects diagnostics from the
    /// published predecessor. This is not a Unix listener or publication.
    pub async fn serve_base_declaration_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, binding: BaseCstProposalBinding, admission_limits: BaseCstAdmissionLimits,
        declaration_bytes: usize, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        let selected = binding.selected.clone();
        let scope = ProjectionScope { binding, admission_limits, declaration_bytes };
        self.serve_selected_session(stream, selected, lifetime, Opening::Ordinary(Some(scope)), None).await
    }

    /// Opt-in diagnostic handshake for the trusted adapter's acknowledged
    /// source. When replay is required, send one terminal reply; a new
    /// connection is necessary. The adapter supplies authorized replay out
    /// of band, not as wire input or a previously issued lease token.
    /// The adapter must discard the stream when this call returns, including
    /// when passing a borrowed transport.
    pub async fn serve_recoverable_build_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, replay: Option<Delta>,
        limits: BaseCstAdmissionLimits, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        self.serve_selected_session(stream, selected, lifetime, Opening::Recoverable { replay, limits, ordinary_llvm: None }, None).await
    }

    /// Explicit developer/local adapter capability on the same existing
    /// recoverable session. The adapter selects the source and all work
    /// limits out of band. NativeKnown and the ordinary diagnostic sessions
    /// do not get this capability. Does not supply any listener or builder.
    pub async fn serve_recoverable_ordinary_llvm_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, replay: Option<Delta>,
        limits: BaseCstAdmissionLimits, max_partition_bytes: usize, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        let ordinary_llvm = Some(OrdinaryLlvmScope {
            limits, bytes: max_partition_bytes.min(MAX_SESSION_FRAME_BYTES), version: ORDINARY_LLVM_PROJECTION_VERSION,
            pinned_profile: None,
        });
        self.serve_selected_session(stream, selected, lifetime,
            Opening::Recoverable { replay, limits, ordinary_llvm }, None).await
    }

    /// Compare ordinary authored LLVM for one exact profile nominated with
    /// the source by a trusted, local activator that owns both stream ends.
    /// A replay-required exchange must close; only a fresh connection can
    /// receive the authorized client-side replay for the same selection.
    pub async fn serve_scoped_ordinary_llvm_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, replay: Option<Delta>, profile: String,
        limits: BaseCstAdmissionLimits, max_partition_bytes: usize, cutoff: Instant,
    ) -> Result<BuildSessionEnd> {
        if profile.is_empty() || profile.len() > MAX_ORDINARY_LLVM_PROFILE_BYTES {
            return Err("invalid selected ordinary LLVM profile".into());
        }
        let lifetime = cutoff.saturating_duration_since(Instant::now());
        let ordinary_llvm = Some(OrdinaryLlvmScope {
            limits, bytes: max_partition_bytes.min(MAX_SESSION_FRAME_BYTES),
            version: ORDINARY_LLVM_PROJECTION_VERSION, pinned_profile: Some(profile),
        });
        self.serve_selected_session(stream, selected, lifetime,
            Opening::Recoverable { replay, limits, ordinary_llvm }, Some(cutoff)).await
    }

    /// The trusted local activator owns and binds this stream, selects a
    /// known source out of band, and never supplies replay for this path.
    /// The v2 builder witness is not an OS peer authentication mechanism.
    pub async fn serve_known_ordinary_builder_input_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection,
        limits: BaseCstAdmissionLimits, max_partition_bytes: usize, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        self.serve_known_builder_input_session(stream, selected, limits, max_partition_bytes, lifetime,
            ORDINARY_LLVM_BUILDER_INPUT_VERSION).await
    }

    /// Explicitly scoped experimental executable projection. Never accepts replay.
    pub async fn serve_known_executable_builder_input_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection,
        limits: BaseCstAdmissionLimits, max_partition_bytes: usize, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        self.serve_known_builder_input_session(stream, selected, limits, max_partition_bytes, lifetime,
            ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION).await
    }

    async fn serve_known_builder_input_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection,
        limits: BaseCstAdmissionLimits, max_partition_bytes: usize, lifetime: Duration, version: u16,
    ) -> Result<BuildSessionEnd> {
        let ordinary_llvm = Some(OrdinaryLlvmScope {
            limits, bytes: max_partition_bytes.min(MAX_SESSION_FRAME_BYTES), version, pinned_profile: None,
        });
        self.serve_selected_session(stream, selected, lifetime,
            Opening::Recoverable { replay: None, limits, ordinary_llvm }, None).await
    }

    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    pub(super) async fn serve_native_executable_inputs_before<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, profile: String,
        limits: BaseCstAdmissionLimits, max_partition_bytes: usize, lifetime: Duration, cutoff: Instant,
    ) -> Result<BuildSessionEnd> {
        let ordinary_llvm = Some(OrdinaryLlvmScope {
            limits, bytes: max_partition_bytes.min(MAX_SESSION_FRAME_BYTES),
            version: ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION, pinned_profile: Some(profile),
        });
        self.serve_selected_session(stream, selected, lifetime,
            Opening::Recoverable { replay: None, limits, ordinary_llvm }, Some(cutoff)).await
    }

    /// Only the NativeKnown host passes a non-renewable authority cut.
    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    pub(super) async fn serve_known_build_session_before<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, lifetime: Duration, cutoff: Instant,
    ) -> Result<BuildSessionEnd> {
        const NO_REPLAY: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
            records: 0, record_bytes: 0, graph_edges: 0, expanded_record_bytes: 0,
            cst_occurrences: 0, cst_depth: 0, authored_bytes: 0,
        };
        self.serve_selected_session(stream, selected, lifetime,
            Opening::Recoverable { replay: None, limits: NO_REPLAY, ordinary_llvm: None }, Some(cutoff)).await
    }

    async fn serve_selected_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, lifetime: Duration,
        opening: Opening, hard_deadline: Option<Instant>,
    ) -> Result<BuildSessionEnd> {
        let start_deadline = match hard_deadline {
            Some(cutoff) => cutoff,
            None => Instant::now().checked_add(lifetime).ok_or("Build session start timeout is out of range")?,
        };
        if cutoff_reached(hard_deadline) { return Ok(BuildSessionEnd::Expired); }
        let (mut input, mut output) = io::split(stream);
        let mut frames = SessionFrameReader::default();
        let start = tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(start_deadline)) => return Ok(BuildSessionEnd::Expired),
            event = frames.read::<_, Command>(&mut input) => event,
        };
        if cutoff_reached(hard_deadline) { return Ok(BuildSessionEnd::Expired); }
        let Some(Command { id, request }) = start? else { return Ok(BuildSessionEnd::PeerClosed); };
        let recoverable = matches!(opening, Opening::Recoverable { .. });
        let refused = match (recoverable, request) {
            (false, Request::Start { version }) if version == BUILD_DIAGNOSTIC_VERSION => None,
            (true, Request::StartRecoverable { version }) if version == BUILD_RECONCILIATION_VERSION => None,
            (false, Request::Start { .. }) | (true, Request::StartRecoverable { .. }) => Some(Refusal::UnsupportedVersion),
            _ => Some(Refusal::ExpectedStart),
        };
        if let Some(reason) = refused {
            return refuse_start(&mut output, start_deadline, hard_deadline, id, reason).await;
        }
        if Instant::now() >= start_deadline { return Ok(BuildSessionEnd::Expired); }
        let source_root = selected.root;
        let (projection, recovery, ordinary_llvm) = match opening {
            Opening::Ordinary(projection) => (projection, None, None),
            Opening::Recoverable { replay, limits, ordinary_llvm } => (None, Some((replay, limits)), ordinary_llvm),
        };
        let opened: Result<Opened<'_>> = tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(start_deadline)) => return Ok(BuildSessionEnd::Expired),
            opened = async {
                if let Some((replay, limits)) = recovery {
                    Ok(match self.open_recoverable_build_diagnostics_before(selected, replay, limits, lifetime, Some(start_deadline), hard_deadline).await? {
                        BuildSourceOpening::Opened { session, .. } => Opened::Session(session),
                        BuildSourceOpening::ReplayRequired { .. } => Opened::ReplayRequired,
                    })
                } else {
                    Ok(self.open_build_diagnostics_before(selected, lifetime, Some(start_deadline)).await?
                        .map_or(Opened::Unavailable, Opened::Session))
                }
            } => opened,
        };
        if cutoff_reached(hard_deadline) { return Ok(BuildSessionEnd::Expired); }
        let session = match opened {
            Ok(Opened::Session(session)) => session,
            Ok(Opened::Unavailable) => return refuse_start(&mut output, start_deadline, hard_deadline, id, Refusal::SourceUnavailable).await,
            Ok(Opened::ReplayRequired) => {
                let reply = Reply::ReplayRequired { version: BUILD_RECONCILIATION_VERSION, source_root };
                return Ok(if send(&mut output, start_deadline, hard_deadline, id, reply).await? {
                    BuildSessionEnd::ReplayRequired
                } else { BuildSessionEnd::Expired });
            }
            Err(error) => {
                if matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)) {
                    return Ok(BuildSessionEnd::Expired);
                }
                let reason = refusal(&error, Refusal::SourceUnavailable);
                return refuse_start(&mut output, start_deadline, hard_deadline, id, reason).await;
            }
        };
        let Some(deadline) = session.deadline() else { return Ok(BuildSessionEnd::Expired); };
        let started = if recoverable {
            Reply::RecoverableStarted { version: BUILD_RECONCILIATION_VERSION, source_root }
        } else if projection.is_some() {
            Reply::BaseDeclarationSessionStarted {
                version: BUILD_DIAGNOSTIC_VERSION, source_root, connection: session.connection_id(),
            }
        } else {
            Reply::Started { version: BUILD_DIAGNOSTIC_VERSION, source_root }
        };
        if !send(&mut output, deadline.min(start_deadline), hard_deadline, id, started).await? { return Ok(BuildSessionEnd::Expired); }
        let end = run(&session, &mut frames, &mut input, &mut output, projection.as_ref(), ordinary_llvm).await;
        let deadline = session.deadline();
        session.release();
        if cutoff_reached(hard_deadline) { return Ok(BuildSessionEnd::Expired); }
        Ok(match end? {
            Close::Request(id) => {
                if let Some(deadline) = deadline {
                    let written = send(&mut output, deadline, hard_deadline, id, Reply::Closed).await?;
                    if !written && hard_deadline.is_some() { return Ok(BuildSessionEnd::Expired); }
                } else if hard_deadline.is_some() {
                    return Ok(BuildSessionEnd::Expired);
                }
                BuildSessionEnd::Disconnected
            }
            Close::Peer => BuildSessionEnd::PeerClosed,
            Close::Deadline => BuildSessionEnd::Expired,
        })
    }
}

async fn run<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    session: &BuildDiagnosticSession<'_>, frames: &mut SessionFrameReader, input: &mut R,
    output: &mut W, projection: Option<&ProjectionScope>, mut ordinary_llvm: Option<OrdinaryLlvmScope>,
) -> Result<Close> {
    let mut owner = None;
    let cutoff = session.hard_deadline();
    let selected_projection_only = ordinary_llvm.as_ref().is_some_and(|scope| scope.pinned_profile.is_some());
    loop {
        if cutoff_reached(cutoff) { return Ok(Close::Deadline); }
        let Some(deadline) = session.deadline() else { return Ok(Close::Deadline); };
        let request = tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(deadline)) => return Ok(Close::Deadline),
            request = frames.read::<_, Command>(input) => request,
        };
        if cutoff_reached(cutoff) { return Ok(Close::Deadline); }
        let (id, operation) = match request? {
            Some(Command { id, request: Request::Analyze { .. } }) if selected_projection_only => {
                if !send(output, deadline, cutoff, id, Reply::Refused { reason: Refusal::AnalysisUnavailable }).await? {
                    return Ok(Close::Deadline);
                }
                continue;
            }
            Some(Command { id, request: Request::Analyze { request } }) => (id, Operation::Analyze(request)),
            Some(Command { id, request: Request::ProjectBaseDeclarations { version, proposal } }) => {
                let refusal = if version != BASE_DECLARATION_PROJECTION_VERSION {
                    Refusal::UnsupportedVersion
                } else if owner.is_some() {
                    Refusal::BaseProjectionAlreadyHeld
                } else if let Some(scope) = projection {
                    if let Some(end) = drive(session, id, Operation::Project(scope, proposal),
                        frames, input, output, &mut owner).await?
                    {
                        return Ok(end);
                    }
                    continue;
                } else {
                    Refusal::BaseProjectionUnavailable
                };
                if !send(output, deadline, cutoff, id, Reply::Refused { reason: refusal }).await? {
                    return Ok(Close::Deadline);
                }
                continue;
            }
            Some(Command { id, request: Request::ProjectOrdinaryLlvm { version, profile } }) => {
                let supported = matches!(version, ORDINARY_LLVM_PROJECTION_VERSION | ORDINARY_LLVM_BUILDER_INPUT_VERSION | ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION);
                let scope = ordinary_llvm.as_ref().filter(|scope| scope.version == version
                    && profile.len() <= MAX_ORDINARY_LLVM_PROFILE_BYTES
                    && scope.pinned_profile.as_ref().is_none_or(|selected| selected == &profile)).cloned();
                if let Some(scope) = scope {
                    if selected_projection_only { ordinary_llvm = None; }
                    (id, Operation::OrdinaryLlvm(scope, version, profile))
                } else {
                    let reason = if !supported { Refusal::UnsupportedVersion }
                        else { Refusal::OrdinaryLlvmUnavailable };
                    if !send(output, deadline, cutoff, id, Reply::Refused { reason }).await? {
                        return Ok(Close::Deadline);
                    }
                    continue;
                }
            }
            request => {
                if let Some(end) = control(session, request, output).await? { return Ok(end); }
                continue;
            }
        };
        if let Some(end) = drive(session, id, operation, frames, input, output, &mut owner).await? {
            return Ok(end);
        }
    }
}

async fn drive<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    session: &BuildDiagnosticSession<'_>, id: CommandId, operation: Operation<'_>,
    frames: &mut SessionFrameReader, input: &mut R, output: &mut W,
    owner: &mut Option<PreparedBaseDeclarations>,
) -> Result<Option<Close>> {
    let failure = match &operation {
        Operation::Analyze(_) => Refusal::AnalysisUnavailable,
        Operation::Project(..) => Refusal::BaseProjectionUnavailable,
        Operation::OrdinaryLlvm(..) => Refusal::OrdinaryLlvmUnavailable,
    };
    let job = async {
        match operation {
            Operation::Analyze(request) => session.dispatch_framed(request).await.map(Completed::Analysis),
            Operation::Project(scope, proposal) => session.project_base_declarations(
                scope.binding.clone(), proposal, scope.admission_limits, scope.declaration_bytes,
            ).await.map(Completed::Projection),
            Operation::OrdinaryLlvm(scope, version, profile) => session.project_ordinary_llvm_framed(
                profile, scope.limits, scope.bytes, version != ORDINARY_LLVM_PROJECTION_VERSION,
                version == ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION,
            ).await.map(|(projection, witness)| Completed::OrdinaryLlvm(version, projection, witness)),
        }
    };
    tokio::pin!(job);
    let cutoff = session.hard_deadline();
    loop {
        if cutoff_reached(cutoff) { return Ok(Some(Close::Deadline)); }
        let Some(deadline) = session.deadline() else { return Ok(Some(Close::Deadline)); };
        tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(deadline)) => return Ok(Some(Close::Deadline)),
            response = &mut job => {
                if cutoff_reached(cutoff) { return Ok(Some(Close::Deadline)); }
                let reply = match response {
                    Ok(Completed::Analysis(response)) => Reply::Analysis { response },
                    Ok(Completed::Projection(prepared)) => {
                        let scope = prepared.admission();
                        let projection = BaseDeclarationProjection {
                            version: BASE_DECLARATION_PROJECTION_VERSION, connection: session.connection_id(),
                            base: scope.base, proposed: scope.proposed, attempt: scope.attempt, preparation: prepared.identity().digest(),
                            entry_projection: prepared.entry_projection(),
                        };
                        *owner = Some(prepared);
                        Reply::BaseDeclarationProjection { projection }
                    }
                    Ok(Completed::OrdinaryLlvm(version, RootedLlvmProjection::Emitted(partition), builder)) =>
                        Reply::OrdinaryLlvmProjection { projection: OrdinaryLlvmProjection {
                            version, builder,
                            source_root: partition.source_root, profile: partition.profile,
                            compiler_image: partition.compiler_image, digest: partition.digest,
                            partition: String::from_utf8(partition.bytes)?,
                        }},
                    Ok(Completed::OrdinaryLlvm(..)) => Reply::Refused { reason: Refusal::OrdinaryLlvmUnavailable },
                    Err(error) => {
                        if matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)) {
                            return Ok(Some(Close::Deadline));
                        }
                        Reply::Refused { reason: refusal(&error, failure) }
                    }
                };
                let written = match send(output, deadline, cutoff, id, reply).await {
                    Err(SessionFrameError::TooLarge) =>
                        send(output, deadline, cutoff, id, Reply::Refused { reason: Refusal::ResponseTooLarge }).await?,
                    result => result?,
                };
                return Ok((!written).then_some(Close::Deadline));
            }
            request = frames.read::<_, Command>(input) => {
                if cutoff_reached(cutoff) { return Ok(Some(Close::Deadline)); }
                let request = request?;
                if request.as_ref().is_some_and(|command| command.id == id) {
                    return Err("framed Build command ID is already outstanding".into());
                }
                if let Some(end) = control(session, request, output).await? { return Ok(Some(end)); }
            }
        }
    }
}

async fn control<W: AsyncWrite + Unpin>(
    session: &BuildDiagnosticSession<'_>, command: Option<Command>, output: &mut W,
) -> Result<Option<Close>> {
    let cutoff = session.hard_deadline();
    if cutoff_reached(cutoff) { return Ok(Some(Close::Deadline)); }
    let Some(Command { id, request }) = command else { return Ok(Some(Close::Peer)); };
    let reply = match request {
        Request::Disconnect => return Ok(Some(Close::Request(id))),
        Request::Start { .. } | Request::StartRecoverable { .. } => Reply::Refused { reason: Refusal::AlreadyStarted },
        Request::Analyze { .. } | Request::ProjectBaseDeclarations { .. } | Request::ProjectOrdinaryLlvm { .. } =>
            Reply::Refused { reason: Refusal::Busy },
        Request::Renew => {
            if !session.renew() { return Ok(Some(Close::Deadline)); }
            Reply::Renewed
        }
    };
    let Some(deadline) = session.deadline() else { return Ok(Some(Close::Deadline)); };
    Ok((!send(output, deadline, cutoff, id, reply).await?).then_some(Close::Deadline))
}

async fn refuse_start<W: AsyncWrite + Unpin>(
    output: &mut W, until: Instant, cutoff: Option<Instant>, id: CommandId, reason: Refusal,
) -> Result<BuildSessionEnd> {
    Ok(if send(output, until, cutoff, id, Reply::Refused { reason }).await? {
        BuildSessionEnd::Refused
    } else { BuildSessionEnd::Expired })
}

fn cutoff_reached(cutoff: Option<Instant>) -> bool {
    cutoff.is_some_and(|time| Instant::now() >= time)
}

fn refusal(error: &super::Error, otherwise: Refusal) -> Refusal {
    if matches!(error.downcast_ref::<StorageAdmissionError>(), Some(StorageAdmissionError::Busy)) {
        Refusal::Busy
    } else if matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Version)) {
        Refusal::UnsupportedVersion
    } else {
        otherwise
    }
}

async fn send<W: AsyncWrite + Unpin>(
    output: &mut W, until: Instant, cutoff: Option<Instant>, id: CommandId, reply: Reply,
) -> std::result::Result<bool, SessionFrameError> {
    // The ordinary route has no hard cutoff. On NativeKnown, check before
    // potentially expensive encode; the writer checks the same absolute
    // bound again before any first byte, including after a pending poll.
    if cutoff_reached(cutoff) { return Ok(false); }
    let until = cutoff.map_or(until, |strict| until.min(strict));
    let bytes = encode_session_frame(&BuildSessionResponse { id, reply });
    if cutoff_reached(cutoff) { return Ok(false); }
    let bytes = bytes?;
    tokio::select! {
        biased;
        _ = sleep_until(TokioInstant::from_std(until)) => Ok(false),
        result = write_encoded_session_frame(output, &bytes, || Instant::now() < until) => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceStore, StorageAdmissionError};
    use tokio::io::AsyncWriteExt;
    use std::num::NonZeroUsize;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc};
    use tokio::runtime::{Builder, Handle};
    use wosy_protocol::{BuildAnalysisOrigin, BuildDiagnosticResponse, NodeId};

    async fn send_request(stream: &mut io::DuplexStream, id: u64, request: Request) {
        let command = Command { id: CommandId(id), request };
        stream.write_all(&encode_session_frame(&command).unwrap()).await.unwrap();
    }

    async fn receive(stream: &mut io::DuplexStream, frames: &mut SessionFrameReader, id: u64) -> Reply {
        let message: BuildSessionResponse<Diagnostic> = frames.read(stream).await.unwrap().unwrap();
        assert_eq!(message.id, CommandId(id));
        message.reply
    }

    async fn start(stream: &mut io::DuplexStream, frames: &mut SessionFrameReader, root: NodeId) {
        send_request(stream, 1, Request::Start { version: BUILD_DIAGNOSTIC_VERSION }).await;
        let reply = receive(stream, frames, 1).await;
        assert!(matches!(reply, Reply::Started { version: BUILD_DIAGNOSTIC_VERSION, source_root } if source_root == root));
    }

    fn analyze_request() -> Request {
        Request::Analyze { request: BuildDiagnosticRequest::BuildDiagnostics {
            version: BUILD_DIAGNOSTIC_VERSION, profile: "dev".into(),
        }}
    }

    #[test]
    fn expired_response_does_not_start_writing() {
        use tokio::io::AsyncReadExt;
        Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
            let (mut server, mut client) = io::duplex(1);
            assert!(!send(&mut server, Instant::now(), None, CommandId(1), Reply::Renewed).await.unwrap());
            assert!(!send(&mut server, Instant::now() + Duration::from_secs(30), Some(Instant::now()),
                CommandId(3), Reply::Renewed).await.unwrap());
            server.write_all(&[42]).await.unwrap();
            let until = Instant::now() + Duration::from_millis(10);
            assert!(!send(&mut server, until, None, CommandId(2), Reply::Renewed).await.unwrap());
            drop(server);
            let mut received = Vec::new();
            client.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, [42]);
        });
    }

    #[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
    #[test]
    #[ignore = "requires an owned source fixture and its empty replacement store"]
    fn native_known_frames_renew_analyze_and_refuse_source_replay() {
        let path = |variable: &str| PathBuf::from(std::env::var_os(variable).unwrap());
        let project = path("WOSY_NATIVE_KNOWN_PROJECT").canonicalize().unwrap();
        let root = NodeId::parse_hex(&std::env::var("WOSY_NATIVE_KNOWN_ROOT").unwrap()).unwrap();
        let selected = BuildSourceSelection { root, project: project.to_str().unwrap().into(), target: "app".into() };
        Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
            let lifetime = Duration::from_secs(30);
            let recoverable_start = || Request::StartRecoverable { version: BUILD_RECONCILIATION_VERSION };
            let unit = NonZeroUsize::new(1).unwrap();

            let store = Arc::new(SourceStore::open(&path("WOSY_NATIVE_KNOWN_STORE"), 8 << 20).unwrap());
            assert!(store.snapshot(root).unwrap().is_some());
            let worker = SourceStorageWorker::new(Arc::clone(&store), Handle::current(), unit, unit, unit).unwrap();
            for profile in ["dev", "native"] {
                let (mut client, stream) = io::duplex(256);
                let (end, ()) = tokio::join!(
                    worker.serve_known_build_session_before(stream, selected.clone(), lifetime, Instant::now() + lifetime),
                    async move {
                        let mut frames = SessionFrameReader::default();
                        send_request(&mut client, 1, recoverable_start()).await;
                        assert!(matches!(receive(&mut client, &mut frames, 1).await,
                            Reply::RecoverableStarted { version: BUILD_RECONCILIATION_VERSION, source_root }
                            if source_root == root));
                        send_request(&mut client, 2, Request::Renew).await;
                        assert!(matches!(receive(&mut client, &mut frames, 2).await, Reply::Renewed));
                        send_request(&mut client, 3, Request::Analyze { request: BuildDiagnosticRequest::BuildDiagnostics {
                            version: BUILD_DIAGNOSTIC_VERSION, profile: profile.into(),
                        }}).await;
                        let Reply::Analysis { response: BuildDiagnosticResponse::BuildDiagnostics {
                            version, source_root, profile: actual_profile, diagnostics, .. } }
                            = receive(&mut client, &mut frames, 3).await else { panic!("expected ordinary diagnostics"); };
                        assert_eq!((version, source_root, actual_profile.as_str()), (BUILD_DIAGNOSTIC_VERSION, root, profile));
                        assert!(!diagnostics.is_empty());
                        send_request(&mut client, 4, Request::Disconnect).await;
                        assert!(matches!(receive(&mut client, &mut frames, 4).await, Reply::Closed));
                        assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await.unwrap().is_none());
                    },
                );
                assert_eq!(end.unwrap(), BuildSessionEnd::Disconnected);
            }
            drop(worker);
            Arc::into_inner(store).unwrap().close().await.unwrap();

            let missing = Arc::new(SourceStore::open(&path("WOSY_NATIVE_KNOWN_EMPTY_STORE"), 8 << 20).unwrap());
            assert!(missing.snapshot(root).unwrap().is_none());
            let worker = SourceStorageWorker::new(Arc::clone(&missing), Handle::current(), unit, unit, unit).unwrap();
            let (mut client, stream) = io::duplex(256);
            let (end, ()) = tokio::join!(
                worker.serve_known_build_session_before(stream, selected.clone(), lifetime, Instant::now() + lifetime),
                async move {
                    let mut frames = SessionFrameReader::default();
                    send_request(&mut client, 1, recoverable_start()).await;
                    assert!(matches!(receive(&mut client, &mut frames, 1).await,
                        Reply::ReplayRequired { version: BUILD_RECONCILIATION_VERSION, source_root } if source_root == root));
                    assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await.unwrap().is_none());
                },
            );
            assert_eq!(end.unwrap(), BuildSessionEnd::ReplayRequired);
            assert!(missing.snapshot(root).unwrap().is_none());

            // A ready, valid start cannot regain authority at an expired cut.
            let (mut client, stream) = io::duplex(256);
            send_request(&mut client, 1, recoverable_start()).await;
            let end = worker.serve_known_build_session_before(stream, selected, lifetime, Instant::now()).await.unwrap();
            assert_eq!(end, BuildSessionEnd::Expired);
            assert!(SessionFrameReader::default().read::<_, BuildSessionResponse<Diagnostic>>(&mut client)
                .await.unwrap().is_none());
            drop(worker);
            Arc::into_inner(missing).unwrap().close().await.unwrap();
        });
    }

    fn fixture() -> (PathBuf, BuildSourceSelection, NodeId) {
        let path = PathBuf::from(std::env::var_os("WOSY_TEST_IMAGE_STORE").unwrap());
        let root = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").unwrap()).unwrap();
        let scope = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_OTHER_ROOT").unwrap()).unwrap();
        let workspace = path.parent().unwrap().join("project").canonicalize().unwrap();
        let selection = BuildSourceSelection { root, project: workspace.to_str().unwrap().into(), target: "app".into() };
        (path, selection, scope)
    }

    #[test]
    #[ignore = "requires a disposable store with two admitted source fixture roots"]
    fn framed_lifecycle_releases_session_without_aborting_running_work() {
        let (path, selection, scope) = fixture();
        let root = selection.root;
        let runtime = Builder::new_current_thread().max_blocking_threads(1).enable_time().build().unwrap();
        runtime.block_on(async {
            let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
            let bound = |value| NonZeroUsize::new(value).unwrap();
            let worker = SourceStorageWorker::new(Arc::clone(&store), Handle::current(), bound(1), bound(1), bound(20_000)).unwrap();
            let present = || store.plan_source_retention(scope, &[root], &[], 20_000).unwrap().active_read_roots;
            let lifetime = Duration::from_secs(30);
            let (mut client, stream) = io::duplex(256);
            let (end, ()) = tokio::join!(worker.serve_build_session(stream, selection.clone(), lifetime), async {
                send_request(&mut client, 3, Request::Start { version: BUILD_DIAGNOSTIC_VERSION + 1 }).await;
                let reply = receive(&mut client, &mut SessionFrameReader::default(), 3).await;
                assert!(matches!(reply, Reply::Refused { reason: Refusal::UnsupportedVersion }));
                assert_eq!(present(), 0);
            });
            assert_eq!(end.unwrap(), BuildSessionEnd::Refused);

            let (mut client, stream) = io::duplex(256);
            let (end, (release, barrier)) = tokio::join!(worker.serve_build_session(stream, selection.clone(), lifetime), async {
                let mut frames = SessionFrameReader::default();
                start(&mut client, &mut frames, root).await;
                assert_eq!(present(), 1);
                let (entered, waiting) = tokio::sync::oneshot::channel();
                let (release, blocked) = mpsc::channel::<()>();
                let barrier = Handle::current().spawn_blocking(move || {
                    let _ = entered.send(());
                    let _ = blocked.recv();
                });
                waiting.await.unwrap();
                send_request(&mut client, 11, analyze_request()).await;
                send_request(&mut client, 12, analyze_request()).await;
                let reply = receive(&mut client, &mut frames, 12).await;
                assert!(matches!(reply, Reply::Refused { reason: Refusal::Busy }));
                send_request(&mut client, 13, Request::Renew).await;
                let reply = receive(&mut client, &mut frames, 13).await;
                assert!(matches!(reply, Reply::Renewed));
                drop(client);
                (release, barrier)
            });
            assert_eq!(end.unwrap(), BuildSessionEnd::PeerClosed);
            assert_eq!(present(), 0);
            assert!(matches!(worker.try_inspect(scope, &[root], &[]), Err(StorageAdmissionError::Busy)));
            release.send(()).unwrap();
            barrier.await.unwrap();
            loop {
                match worker.try_inspect(scope, &[root], &[]) {
                    Ok(read) => { assert_eq!(read.finish().await.unwrap().inventory.plan.candidate_roots, 1); break; }
                    Err(StorageAdmissionError::Busy) => tokio::task::yield_now().await,
                    Err(other) => panic!("fixture worker error: {other}"),
                }
            }

            let (mut client, stream) = io::duplex(256);
            let (end, ()) = tokio::join!(worker.serve_build_session(stream, selection.clone(), lifetime), async {
                let mut frames = SessionFrameReader::default();
                start(&mut client, &mut frames, root).await;
                let (entered, waiting) = tokio::sync::oneshot::channel();
                let (release, blocked) = mpsc::channel::<()>();
                let barrier = Handle::current().spawn_blocking(move || {
                    let _ = entered.send(());
                    let _ = blocked.recv();
                });
                waiting.await.unwrap();
                send_request(&mut client, 21, analyze_request()).await;
                send_request(&mut client, 22, analyze_request()).await;
                let busy = receive(&mut client, &mut frames, 22).await;
                assert!(matches!(busy, Reply::Refused { reason: Refusal::Busy }));
                release.send(()).unwrap();
                barrier.await.unwrap();
                let reply = receive(&mut client, &mut frames, 21).await;
                assert!(matches!(reply, Reply::Analysis { response: BuildDiagnosticResponse::BuildDiagnostics {
                    source_root, origin: BuildAnalysisOrigin::Derived, ..
                }} if source_root == root));
                send_request(&mut client, 23, Request::Disconnect).await;
                let reply = receive(&mut client, &mut frames, 23).await;
                assert!(matches!(reply, Reply::Closed));
                assert_eq!(present(), 0);
            });
            assert_eq!(end.unwrap(), BuildSessionEnd::Disconnected);

            let (mut client, stream) = io::duplex(256);
            let (end, ()) = tokio::join!(
                worker.serve_build_session(stream, selection, Duration::from_millis(150)),
                async {
                    let mut frames = SessionFrameReader::default();
                    start(&mut client, &mut frames, root).await;
                    assert_eq!(present(), 1);
                    assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await.unwrap().is_none());
                }
            );
            assert_eq!(end.unwrap(), BuildSessionEnd::Expired);
            assert_eq!(present(), 0);
            drop(worker);
            Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
        });
    }

    #[test]
    #[ignore = "requires a disposable store with two admitted source fixture roots"]
    fn framed_disconnect_stops_later_analyze_phases() {
        use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};

        let (path, selection, scope) = fixture();
        let root = selection.root;
        let runtime = Builder::new_current_thread().max_blocking_threads(1).enable_time().build().unwrap();
        runtime.block_on(async {
            let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
            let bound = |value| NonZeroUsize::new(value).unwrap();
            let mut worker = SourceStorageWorker::new(
                Arc::clone(&store), Handle::current(), bound(1), bound(1), bound(20_000),
            ).unwrap();
            let (entered, wait) = tokio::sync::oneshot::channel();
            let (resume, held) = mpsc::channel::<()>();
            let gate = Mutex::new(Some((entered, held)));
            let phases = Arc::new(AtomicUsize::new(0));
            let observed = Arc::clone(&phases);
            worker.after_cancellable_phase(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                if let Some((entered, held)) = gate.lock().unwrap().take() {
                    // Stop inside the real source-admission phase after its
                    // snapshot owns an independent read, before handing it
                    // back to the async scheduler for compilation.
                    entered.send(()).unwrap();
                    held.recv_timeout(Duration::from_secs(10)).unwrap();
                }
            });
            let limit = Duration::from_secs(10);
            let lifetime = Duration::from_secs(30);
            let (mut client, stream) = io::duplex(256);
            let (end, ()) = tokio::time::timeout(limit, async {
                tokio::join!(worker.serve_build_session(stream, selection.clone(), lifetime), async {
                    let mut frames = SessionFrameReader::default();
                    start(&mut client, &mut frames, root).await;
                    send_request(&mut client, 31, analyze_request()).await;
                    wait.await.unwrap();
                    send_request(&mut client, 32, Request::Disconnect).await;
                    assert!(matches!(receive(&mut client, &mut frames, 32).await, Reply::Closed));
                    assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut client).await.unwrap().is_none());
                })
            }).await.unwrap();
            assert_eq!(end.unwrap(), BuildSessionEnd::Disconnected);
            let held = store.plan_source_retention(scope, &[root], &[], 20_000).unwrap();
            assert_eq!((held.active_read_roots, held.candidate_roots), (1, 0));
            assert!(matches!(worker.try_inspect(scope, &[root], &[]), Err(StorageAdmissionError::Busy)));
            resume.send(()).unwrap();

            let drained = tokio::time::timeout(limit, async {
                loop {
                    match worker.try_inspect(scope, &[root], &[]) {
                        Ok(read) => break read.finish().await.unwrap().inventory.plan,
                        Err(StorageAdmissionError::Busy) => tokio::task::yield_now().await,
                        Err(other) => panic!("fixture worker error: {other}"),
                    }
                }
            }).await.unwrap();
            assert_eq!((drained.active_read_roots, drained.candidate_roots), (0, 1));
            assert_eq!(phases.load(Ordering::Relaxed), 1);

            let (mut client, stream) = io::duplex(256);
            let (end, ()) = tokio::time::timeout(limit, async {
                tokio::join!(worker.serve_build_session(stream, selection, lifetime), async {
                    let mut frames = SessionFrameReader::default();
                    start(&mut client, &mut frames, root).await;
                    send_request(&mut client, 41, analyze_request()).await;
                    assert!(matches!(receive(&mut client, &mut frames, 41).await,
                        Reply::Analysis { response: BuildDiagnosticResponse::BuildDiagnostics {
                            source_root, origin: BuildAnalysisOrigin::Derived, ..
                        }} if source_root == root));
                    drop(client);
                })
            }).await.unwrap();
            assert_eq!(end.unwrap(), BuildSessionEnd::PeerClosed);
            assert!(phases.load(Ordering::Relaxed) > 1);
            drop(worker);
            Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
        });
    }

    #[test]
    #[ignore = "requires a fresh source-only store with two annotation revisions of the same project"]
    fn disconnected_framed_waiter_releases_slot_without_stopping_shared_producer() {
        use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};

        let (path, selected, alternate) = fixture();
        let root = selected.root;
        let cancelled = BuildSourceSelection { root: alternate, ..selected.clone() };
        let runtime = Builder::new_current_thread().max_blocking_threads(4).enable_time().build().unwrap();
        runtime.block_on(async {
            let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
            assert_eq!(store.snapshot(root).unwrap().unwrap().executable,
                store.snapshot(alternate).unwrap().unwrap().executable);
            assert_ne!(root, alternate);
            let bound = |n| NonZeroUsize::new(n).unwrap();
            let mut worker = SourceStorageWorker::new(
                Arc::clone(&store), Handle::current(), bound(3), bound(1), bound(20_000),
            ).unwrap();
            let (entered, wait_for_producer) = tokio::sync::oneshot::channel();
            let (resume, held) = mpsc::channel::<()>();
            let producer_gate = Mutex::new(Some((entered, held)));
            let phases = AtomicUsize::new(0);
            worker.after_cancellable_phase(move || {
                // Fresh producer: lease read, project index scan, then
                // actual base planning inside its sorted AST flights.
                if phases.fetch_add(1, Ordering::Relaxed) == 2 {
                    let (entered, held) = producer_gate.lock().unwrap().take().unwrap();
                    entered.send(()).unwrap();
                    held.recv_timeout(Duration::from_secs(10)).unwrap();
                }
            });
            let (blocked, mut waits) = tokio::sync::mpsc::channel(8);
            worker.on_cancellable_gate_wait(move || blocked.try_send(()).unwrap());

            let (mut producer, producer_stream) = io::duplex(256);
            let (mut waiter, waiter_stream) = io::duplex(256);
            let (mut follower, follower_stream) = io::duplex(256);
            let lifetime = Duration::from_secs(30);
            let (owner, abandoned, next, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::join!(
                    worker.serve_build_session(producer_stream, selected.clone(), lifetime),
                    worker.serve_build_session(waiter_stream, cancelled, lifetime),
                    worker.serve_build_session(follower_stream, selected, lifetime),
                    async {
                        let mut producer_frames = SessionFrameReader::default();
                        let mut waiter_frames = SessionFrameReader::default();
                        let mut follower_frames = SessionFrameReader::default();
                        start(&mut producer, &mut producer_frames, root).await;
                        send_request(&mut producer, 61, analyze_request()).await;
                        wait_for_producer.await.unwrap();
                        start(&mut waiter, &mut waiter_frames, alternate).await;
                        send_request(&mut waiter, 71, analyze_request()).await;
                        waits.recv().await.unwrap();
                        start(&mut follower, &mut follower_frames, root).await;
                        send_request(&mut follower, 81, analyze_request()).await;
                        waits.recv().await.unwrap();
                        assert!(matches!(worker.try_inspect(root, &[alternate], &[]), Err(StorageAdmissionError::Busy)));
                        let before = store.plan_source_retention(root, &[alternate], &[], 20_000).unwrap();
                        assert_eq!((before.active_read_roots, before.candidate_roots), (2, 0));

                        send_request(&mut waiter, 72, Request::Disconnect).await;
                        assert!(matches!(receive(&mut waiter, &mut waiter_frames, 72).await, Reply::Closed));
                        assert!(waiter_frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut waiter).await.unwrap().is_none());
                        let after = loop {
                            match worker.try_inspect(root, &[alternate], &[]) {
                                Ok(job) => break job.finish().await.unwrap().inventory.plan,
                                Err(StorageAdmissionError::Busy) => tokio::task::yield_now().await,
                                Err(other) => panic!("fixture worker error: {other}"),
                            }
                        };
                        // No owner progress: cancelled revision's admitted
                        // read and the sole freed slot have drained already.
                        assert_eq!((after.active_read_roots, after.candidate_roots), (1, 1));
                        resume.send(()).unwrap();
                        let (a, b) = tokio::join!(
                            receive(&mut producer, &mut producer_frames, 61),
                            receive(&mut follower, &mut follower_frames, 81),
                        );
                        let shape = |reply| match reply {
                            Reply::Analysis { response: BuildDiagnosticResponse::BuildDiagnostics {
                                source_root, profile, preparation, receipt, phase, origin, diagnostics, ..
                            }} => ((source_root, profile, preparation, receipt, phase, diagnostics), origin),
                            _ => panic!("expected the ordinary owner and follower results"),
                        };
                        let (a, owner_origin) = shape(a);
                        let (b, follower_origin) = shape(b);
                        assert_eq!(a, b);
                        assert_eq!([owner_origin, follower_origin].into_iter()
                            .filter(|origin| *origin == BuildAnalysisOrigin::Derived).count(), 1);
                        drop(producer);
                        drop(follower);
                    },
                )
            }).await.unwrap();
            assert_eq!((owner.unwrap(), abandoned.unwrap(), next.unwrap()), (
                BuildSessionEnd::PeerClosed, BuildSessionEnd::Disconnected, BuildSessionEnd::PeerClosed,
            ));
            drop(worker);
            Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
        });
    }
}
