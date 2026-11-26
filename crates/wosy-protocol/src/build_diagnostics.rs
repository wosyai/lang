use serde::{Deserialize, Serialize};

use super::{BaseCstProposal, NodeId};

pub const BUILD_DIAGNOSTIC_VERSION: u16 = 1;
pub const BASE_DECLARATION_PROJECTION_VERSION: u16 = 1;
pub const BUILD_RECONCILIATION_VERSION: u16 = 1;

/// Local adapter's already acknowledged SQLite selection. This is not a
/// wire authorization or a way for a request to switch roots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildSourceSelection {
    pub root: NodeId,
    pub project: String,
    pub target: String,
}

/// The server's selected session supplies the root; a command cannot select
/// a different snapshot or bypass the prepared profile.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuildDiagnosticRequest {
    BuildDiagnostics { version: u16, profile: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BuildAnalysisPhase {
    PreparationRejected,
    Analyzed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BuildAnalysisOrigin {
    Derived,
    Live,
    Stored,
}

/// Only compiler-owned diagnostics, not a Build result, artifact or Check
/// verdict. The daemon supplies the canonical compiler diagnostic model as D.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuildDiagnosticResponse<D> {
    BuildDiagnostics {
        version: u16,
        source_root: NodeId,
        profile: String,
        preparation: NodeId,
        receipt: NodeId,
        phase: BuildAnalysisPhase,
        origin: BuildAnalysisOrigin,
        diagnostics: Vec<D>,
    },
}

/// A sender does not reuse an ID until it has received that command's reply.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BuildSessionCommandId(pub u64);

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSessionCommand {
    pub id: BuildSessionCommandId,
    pub request: BuildSessionRequest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSessionResponse<D> {
    pub id: BuildSessionCommandId,
    pub reply: BuildSessionReply<D>,
}

/// A connection negotiates the diagnostic protocol before using its single
/// host-selected source lease. None of these messages select a source root.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuildSessionRequest {
    Start { version: u16 },
    /// Available only at a host-configured recoverable session. The wire
    /// request never contains the scope or a source payload.
    StartRecoverable { version: u16 },
    Analyze { request: BuildDiagnosticRequest },
    ProjectBaseDeclarations { version: u16, proposal: BaseCstProposal },
    Renew,
    Disconnect,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSessionRefusal {
    ExpectedStart,
    AlreadyStarted,
    UnsupportedVersion,
    Busy,
    SourceUnavailable,
    AnalysisUnavailable,
    BaseProjectionUnavailable,
    BaseProjectionAlreadyHeld,
    ResponseTooLarge,
}

/// Ordinal of the issued lease within the hosting store instance. A client
/// learns it on the same live stream; it is not a durable restart token.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BaseDeclarationConnectionId(pub u64);

/// Observation of the connection's ordinary base declaration owner. Not a
/// source acknowledgement, secondary lease, lookup token, or semantic Prepared.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaseDeclarationProjection {
    pub version: u16,
    pub connection: BaseDeclarationConnectionId,
    pub base: NodeId,
    pub proposed: NodeId,
    pub attempt: i64,
    pub preparation: NodeId,
    pub entry_projection: NodeId,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuildSessionReply<D> {
    Started { version: u16, source_root: NodeId },
    RecoverableStarted { version: u16, source_root: NodeId },
    /// Terminal for this connection and not a lease or a publication ack.
    ReplayRequired { version: u16, source_root: NodeId },
    BaseDeclarationSessionStarted { version: u16, source_root: NodeId, connection: BaseDeclarationConnectionId },
    Analysis { response: BuildDiagnosticResponse<D> },
    BaseDeclarationProjection { projection: BaseDeclarationProjection },
    Renewed,
    Closed,
    Refused { reason: BuildSessionRefusal },
}
