//! The client side of one recoverable, source-only diagnostic conversation.
use super::{AcknowledgedSourceGeneration, Result, SourceReplay, SqliteSourceState};
use std::{future::Future, io, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use wosy_compiler::Diagnostic;
use wosy_protocol::{
    encode_session_frame, write_encoded_session_frame, BuildDiagnosticRequest, BuildDiagnosticResponse, BuildSessionCommand,
    BuildSessionCommandId, BuildSessionRefusal, BuildSessionReply, BuildSessionRequest,
    BuildSessionResponse, BuildSourceSelection, OrdinaryLlvmProjection, SessionFrameReader, BUILD_DIAGNOSTIC_VERSION,
    BUILD_RECONCILIATION_VERSION, ORDINARY_LLVM_PROJECTION_VERSION,
};

#[derive(Clone)]
pub struct CapturedBuildDiagnostics {
    pub(super) source: AcknowledgedSourceGeneration,
    profile: String,
}

pub struct RequiredSourceReplay(CapturedBuildDiagnostics);

pub enum DiagnosticExchange {
    StaleSelection,
    ReplayRequired(RequiredSourceReplay),
    Refused(BuildSessionRefusal),
    Diagnostics(BuildDiagnosticResponse<Diagnostic>),
}

pub(super) enum CommandExchange<T> {
    StaleSelection,
    ReplayRequired(RequiredSourceReplay),
    Refused(BuildSessionRefusal),
    Answer(T),
}

impl From<CommandExchange<BuildDiagnosticResponse<Diagnostic>>> for DiagnosticExchange {
    fn from(exchange: CommandExchange<BuildDiagnosticResponse<Diagnostic>>) -> Self {
        match exchange {
            CommandExchange::StaleSelection => Self::StaleSelection,
            CommandExchange::ReplayRequired(replay) => Self::ReplayRequired(replay),
            CommandExchange::Refused(reason) => Self::Refused(reason),
            CommandExchange::Answer(response) => Self::Diagnostics(response),
        }
    }
}

pub(super) fn diagnostic_deadline(timeout: Duration) -> Result<Instant> {
    Instant::now().checked_add(timeout).ok_or("invalid diagnostic session deadline".into())
}

pub(super) fn require_diagnostic_time(deadline: Instant) -> Result<()> {
    if Instant::now() < deadline { Ok(()) }
    else { Err(diagnostic_timeout()) }
}

fn diagnostic_timeout() -> Box<dyn std::error::Error + Send + Sync> {
    io::Error::new(io::ErrorKind::TimedOut, "diagnostic session deadline elapsed").into()
}

pub(super) async fn within_diagnostic_deadline<T>(
    deadline: Instant, operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = tokio::time::sleep_until(deadline) => Err(diagnostic_timeout()),
        answer = operation => answer,
    }
}

impl SqliteSourceState {
    /// Capture the acknowledged revision, not the last issued upload attempt.
    pub fn capture_build_diagnostics(&self, target: &str, profile: &str) -> Result<Option<CapturedBuildDiagnostics>> {
        Ok(self.capture_acknowledged_source(target)?.map(|source| CapturedBuildDiagnostics { source, profile: profile.into() }))
    }
}

impl CapturedBuildDiagnostics {
    /// Local host adapter's connection scope, not input to the wire command.
    pub fn selection(&self) -> &BuildSourceSelection { self.source.selection() }

    pub fn generation(&self) -> i64 { self.source.generation() }

    pub(super) fn is_current(&self, local: &SqliteSourceState) -> Result<bool> {
        self.source.is_current(local)
    }

