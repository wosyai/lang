use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedExpressionOutputFact {
    pub source_span: SourceSpan,
    pub outputs: ScalarOutputSequence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedAssignmentTypeFact {
    pub occurrence: ScalarAssignmentOccurrence,
    pub replacements: ScalarOutputSequence,
    pub previous: ScalarOutputSequence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EvaluatedOutputSequence {
    pub entries: Vec<EvaluatedOutputEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EvaluatedOutputEntry {
    pub value: EvaluatedValueId,
    pub shape: ValueShapeId,
}

pub(super) struct EvaluatedAssignmentSelectors {
    pub root: EvaluatedAssignmentRoot,
    pub indexes: Vec<EvaluatedValueId>,
}

pub(super) enum EvaluatedAssignmentRoot {
    Direct,
    Through(EvaluatedOutputEntry),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourceValueTypeFacts {
    pub assignments: Vec<ResolvedAssignmentTypeFact>,
    pub expressions: Vec<ResolvedExpressionOutputFact>,
}

impl SourceValueTypeFacts {
    pub(super) fn selected_output(
        &self,
        source_span: &SourceSpan,
        output: usize,
    ) -> Result<&ScalarType, SourceSpan> {
        let mut facts = self
            .expressions
            .iter()
            .filter(|fact| fact.source_span == *source_span);
        let fact = facts.next().ok_or_else(|| source_span.clone())?;
        if facts.next().is_some() {
            return Err(source_span.clone());
        }
        fact.outputs
            .outputs
            .get(output)
            .map(|output| &output.ty)
            .ok_or_else(|| source_span.clone())
    }
}

pub(super) fn record_assignment_type_fact(
    facts: &RefCell<Vec<ResolvedAssignmentTypeFact>>,
    assignment: &ScalarAssignment,
    replacements: &ScalarOutputSequence,
    previous: &ScalarOutputSequence,
) {
    let fact = ResolvedAssignmentTypeFact {
        occurrence: (*assignment.occurrence).clone(),
        replacements: replacements.clone(),
        previous: previous.clone(),
    };
    let mut facts = facts.borrow_mut();
    if let Some(known) = facts
        .iter_mut()
        .find(|known| known.occurrence == fact.occurrence)
    {
        *known = fact;
    } else {
        facts.push(fact);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentOperationId {
    pub producer: ProducerCfgOwnerId,
    pub occurrence: ScalarAssignmentOccurrence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedAssignmentPlan {
    pub operation: AssignmentOperationId,
    pub targets: Vec<ResolvedAssignmentTarget>,
    pub replacements: Vec<AssignmentReplacementProjection>,
    pub previous_outputs: Vec<AssignmentPreviousPlan>,
    pub consumption: AssignmentConsumption,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedAssignmentTarget {
    pub ordinal: usize,
    pub source_span: SourceSpan,
    pub place: ScalarPlace,
    pub ty: ScalarType,
    pub storage: ResolvedAssignmentStorage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ResolvedAssignmentStorage {
    Existing(ReferencePlaceId),
    Through { pointer: ResolvedValueOutputId },
    InferredDeclaration(ReferenceBindingId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentReplacementProjection {
    pub expression_ordinal: usize,
    pub producer_output: usize,
    pub destination_ordinal: usize,
    pub source_type: ScalarType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssignmentConsumption {
    Received {
        outputs: Vec<ValueConsumerId>,
        discards: Vec<ResolvedValueOutputId>,
    },
    Discarded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAssignmentEvaluation {
    pub operation: AssignmentOperationId,
    pub rhs: Vec<AssignmentRhsEvaluation>,
    pub destinations: Vec<EvaluatedAssignmentDestination>,
    pub previous: Vec<AssignmentSnapshot>,
    pub writes: Vec<AssignmentCommit>,
    pub results: Vec<AssignmentResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentRhsEvaluation {
    pub expression_ordinal: usize,
    pub producer_output: usize,
    pub value: EvaluatedValueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentPreviousPlan {
    pub target: usize,
    pub previous: PreviousOutputPlan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PreviousOutputPlan {
    Existing {
        output: ResolvedValueOutputId,
        shape: ValueShapeId,
    },
    InferredDeclaration {
        binding: ReferenceBindingId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentDestinationId {
    pub operation: AssignmentOperationId,
    pub target: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedAssignmentDestination {
    pub id: AssignmentDestinationId,
    pub point: CfgPointId,
    pub source_span: SourceSpan,
    pub shape: ValueShapeId,
    pub storage: EvaluatedDestinationStorage,
    pub selector_inputs: Vec<EvaluatedValueId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EvaluatedDestinationStorage {
    Direct(ReferencePlaceId),
    Through {
        pointer: EvaluatedValueId,
        cases: Vec<DestinationCase>,
    },
    Inferred(ReferenceBindingId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DestinationCase {
    pub case: ValueCaseId,
    pub place: ReferencePlaceId,
    pub access: ReferenceBorrowAccess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentSnapshot {
    pub destination: AssignmentDestinationId,
    pub point: CfgPointId,
    pub state: SnapshotState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SnapshotState {
    Saved(EvaluatedValueId),
    ElidedDiscard { cases: Vec<EvaluatedValueCase> },
    NoPriorDeclaration,
    Unavailable { cases: Vec<EvaluatedValueCase> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentCommit {
    pub destination: AssignmentDestinationId,
    pub point: CfgPointId,
    pub replacement: EvaluatedValueId,
    pub previous: CfgPointId,
    pub dispositions: Vec<ValueDisposition>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentResult {
    pub operation: AssignmentOperationId,
    pub output: usize,
    pub snapshot: CfgPointId,
    pub value: EvaluatedValueId,
    pub consumers: Vec<ValueConsumerId>,
}

pub mod correspondence;
pub use correspondence::validate_assignment_destinations;
