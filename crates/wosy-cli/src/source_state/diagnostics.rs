//! The client side of one recoverable, source-only diagnostic conversation.
use super::{Result, SourceReplay, SqliteSourceState};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use wosy_compiler::Diagnostic;
use wosy_protocol::{
    encode_session_frame, BuildDiagnosticRequest, BuildDiagnosticResponse, BuildSessionCommand,
    BuildSessionCommandId, BuildSessionRefusal, BuildSessionReply, BuildSessionRequest,
    BuildSessionResponse, BuildSourceSelection, SessionFrameReader, BUILD_DIAGNOSTIC_VERSION,
    BUILD_RECONCILIATION_VERSION,
};

#[derive(Clone)]
pub struct CapturedBuildDiagnostics {
    pub(super) selected: BuildSourceSelection,
    pub(super) generation: i64,
    profile: String,
}

pub struct RequiredSourceReplay(CapturedBuildDiagnostics);

pub enum DiagnosticExchange {
    StaleSelection,
    ReplayRequired(RequiredSourceReplay),
    Refused(BuildSessionRefusal),
    Diagnostics(BuildDiagnosticResponse<Diagnostic>),
}

impl SqliteSourceState {
    /// Capture the acknowledged revision, not the last issued upload attempt.
    pub fn capture_build_diagnostics(&self, target: &str, profile: &str) -> Result<Option<CapturedBuildDiagnostics>> {
        let state = self.selection(target)?;
        let Some(root) = state.selected else { return Ok(None); };
        let generation = state.selected_generation.ok_or("acknowledged source has no generation")?;
        let project = self.project.to_str().ok_or("workspace path is not UTF-8")?;
        Ok(Some(CapturedBuildDiagnostics {
            selected: BuildSourceSelection { root, project: project.into(), target: target.into() },
            generation, profile: profile.into(),
        }))
    }
}

impl CapturedBuildDiagnostics {
    /// Local host adapter's connection scope, not input to the wire command.
    pub fn selection(&self) -> &BuildSourceSelection { &self.selected }

    pub fn generation(&self) -> i64 { self.generation }

    pub(super) fn is_current(&self, local: &SqliteSourceState) -> Result<bool> {
        if local.project.to_str() != Some(self.selected.project.as_str()) { return Ok(false); }
        let state = local.selection(&self.selected.target)?;
        Ok(state.selected == Some(self.selected.root) && state.selected_generation == Some(self.generation))
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
        &self, local: &mut SqliteSourceState, mut stream: T, timeout: Duration,
    ) -> Result<DiagnosticExchange> {
        if !self.is_current(local)? { return Ok(DiagnosticExchange::StaleSelection); }
        Ok(tokio::time::timeout(timeout, self.converse(&mut stream)).await??)
    }

    async fn converse<T: AsyncRead + AsyncWrite + Unpin>(&self, stream: &mut T) -> Result<DiagnosticExchange> {
        let mut frames = SessionFrameReader::default();
        send(stream, 1, BuildSessionRequest::StartRecoverable { version: BUILD_RECONCILIATION_VERSION }).await?;
        match receive(stream, &mut frames, 1).await? {
            BuildSessionReply::ReplayRequired { version: BUILD_RECONCILIATION_VERSION, source_root }
                if source_root == self.selected.root =>
            {
                terminal(stream, &mut frames).await?;
                return Ok(DiagnosticExchange::ReplayRequired(RequiredSourceReplay(self.clone())));
            }
            BuildSessionReply::RecoverableStarted { version: BUILD_RECONCILIATION_VERSION, source_root }
                if source_root == self.selected.root => {}
            BuildSessionReply::Refused { reason } => {
                terminal(stream, &mut frames).await?;
                return Ok(DiagnosticExchange::Refused(reason));
            }
            _ => return Err("diagnostic session did not select the captured source".into()),
        }
        send(stream, 2, BuildSessionRequest::Analyze { request: BuildDiagnosticRequest::BuildDiagnostics {
            version: BUILD_DIAGNOSTIC_VERSION, profile: self.profile.clone(),
        }}).await?;
        let result = match receive(stream, &mut frames, 2).await? {
            BuildSessionReply::Analysis { response } => {
                let BuildDiagnosticResponse::BuildDiagnostics { version, source_root, profile, .. } = &response;
                if *version != BUILD_DIAGNOSTIC_VERSION || *source_root != self.selected.root || profile != &self.profile {
                    return Err("diagnostic reply does not match the captured source or profile".into());
                }
                DiagnosticExchange::Diagnostics(response)
            }
            BuildSessionReply::Refused { reason } => DiagnosticExchange::Refused(reason),
            _ => return Err("diagnostic request did not receive an analysis or refusal".into()),
        };
        send(stream, 3, BuildSessionRequest::Disconnect).await?;
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
        local.replay_acknowledged_at(&self.0.selected, Some(self.0.generation))
    }
}

async fn send<T: AsyncWrite + Unpin>(stream: &mut T, id: u64, request: BuildSessionRequest) -> Result<()> {
    stream.write_all(&encode_session_frame(&BuildSessionCommand { id: BuildSessionCommandId(id), request })?).await?;
    stream.flush().await?;
    Ok(())
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
