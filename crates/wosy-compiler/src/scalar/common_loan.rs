use super::reference::source_epoch;
use super::*;

mod extraction;
pub(super) mod local_slice;
pub use local_slice::{LocalLoanCandidateRef, LocalLoanPoint, LocalLoanSlice};
mod solver;
#[cfg(test)]
pub(super) mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CommonLoanRoleId(pub usize);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CommonLoanSiteId(pub usize);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanSourceRole {
    Binding(ReferenceBindingId),
    Evaluated(EvaluatedValueId),
    Frozen(SourceEventId),
    Saved(SourceEventId),
    Result(SourceEventId),
    Boundary(BoundaryPortRole),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanRole {
    pub owner: ProducerCfgOwnerId,
    pub source: CommonLoanSourceRole,
    pub projection: Vec<ValueProjection>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanTarget {
    Storage {
        binding: ReferenceBindingId,
        projection: Vec<ValueProjection>,
    },
    Through {
        pointer: CommonLoanRoleId,
        place: ScalarPlace,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanSite {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub mode: ScalarReferenceMutability,
    pub target: CommonLoanTarget,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanUseKind {
    Access,
    Terminal,
    Escape,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanFactKind {
    NoLoan {
        outputs: Vec<EvaluatedValueId>,
    },
    Issue {
        output: CommonLoanRoleId,
        site: CommonLoanSiteId,
    },
    Flow {
        source: CommonLoanRoleId,
        target: CommonLoanRoleId,
        policy: CopyPolicy,
    },
    KillSlot(CommonLoanRoleId),
    Use {
        role: CommonLoanRoleId,
        mode: ScalarReferenceMutability,
        target: CommonLoanTarget,
        kind: CommonLoanUseKind,
    },
    ParentSite {
        child: CommonLoanSiteId,
        parent: CommonLoanRoleId,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanFact {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub kind: CommonLoanFactKind,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanEvent {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub kind: source_epoch::SourceEventKind,
    pub successors: Vec<SourceEventId>,
    pub facts: Vec<CommonLoanFact>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanPendingReason {
    MissingOutput,
    AmbiguousOutputs(Vec<EvaluatedValueId>),
    MissingBindingInput(ReferenceBindingId),
    MissingAssignmentInput,
    DeferredPolicy,
    Projection,
    SelectedTarget,
    Shape(ValueShapeId),
    Boundary,
    Capture,
    Call,
    RawAuthority,
    Release,
    Invalidate,
    UnresolvedParent(CommonLoanSiteId),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanPending {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub reason: CommonLoanPendingReason,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanInput {
    pub owner: ProducerCfgOwnerId,
    pub outputs: Vec<CommonLoanOutput>,
    pub binding_shapes: Vec<SourceBindingShapeFact>,
    pub final_inputs: Vec<FinalOutputInput>,
    pub roles: Vec<CommonLoanRole>,
    pub sites: Vec<CommonLoanSite>,
    pub events: Vec<CommonLoanEvent>,
    pub pending: Vec<CommonLoanPending>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanOutput {
    pub value: EvaluatedValueId,
    pub shape: ResolvedValueShape,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CommonLoanMembership {
    pub role: CommonLoanRoleId,
    pub site: CommonLoanSiteId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanSubset {
    pub source: CommonLoanRoleId,
    pub target: CommonLoanRoleId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanPoint {
    pub event: SourceEventId,
    pub incoming: Vec<CommonLoanMembership>,
    pub outgoing: Vec<CommonLoanMembership>,
    pub demand_in: Vec<CommonLoanRoleId>,
    pub demand_out: Vec<CommonLoanRoleId>,
    pub issued: Vec<CommonLoanSiteId>,
    pub may_live: Vec<CommonLoanSiteId>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanCandidateReason {
    PossibleConflict,
    EscapeOutlives,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanCandidate {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub role: CommonLoanRoleId,
    pub mode: ScalarReferenceMutability,
    pub target: CommonLoanTarget,
    pub sites: Vec<CommonLoanSiteId>,
    pub reason: CommonLoanCandidateReason,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonLoanAnalysis {
    pub input: CommonLoanInput,
    pub subset: Vec<CommonLoanSubset>,
    pub points: Vec<CommonLoanPoint>,
    pub candidates: Vec<CommonLoanCandidate>,
    pub pending: Vec<CommonLoanPending>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommonLoanQuestion {
    CoveredNoCandidates,
    PendingValidation,
}

impl CommonLoanAnalysis {
    pub fn common_question(&self) -> CommonLoanQuestion {
        if self.pending.is_empty() && self.candidates.is_empty() {
            CommonLoanQuestion::CoveredNoCandidates
        } else {
            CommonLoanQuestion::PendingValidation
        }
    }
}

fn invalid(source: &SourceSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: source.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

pub(super) fn register(cfg: &mut ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    let input = extraction::extract(
        &cfg.evaluated_values,
        &cfg.producer_owner,
        &cfg.initial_points,
    )?;
    let analysis = solver::solve(input);
    cfg.evaluated_values
        .common_loans
        .retain(|old| old.input.owner != cfg.producer_owner);
    cfg.evaluated_values.common_loans.push(analysis);
    Ok(())
}

pub(super) fn validate_epoch(
    cfg: &ScalarReferenceCfg,
    analysis: &CommonLoanAnalysis,
) -> Result<(), ReferenceAnalysisError> {
    if analysis.input.owner != cfg.producer_owner
        || analysis.input
            != extraction::extract(
                &cfg.evaluated_values,
                &cfg.producer_owner,
                &cfg.initial_points,
            )?
    {
        return Err(invalid(&cfg.initial_points[0].source_span));
    }
    Ok(())
}

pub(super) fn require_epoch<'a>(
    tables: &'a EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    points: &[ReferenceFlowPoint],
    event: &SourceEventId,
) -> Result<&'a CommonLoanAnalysis, ReferenceAnalysisError> {
    let point = points
        .get(event.point.0)
        .ok_or_else(|| invalid(&points[0].source_span))?;
    let analyses = tables
        .common_loans
        .iter()
        .filter(|analysis| analysis.input.owner == *owner)
        .collect::<Vec<_>>();
    let [analysis] = analyses.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    if event.owner != *owner
        || !matches!(point.kind, CfgPointKind::ReturnOutput { .. })
        || analysis.input != extraction::extract(tables, owner, points)?
        || !analysis.input.events.iter().any(|row| {
            row.event == *event
                && row.source_span == point.source_span
                && row.kind == source_epoch::SourceEventKind::ReturnOutput
        })
    {
        return Err(invalid(&point.source_span));
    }
    Ok(analysis)
}
