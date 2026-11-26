//! Route one bounded byte stream through the existing diagnostic session.
use super::{
    build_session::BuildRequestRejection, encode_session_frame, BaseCstAdmissionLimits,
    BuildDiagnosticSession, BuildSourceOpening, PreparedBaseDeclarations, Result, SessionFrameError, SessionFrameReader,
    SourceStorageWorker, StorageAdmissionError,
};
use std::future::poll_fn;
use std::task::Poll;
use std::time::{Duration, Instant};
use tokio::io::{self, AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::{sleep_until, Instant as TokioInstant};
use wosy_compiler::Diagnostic;
use wosy_protocol::{
    BaseCstProposal, BaseCstProposalBinding, BaseDeclarationProjection, BuildDiagnosticRequest,
    BuildDiagnosticResponse, BuildSessionCommand as Command, BuildSessionCommandId as CommandId,
    BuildSessionRefusal as Refusal, BuildSessionReply, BuildSessionRequest as Request,
    BuildSessionResponse, BuildSourceSelection, Delta, BASE_DECLARATION_PROJECTION_VERSION,
    BUILD_DIAGNOSTIC_VERSION, BUILD_RECONCILIATION_VERSION,
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

enum Opening {
    Ordinary(Option<ProjectionScope>),
    Recoverable { replay: Option<Delta>, limits: BaseCstAdmissionLimits },
}

enum Opened<'a> {
    Session(BuildDiagnosticSession<'a>),
    Unavailable,
    ReplayRequired,
}

enum Operation<'a> {
    Analyze(BuildDiagnosticRequest),
    Project(&'a ProjectionScope, BaseCstProposal),
}

enum Completed {
    Analysis(BuildDiagnosticResponse<Diagnostic>),
    Projection(PreparedBaseDeclarations),
}

impl SourceStorageWorker {
    /// Serve one host-selected connection on a Tokio runtime with its time
    /// driver enabled. The adapter supplies the acknowledged selection;
    /// wire commands never change roots. This does not provide a listener.
    pub async fn serve_build_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, lifetime: Duration,
    ) -> Result<BuildSessionEnd> {
        self.serve_selected_session(stream, selected, lifetime, Opening::Ordinary(None)).await
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
        self.serve_selected_session(stream, selected, lifetime, Opening::Ordinary(Some(scope))).await
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
        self.serve_selected_session(stream, selected, lifetime, Opening::Recoverable { replay, limits }).await
    }

    async fn serve_selected_session<T: AsyncRead + AsyncWrite + Unpin>(
        &self, stream: T, selected: BuildSourceSelection, lifetime: Duration,
        opening: Opening,
    ) -> Result<BuildSessionEnd> {
        let start_deadline = Instant::now().checked_add(lifetime).ok_or("Build session start timeout is out of range")?;
        let (mut input, mut output) = io::split(stream);
        let mut frames = SessionFrameReader::default();
        let start = tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(start_deadline)) => return Ok(BuildSessionEnd::Expired),
            event = frames.read::<_, Command>(&mut input) => event?,
        };
        let Some(Command { id, request }) = start else { return Ok(BuildSessionEnd::PeerClosed); };
        let recoverable = matches!(opening, Opening::Recoverable { .. });
        let refused = match (recoverable, request) {
            (false, Request::Start { version }) if version == BUILD_DIAGNOSTIC_VERSION => None,
            (true, Request::StartRecoverable { version }) if version == BUILD_RECONCILIATION_VERSION => None,
            (false, Request::Start { .. }) | (true, Request::StartRecoverable { .. }) => Some(Refusal::UnsupportedVersion),
            _ => Some(Refusal::ExpectedStart),
        };
        if let Some(reason) = refused {
            return refuse_start(&mut output, start_deadline, id, reason).await;
        }
        if Instant::now() >= start_deadline { return Ok(BuildSessionEnd::Expired); }
        let source_root = selected.root;
        let (projection, recovery) = match opening {
            Opening::Ordinary(projection) => (projection, None),
            Opening::Recoverable { replay, limits } => (None, Some((replay, limits))),
        };
        let opened: Result<Opened<'_>> = tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(start_deadline)) => return Ok(BuildSessionEnd::Expired),
            opened = async {
                if let Some((replay, limits)) = recovery {
                    Ok(match self.open_recoverable_build_diagnostics_before(selected, replay, limits, lifetime, Some(start_deadline)).await? {
                        BuildSourceOpening::Opened { session, .. } => Opened::Session(session),
                        BuildSourceOpening::ReplayRequired { .. } => Opened::ReplayRequired,
                    })
                } else {
                    Ok(self.open_build_diagnostics_before(selected, lifetime, Some(start_deadline)).await?
                        .map_or(Opened::Unavailable, Opened::Session))
                }
            } => opened,
        };
        let session = match opened {
            Ok(Opened::Session(session)) => session,
            Ok(Opened::Unavailable) => return refuse_start(&mut output, start_deadline, id, Refusal::SourceUnavailable).await,
            Ok(Opened::ReplayRequired) => {
                let reply = Reply::ReplayRequired { version: BUILD_RECONCILIATION_VERSION, source_root };
                return Ok(if send(&mut output, start_deadline, id, reply).await? {
                    BuildSessionEnd::ReplayRequired
                } else { BuildSessionEnd::Expired });
            }
            Err(error) => {
                if matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)) {
                    return Ok(BuildSessionEnd::Expired);
                }
                let reason = refusal(&error, Refusal::SourceUnavailable);
                return refuse_start(&mut output, start_deadline, id, reason).await;
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
        if !send(&mut output, deadline.min(start_deadline), id, started).await? { return Ok(BuildSessionEnd::Expired); }
        let end = run(&session, &mut frames, &mut input, &mut output, projection.as_ref()).await?;
        let deadline = session.deadline();
        session.release();
        Ok(match end {
            Close::Request(id) => {
                if let Some(deadline) = deadline { send(&mut output, deadline, id, Reply::Closed).await?; }
                BuildSessionEnd::Disconnected
            }
            Close::Peer => BuildSessionEnd::PeerClosed,
            Close::Deadline => BuildSessionEnd::Expired,
        })
    }
}

