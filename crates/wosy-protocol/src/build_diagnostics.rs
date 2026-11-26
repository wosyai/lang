use serde::{Deserialize, Serialize};

use super::NodeId;

pub const BUILD_DIAGNOSTIC_VERSION: u16 = 1;

/// Local adapter's already acknowledged SQLite selection. This is not a
/// wire authorization or a way for a request to switch roots.
#[derive(Clone)]
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