    /// An owned stream dedicated to the captured selection. The trusted
    /// adapter binds its server peer out of band; no source or raw graph
    /// lookup travels on this diagnostic stream. Dropping the operation
    /// discards the stream. This does not provide or select a listener.
    pub async fn exchange<T: AsyncRead + AsyncWrite + Unpin + 'static>(
        &self, local: &mut SqliteSourceState, stream: T, timeout: Duration,
    ) -> Result<DiagnosticExchange> {
        let result = self.exchange_unprojected(local, stream, timeout).await?;
        // One local read after the exchange. No SQLite transaction spans
        // the asynchronous conversation or makes a cross-store guarantee.
        if !matches!(result, DiagnosticExchange::StaleSelection) && !self.is_current(local)? {
            return Ok(DiagnosticExchange::StaleSelection);
        }
        Ok(result)
    }

    pub(super) async fn exchange_unprojected<T: AsyncRead + AsyncWrite + Unpin + 'static>(
        &self, local: &mut SqliteSourceState, stream: T, timeout: Duration,
    ) -> Result<DiagnosticExchange> {
        let deadline = diagnostic_deadline(timeout)?;
        within_diagnostic_deadline(deadline, self.converse_current(local, stream, deadline)).await
    }

    pub(super) async fn converse_current<T: AsyncRead + AsyncWrite + Unpin + 'static>(
        &self, local: &mut SqliteSourceState, stream: T, deadline: Instant,
    ) -> Result<DiagnosticExchange> {
        self.converse_command(local, stream, deadline, Self::analysis_request, Self::accept_analysis)
            .await.map(Into::into)
    }

    pub(super) fn analysis_request(&self) -> BuildSessionRequest {
        BuildSessionRequest::Analyze { request: BuildDiagnosticRequest::BuildDiagnostics {
            version: BUILD_DIAGNOSTIC_VERSION, profile: self.profile.clone(),
        }}
    }

    pub(super) fn accept_analysis(&self, reply: BuildSessionReply<Diagnostic>)
        -> Result<BuildDiagnosticResponse<Diagnostic>> {
        let BuildSessionReply::Analysis { response } = reply else {
            return Err("diagnostic request did not receive an analysis or refusal".into());
        };
        let BuildDiagnosticResponse::BuildDiagnostics { version, source_root, profile, .. } = &response;
        if *version != BUILD_DIAGNOSTIC_VERSION || *source_root != self.selection().root || profile != &self.profile {
            return Err("diagnostic reply does not match the captured source or profile".into());
        }
        Ok(response)
    }

    pub(super) fn ordinary_llvm_request(&self) -> BuildSessionRequest {
        BuildSessionRequest::ProjectOrdinaryLlvm {
            version: ORDINARY_LLVM_PROJECTION_VERSION, profile: self.profile.clone(),
        }
    }

    pub(super) fn accept_ordinary_llvm(
        &self, reply: BuildSessionReply<Diagnostic>, max_bytes: usize,
    ) -> Result<OrdinaryLlvmProjection> {
        let BuildSessionReply::OrdinaryLlvmProjection { projection } = reply else {
            return Err("ordinary LLVM request did not receive a projection".into());
        };
        if projection.version != ORDINARY_LLVM_PROJECTION_VERSION || projection.source_root != self.selection().root
            || projection.profile != self.profile {
            return Err("ordinary LLVM reply did not match the captured source and profile".into());
        }
        if projection.partition.len() > max_bytes {
            return Err("ordinary LLVM partition exceeds the client byte budget".into());
        }
        if blake3::hash(projection.partition.as_bytes()).as_bytes() != &projection.digest.0 {
            return Err("ordinary LLVM partition does not match its digest".into());
        }
        Ok(projection)
    }

    pub(super) async fn converse_command<Stream, Answer>(
        &self, local: &mut SqliteSourceState, mut stream: Stream, deadline: Instant,
        command: impl FnOnce(&Self) -> BuildSessionRequest,
        verify: impl FnOnce(&Self, BuildSessionReply<Diagnostic>) -> Result<Answer>,
    ) -> Result<CommandExchange<Answer>>
    where Stream: AsyncRead + AsyncWrite + Unpin + 'static {
        if !self.is_current(local)? { return Ok(CommandExchange::StaleSelection); }
        let answer = self.converse(&mut stream, deadline, command, verify).await?;
        require_diagnostic_time(deadline)?;
        Ok(answer)
    }

    async fn converse<T: AsyncRead + AsyncWrite + Unpin, Answer>(
        &self, stream: &mut T, deadline: Instant,
        command: impl FnOnce(&Self) -> BuildSessionRequest,
        verify: impl FnOnce(&Self, BuildSessionReply<Diagnostic>) -> Result<Answer>,
    ) -> Result<CommandExchange<Answer>> {
        let mut frames = SessionFrameReader::default();
        send(stream, deadline, 1, BuildSessionRequest::StartRecoverable { version: BUILD_RECONCILIATION_VERSION }).await?;
        match receive(stream, &mut frames, 1).await? {
            BuildSessionReply::ReplayRequired { version: BUILD_RECONCILIATION_VERSION, source_root }
                if source_root == self.selection().root =>
            {
                terminal(stream, &mut frames).await?;
                return Ok(CommandExchange::ReplayRequired(RequiredSourceReplay(self.clone())));
            }
            BuildSessionReply::RecoverableStarted { version: BUILD_RECONCILIATION_VERSION, source_root }
                if source_root == self.selection().root => {}
            BuildSessionReply::Refused { reason } => {
                terminal(stream, &mut frames).await?;
                return Ok(CommandExchange::Refused(reason));
            }
            _ => return Err("diagnostic session did not select the captured source".into()),
        }
        send(stream, deadline, 2, command(self)).await?;
        let result = match receive(stream, &mut frames, 2).await? {
            BuildSessionReply::Refused { reason } => CommandExchange::Refused(reason),
            reply => CommandExchange::Answer(verify(self, reply)?),
        };
        send(stream, deadline, 3, BuildSessionRequest::Disconnect).await?;
        if !matches!(receive(stream, &mut frames, 3).await?, BuildSessionReply::Closed) {
            return Err("diagnostic session did not close".into());
        }
        terminal(stream, &mut frames).await?;
        Ok(result)
    }
}