async fn run<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    session: &BuildDiagnosticSession<'_>, frames: &mut SessionFrameReader, input: &mut R,
    output: &mut W, projection: Option<&ProjectionScope>,
) -> Result<Close> {
    let mut owner = None;
    loop {
        let Some(deadline) = session.deadline() else { return Ok(Close::Deadline); };
        let request = tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(deadline)) => return Ok(Close::Deadline),
            request = frames.read::<_, Command>(input) => request?,
        };
        let (id, operation) = match request {
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
                if !send(output, deadline, id, Reply::Refused { reason: refusal }).await? {
                    return Ok(Close::Deadline);
                }
                continue;
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
    };
    let job = async {
        match operation {
            Operation::Analyze(request) => session.dispatch_framed(request).await.map(Completed::Analysis),
            Operation::Project(scope, proposal) => session.project_base_declarations(
                scope.binding.clone(), proposal, scope.admission_limits, scope.declaration_bytes,
            ).await.map(Completed::Projection),
        }
    };
    tokio::pin!(job);
    loop {
        let Some(deadline) = session.deadline() else { return Ok(Some(Close::Deadline)); };
        tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(deadline)) => return Ok(Some(Close::Deadline)),
            response = &mut job => {
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
                    Err(error) => {
                        if matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)) {
                            return Ok(Some(Close::Deadline));
                        }
                        Reply::Refused { reason: refusal(&error, failure) }
                    }
                };
                let written = match send(output, deadline, id, reply).await {
                    Err(SessionFrameError::TooLarge) =>
                        send(output, deadline, id, Reply::Refused { reason: Refusal::ResponseTooLarge }).await?,
                    result => result?,
                };
                return Ok((!written).then_some(Close::Deadline));
            }
            request = frames.read::<_, Command>(input) => {
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
    let Some(Command { id, request }) = command else { return Ok(Some(Close::Peer)); };
    let reply = match request {
        Request::Disconnect => return Ok(Some(Close::Request(id))),
        Request::Start { .. } | Request::StartRecoverable { .. } => Reply::Refused { reason: Refusal::AlreadyStarted },
        Request::Analyze { .. } | Request::ProjectBaseDeclarations { .. } => Reply::Refused { reason: Refusal::Busy },
        Request::Renew => {
            if !session.renew() { return Ok(Some(Close::Deadline)); }
            Reply::Renewed
        }
    };
    let Some(deadline) = session.deadline() else { return Ok(Some(Close::Deadline)); };
    Ok((!send(output, deadline, id, reply).await?).then_some(Close::Deadline))
}

async fn refuse_start<W: AsyncWrite + Unpin>(output: &mut W, until: Instant, id: CommandId, reason: Refusal) -> Result<BuildSessionEnd> {
    Ok(if send(output, until, id, Reply::Refused { reason }).await? { BuildSessionEnd::Refused } else { BuildSessionEnd::Expired })
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
    output: &mut W, until: Instant, id: CommandId, reply: Reply,
) -> std::result::Result<bool, SessionFrameError> {
    let bytes = encode_session_frame(&BuildSessionResponse { id, reply })?;
    if Instant::now() >= until { return Ok(false); }
    tokio::select! {
        biased;
        _ = sleep_until(TokioInstant::from_std(until)) => Ok(false),
        result = async {
            let mut written = 0;
            while written < bytes.len() {
                let progress = poll_fn(|cx| {
                    // Timer wake-up is only a bound on waiting. The actual
                    // lease clock authorizes the first byte, including polls
                    // after backpressure that have not written anything yet.
                    if written == 0 && Instant::now() >= until { return Poll::Ready(Ok(None)); }
                    std::pin::Pin::new(&mut *output).poll_write(cx, &bytes[written..]).map_ok(Some)
                }).await?;
                match progress {
                    None => return Ok(false),
                    Some(0) => return Err(std::io::Error::from(std::io::ErrorKind::WriteZero)),
                    Some(count) => written += count,
                }
            }
            // An expired partial send ends the stream; those bytes are not
            // recalled and are never retried as a new frame.
            output.flush().await?;
            Ok(true)
        } => result.map_err(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceStore, StorageAdmissionError};
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
            assert!(!send(&mut server, Instant::now(), CommandId(1), Reply::Renewed).await.unwrap());
            server.write_all(&[42]).await.unwrap();
            let until = Instant::now() + Duration::from_millis(10);
            assert!(!send(&mut server, until, CommandId(2), Reply::Renewed).await.unwrap());
            drop(server);
            let mut received = Vec::new();
            client.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, [42]);
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
}