impl RequiredSourceReplay {
    /// Reopen SQLite's same acknowledged source in one read transaction.
    /// A pending upload or an older acknowledgement of identical bytes can
    /// never substitute. Supply the result via the trusted host on a fresh
    /// stream; this capability is not a final publication receipt.
    pub fn prepare(self, local: &mut SqliteSourceState) -> Result<Option<SourceReplay>> {
        local.replay_acknowledged_at(self.0.selection(), Some(self.0.generation()))
    }
}

async fn send<T: AsyncWrite + Unpin>(
    stream: &mut T, deadline: Instant, id: u64, request: BuildSessionRequest,
) -> Result<()> {
    let bytes = encode_session_frame(&BuildSessionCommand { id: BuildSessionCommandId(id), request })?;
    if write_encoded_session_frame(stream, &bytes, || Instant::now() < deadline).await? { Ok(()) }
    else { Err(diagnostic_timeout()) }
}

async fn receive<T: AsyncRead + Unpin>(
    stream: &mut T, frames: &mut SessionFrameReader, id: u64,
) -> Result<BuildSessionReply<Diagnostic>> {
    let message = frames.read::<_, BuildSessionResponse<Diagnostic>>(stream).await?.ok_or("missing diagnostic reply")?;
    if message.id != BuildSessionCommandId(id) { return Err("diagnostic reply does not match the outstanding command".into()); }
    Ok(message.reply)
}

async fn terminal<T: AsyncRead + Unpin>(stream: &mut T, frames: &mut SessionFrameReader) -> Result<()> {
    if frames.read::<_, BuildSessionResponse<Diagnostic>>(stream).await?.is_some() {
        return Err("diagnostic session sent an unexpected reply after completion".into());
    }
    Ok(())
}
