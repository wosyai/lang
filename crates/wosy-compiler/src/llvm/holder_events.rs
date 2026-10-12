use super::assignment_snapshot::{SavedAssignmentOutput, SavedAssignmentRuntimeLength};
use super::*;
use crate::scalar::{
    AssignmentConsumption, EvaluatedValueFact, EvaluatedValueId, PreviousOutputPlan,
    ResolvedAssignmentPlan, ResolvedValueShape, ScalarAssignmentEvaluation, SnapshotState,
    ValueShapeId,
};
use crate::scalar::{
    CallableEnvironmentId, CallableHolderAction, CallableHolderEdgeFact, CallableHolderExitFact,
    CallableHolderPlace, CfgPointId, ReferenceBindingId, ReferenceBindingKind, ReferenceFlowPoint,
    ReferencePlaceProjection, ScalarAssignment, ScalarAssignmentPointRole, ScalarBinding,
    ScalarBindingPointRole, ScalarBlockOccurrence, ScalarCallOccurrence, ScalarCaptureOccurrence,
    ScalarCapturePointRole, ScalarConditionalPointRole, ScalarReadRole, ScalarReferenceCfg,
    ScalarTopLevelBindingExecution, ScalarTopLevelOccurrence, ScalarTopLevelPointExecution,
    ScalarType, ScalarWhileOccurrence, ScalarWhilePointRole,
};
use std::cell::Cell;
use std::collections::HashMap;

pub(super) struct SelectedElementSlots<'ctx> {
    pub index_point: CfgPointId,
    pub index: PointerValue<'ctx>,
    pub address: PointerValue<'ctx>,
    pub present: PointerValue<'ctx>,
}

pub(super) struct ReadPlaceSlots<'ctx> {
    pub callable: PointerValue<'ctx>,
    pub address: PointerValue<'ctx>,
    pub present: PointerValue<'ctx>,
}

pub(super) struct ReturnOutputSlots<'ctx> {
    pub address: PointerValue<'ctx>,
    pub present: PointerValue<'ctx>,
    pub ty: ScalarType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HolderEmissionEdge {
    Outgoing { from: CfgPointId, to: CfgPointId },
    Terminal { from: CfgPointId },
}

impl HolderEmissionEdge {
    pub(super) fn from(self) -> CfgPointId {
        match self {
            Self::Outgoing { from, .. } | Self::Terminal { from } => from,
        }
    }

    pub(super) fn to(self) -> Option<CfgPointId> {
        match self {
            Self::Outgoing { to, .. } => Some(to),
            Self::Terminal { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum HolderPointKind {
    Capture,
    Call,
    Bind,
    BindArrayElement,
    Assign,
    CopyCallableFields,
    ReturnOutput,
    ScopeExit,
    Branch,
    LoopTest,
    ReadPlace,
}

impl HolderPointKind {
    fn of(kind: &CfgPointKind) -> Option<Self> {
        match kind {
            CfgPointKind::Capture { .. } => Some(Self::Capture),
            CfgPointKind::Call { .. } => Some(Self::Call),
            CfgPointKind::Bind { .. } => Some(Self::Bind),
            CfgPointKind::BindArrayElement { .. } => Some(Self::BindArrayElement),
            CfgPointKind::Assign { .. } => Some(Self::Assign),
            CfgPointKind::CopyCallableFields { .. } => Some(Self::CopyCallableFields),
            CfgPointKind::ReturnOutput { .. } => Some(Self::ReturnOutput),
            CfgPointKind::ScopeExit => Some(Self::ScopeExit),
            CfgPointKind::Branch { .. } => Some(Self::Branch),
            CfgPointKind::LoopTest { .. } => Some(Self::LoopTest),
            CfgPointKind::ReadPlace { .. } => Some(Self::ReadPlace),
            _ => None,
        }
    }
}

pub(super) struct HolderEvents<'ctx> {
    cfg: ScalarReferenceCfg,
    points: HashMap<(wosy_syntax::SourceSpan, HolderPointKind), Vec<CfgPointId>>,
    edges: HashMap<(CfgPointId, CfgPointId), Vec<CallableHolderEdgeFact>>,
    exits: HashMap<CfgPointId, Vec<CallableHolderExitFact>>,
    pub selected: HashMap<CfgPointId, SelectedElementSlots<'ctx>>,
    pub reads: HashMap<CfgPointId, ReadPlaceSlots<'ctx>>,
    pub outputs: HashMap<CfgPointId, ReturnOutputSlots<'ctx>>,
    pub bindings: HashMap<ReferenceBindingId, PointerValue<'ctx>>,
    pub imported_holders:
        HashMap<super::ownership::ImportedHolderKey, super::ownership::ImportedCaptureHolder<'ctx>>,
    last_evaluated_call: Cell<Option<usize>>,
    pub covered_cells: HashMap<ReferenceBindingId, CaptureCoverage>,
    saved_assignment_outputs: Vec<SavedAssignmentOutput<'ctx>>,
    saved_assignment_runtime_lengths: Vec<SavedAssignmentRuntimeLength<'ctx>>,
}

pub(super) struct CaptureCoverage {
    pub backing: crate::scalar::RuntimeBackingFact,
    pub backings: Vec<crate::scalar::RuntimeBackingFact>,
    pub environments: Vec<CallableEnvironmentId>,
    pub owner_edges: Vec<HolderEmissionEdge>,
    pub actions: Vec<(HolderEmissionEdge, CallableHolderAction)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CaptureDescriptor {
    pub literal: wosy_syntax::SourceSpan,
    pub signature: ScalarType,
    pub cells: Vec<CaptureDescriptorCell>,
    pub slot_width: u64,
    pub payload_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CaptureDescriptorCell {
    pub binding: ReferenceBindingId,
    pub ty: ScalarType,
    pub cell_offset: u64,
    pub length_offset: Option<u64>,
    pub residence: super::ownership::CaptureCellResidence,
    pub backings: Vec<crate::scalar::RuntimeBackingFact>,
    pub owner_exits: Vec<HolderEmissionEdge>,
    pub replacements: Vec<(
        HolderEmissionEdge,
        crate::scalar::RuntimeBackingFact,
        crate::scalar::RuntimeBackingFact,
    )>,
    pub initialized_origins: Vec<CallableEnvironmentId>,
    pub initialized_cases: Vec<crate::scalar::CallableCellCaseFact>,
}

enum ScalarCallEmissionOutput<'a> {
    Projected {
        output: usize,
        ty: &'a ScalarType,
        point: &'a ReferenceFlowPoint,
    },
    SignatureOnly {
        output: usize,
        ty: &'a ScalarType,
    },
}

impl<'ctx> HolderEvents<'ctx> {
    pub(super) fn assignment_value(
        &self,
        id: &EvaluatedValueId,
    ) -> Result<(&EvaluatedValueFact, &ResolvedValueShape), String> {
        let matches = self
            .cfg
            .evaluated_values
            .values
            .iter()
            .filter(|fact| fact.id == *id)
            .collect::<Vec<_>>();
        let [fact] = matches.as_slice() else {
            return Err(format!(
                "expected one actual assignment value {id:?}, found {}",
                matches.len()
            ));
        };
        self.cfg_point(id.point)?;
        if id.resolved.producer != self.cfg.producer_owner {
            return Err(format!(
                "assignment value {id:?} belongs to a different CFG producer"
            ));
        }
        Ok((fact, self.assignment_shape(&fact.shape)?))
    }

    fn assignment_shape(&self, id: &ValueShapeId) -> Result<&ResolvedValueShape, String> {
        let matches = self
            .cfg
            .evaluated_values
            .shapes
            .iter()
            .filter(|shape| shape.id == *id)
            .collect::<Vec<_>>();
        let [shape] = matches.as_slice() else {
            return Err(format!(
                "expected one actual assignment shape {id:?}, found {}",
                matches.len()
            ));
        };
        if id.producer != self.cfg.producer_owner {
            return Err(format!(
                "assignment shape {id:?} belongs to a different CFG producer"
            ));
        }
        Ok(shape)
    }

    pub(super) fn assignment_evaluation(
        &self,
        assignment: &ScalarAssignment,
    ) -> Result<(ResolvedAssignmentPlan, ScalarAssignmentEvaluation), String> {
        let operation = crate::scalar::AssignmentOperationId {
            producer: self.cfg.producer_owner.clone(),
            occurrence: assignment.occurrence.as_ref().clone(),
        };
        let plans = self
            .cfg
            .evaluated_values
            .assignment_plans
            .iter()
            .filter(|plan| plan.operation == operation)
            .collect::<Vec<_>>();
        let evaluations = self
            .cfg
            .evaluated_values
            .assignment_evaluations
            .iter()
            .filter(|evaluation| evaluation.operation == operation)
            .collect::<Vec<_>>();
        let ([plan], [evaluation]) = (plans.as_slice(), evaluations.as_slice()) else {
            return Err(format!(
                "expected one assignment plan/evaluation for {operation:?}, found {}/{}",
                plans.len(),
                evaluations.len()
            ));
        };
        let count = assignment.targets.len();
        if plan.targets.len() != count
            || plan.replacements.len() != count
            || plan.previous_outputs.len() != count
            || evaluation.rhs.len() != count
            || evaluation.destinations.len() != count
            || evaluation.previous.len() != count
            || evaluation.writes.len() != count
        {
            return Err(format!(
                "assignment target correspondence differs for {operation:?}"
            ));
        }
        for position in 0..count {
            let target = &assignment.targets[position];
            let resolved = &plan.targets[position];
            let destination = &evaluation.destinations[position];
            let previous = &evaluation.previous[position];
            let write = &evaluation.writes[position];
            let replacement = &plan.replacements[position];
            let rhs = &evaluation.rhs[position];
            if resolved.ordinal != position
                || resolved.place != target.place
                || resolved.source_span
                    != wosy_syntax::SourceSpan::new(
                        operation.occurrence.source_span.source.clone(),
                        target.target_span,
                    )
                || destination.id.operation != operation
                || destination.id.target != position
                || destination.source_span != resolved.source_span
                || self.assignment_shape(&destination.shape)?.ty != resolved.ty
                || previous.destination != destination.id
                || plan.previous_outputs[position].target != position
                || write.destination != destination.id
                || write.previous != previous.point
                || write.replacement != rhs.value
                || replacement.destination_ordinal != position
                || replacement.expression_ordinal != rhs.expression_ordinal
                || replacement.producer_output != rhs.producer_output
                || self.assignment_value(&rhs.value)?.1.ty != replacement.source_type
            {
                return Err(format!("invalid assignment destination/previous/commit row {position} for {operation:?}"));
            }
            let point = self.cfg_point(destination.point)?;
            if !matches!(&point.kind, CfgPointKind::AssignmentDestination { storage, place }
                if storage == &destination.storage && place == &target.place)
            {
                return Err(format!(
                    "invalid actual destination point {:?} for {operation:?}",
                    destination.point
                ));
            }
            let commit = self.cfg_point(write.point)?;
            if !matches!(&commit.kind,
                CfgPointKind::Assign { target: place, rhs_point, .. }
                    if place.place == target.place && *rhs_point == rhs.value.point)
                && !matches!(&commit.kind,
                    CfgPointKind::AssignThrough { place, rhs_point, .. }
                        if place == &target.place && *rhs_point == rhs.value.point)
            {
                return Err(format!(
                    "invalid actual commit point {:?} for {operation:?}",
                    write.point
                ));
            }
            match (&previous.state, &plan.previous_outputs[position].previous) {
                (SnapshotState::Saved(id), PreviousOutputPlan::Existing { output, shape }) => {
                    let (fact, actual_shape) = self.assignment_value(id)?;
                    if id.point != previous.point
                        || id.resolved != *output
                        || fact.shape != *shape
                        || actual_shape.id != destination.shape
                        || !matches!(&self.cfg_point(previous.point)?.kind, CfgPointKind::AssignmentSnapshot { destination_point, .. }
                            if *destination_point == destination.point)
                    {
                        return Err(format!(
                            "invalid saved snapshot row {position} for {operation:?}"
                        ));
                    }
                }
                (
                    SnapshotState::NoPriorDeclaration,
                    PreviousOutputPlan::InferredDeclaration { binding },
                ) => {
                    if !matches!(&destination.storage, crate::scalar::EvaluatedDestinationStorage::Inferred(actual) if actual == binding)
                        || previous.point != destination.point
                    {
                        return Err(format!(
                            "invalid inferred snapshot row {position} for {operation:?}"
                        ));
                    }
                }
                (SnapshotState::ElidedDiscard { .. }, PreviousOutputPlan::Existing { .. })
                | (SnapshotState::Unavailable { .. }, PreviousOutputPlan::Existing { .. }) => {
                    self.cfg_point(previous.point)?;
                }
                _ => {
                    return Err(format!(
                        "inconsistent snapshot category row {position} for {operation:?}"
                    ))
                }
            }
        }
        if matches!(plan.consumption, AssignmentConsumption::Discarded)
            && !evaluation.results.is_empty()
        {
            return Err(format!(
                "discarded assignment {operation:?} has observed result rows"
            ));
        }
        for (output, result) in evaluation.results.iter().enumerate() {
            let snapshot = evaluation
                .previous
                .get(result.output)
                .ok_or_else(|| format!("invalid result destination {result:?}"))?;
            let SnapshotState::Saved(saved) = &snapshot.state else {
                return Err(format!(
                    "assignment result lacks actual saved snapshot {result:?}"
                ));
            };
            let (saved_fact, _) = self.assignment_value(saved)?;
            let (fact, _) = self.assignment_value(&result.value)?;
            if result.operation != operation
                || result.output != output
                || result.snapshot != snapshot.point
                || result.value.resolved != saved.resolved
                || fact.shape != saved_fact.shape
                || result.value.point == saved.point
                || !matches!(&self.cfg_point(result.value.point)?.kind, CfgPointKind::AssignmentResult { snapshot_point, output: ordinal }
                    if *snapshot_point == snapshot.point && *ordinal == result.output)
            {
                return Err(format!(
                    "invalid assignment snapshot/result correspondence {result:?}"
                ));
            }
        }
        if matches!(plan.consumption, AssignmentConsumption::Received { .. }) {
            let saved_count = evaluation
                .previous
                .iter()
                .filter(|snapshot| matches!(snapshot.state, SnapshotState::Saved(_)))
                .count();
            if evaluation.results.len() != saved_count {
                return Err(format!(
                    "assignment {operation:?} has missing actual saved result rows"
                ));
            }
        }
        Ok(((**plan).clone(), (**evaluation).clone()))
    }

    pub(super) fn assignment_result_shape(
        &self,
        assignment: &ScalarAssignment,
    ) -> Option<&ResolvedValueShape> {
        let operation = crate::scalar::AssignmentOperationId {
            producer: self.cfg.producer_owner.clone(),
            occurrence: assignment.occurrence.as_ref().clone(),
        };
        let plans = self
            .cfg
            .evaluated_values
            .assignment_plans
            .iter()
            .filter(|plan| plan.operation == operation)
            .collect::<Vec<_>>();
        let evaluations = self
            .cfg
            .evaluated_values
            .assignment_evaluations
            .iter()
            .filter(|evaluation| evaluation.operation == operation)
            .collect::<Vec<_>>();
        let ([plan], [evaluation]) = (plans.as_slice(), evaluations.as_slice()) else {
            return None;
        };
        if !matches!(plan.consumption, AssignmentConsumption::Received { .. }) {
            return None;
        }
        let results = evaluation
            .results
            .iter()
            .filter(|result| result.output == 0)
            .collect::<Vec<_>>();
        let [result] = results.as_slice() else {
            return None;
        };
        if result.operation != operation
            || result.value.resolved.producer != self.cfg.producer_owner
            || result.value.resolved.output != 0
            || result.value.resolved.output_projection != assignment.occurrence.output_projection
            || result.value.resolved.occurrence
                != wosy_syntax::SourceSpan::new(
                    assignment.occurrence.source_span.source.clone(),
                    assignment.span,
                )
        {
            return None;
        }
        let point = self.cfg.points.get(result.value.point.0)?;
        if point.id != result.value.point
            || !matches!(&point.kind, CfgPointKind::AssignmentResult { snapshot_point, output }
                if *snapshot_point == result.snapshot && *output == 0)
        {
            return None;
        }
        let facts = self
            .cfg
            .evaluated_values
            .values
            .iter()
            .filter(|fact| fact.id == result.value)
            .collect::<Vec<_>>();
        let [fact] = facts.as_slice() else {
            return None;
        };
        if fact.shape.producer != self.cfg.producer_owner {
            return None;
        }
        let shapes = self
            .cfg
            .evaluated_values
            .shapes
            .iter()
            .filter(|shape| shape.id == fact.shape)
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return None;
        };
        Some(shape)
    }

    pub(super) fn emitted_assignment_result_shape(
        &self,
        assignment: &ScalarAssignment,
    ) -> Result<&ResolvedValueShape, String> {
        let (_, evaluation) = self.assignment_evaluation(assignment)?;
        let results = evaluation
            .results
            .iter()
            .filter(|result| result.output == 0)
            .collect::<Vec<_>>();
        let [result] = results.as_slice() else {
            return Err(format!(
                "expected one emitted saved-pointer result for {:?}, found {}",
                evaluation.operation,
                results.len()
            ));
        };
        let (fact, shape) = self.assignment_value(&result.value)?;
        let static_shape = self.assignment_result_shape(assignment).ok_or_else(|| {
            format!(
                "missing unique static saved-pointer shape for {:?}",
                evaluation.operation
            )
        })?;
        let saved = self.saved_assignment_output(&result.value)?;
        if saved.result != **result
            || saved.fact != *fact
            || saved.shape != *shape
            || saved.shape != *static_shape
            || saved.snapshot.point != result.snapshot
        {
            return Err(format!(
                "emitted saved-pointer shape/ID differs for {:?}",
                result.value
            ));
        }
        Ok(&saved.shape)
    }

    pub(super) fn record_saved_assignment_output(
        &mut self,
        output: SavedAssignmentOutput<'ctx>,
    ) -> Result<(), String> {
        let (fact, shape) = self.assignment_value(&output.result.value)?;
        let SnapshotState::Saved(snapshot) = &output.snapshot.state else {
            return Err(format!(
                "emitted result has no saved snapshot {:?}",
                output.result
            ));
        };
        if output.fact != *fact
            || output.shape != *shape
            || output.snapshot.point != output.result.snapshot
            || snapshot.point != output.snapshot.point
            || self.assignment_value(snapshot)?.1.id != output.shape.id
        {
            return Err(format!(
                "emitted saved result differs from actual facts {:?}",
                output.result
            ));
        }
        if self
            .saved_assignment_outputs
            .iter()
            .any(|known| known.result.value == output.result.value)
        {
            return Err(format!(
                "duplicate emitted saved assignment result {:?}",
                output.result.value
            ));
        }
        self.saved_assignment_outputs.push(output);
        Ok(())
    }

    pub(super) fn saved_assignment_output(
        &self,
        id: &EvaluatedValueId,
    ) -> Result<&SavedAssignmentOutput<'ctx>, String> {
        self.saved_assignment_outputs
            .iter()
            .find(|output| output.result.value == *id)
            .ok_or_else(|| format!("missing emitted saved assignment result {id:?}"))
    }

    fn validate_saved_assignment_runtime_length(
        &self,
        row: &SavedAssignmentRuntimeLength<'ctx>,
    ) -> Result<(), String> {
        let evaluations = self
            .cfg
            .evaluated_values
            .assignment_evaluations
            .iter()
            .filter(|evaluation| evaluation.operation == row.destination.operation)
            .collect::<Vec<_>>();
        let [evaluation] = evaluations.as_slice() else {
            return Err(format!(
                "missing unique evaluated-length operation for {:?}",
                row.result
            ));
        };
        let destination = evaluation
            .destinations
            .get(row.destination.target)
            .ok_or_else(|| format!("missing evaluated-length destination for {:?}", row.result))?;
        let snapshot = evaluation
            .previous
            .get(row.destination.target)
            .ok_or_else(|| format!("missing evaluated-length snapshot for {:?}", row.result))?;
        let results = evaluation
            .results
            .iter()
            .filter(|result| result.value == row.result)
            .collect::<Vec<_>>();
        let [result] = results.as_slice() else {
            return Err(format!(
                "missing unique evaluated-length result {:?}",
                row.result
            ));
        };
        let (fact, shape) = self.assignment_value(&row.result)?;
        let (snapshot_fact, snapshot_shape) = self.assignment_value(&row.snapshot)?;
        let crate::scalar::EvaluatedDestinationStorage::Direct(place) = &row.storage else {
            return Err(format!(
                "missing evaluated-length for unsupported storage at {:?}",
                row.snapshot
            ));
        };
        if !place.projections.is_empty()
            || !matches!(place.place, crate::ScalarPlace::Name { .. })
            || place.source != destination.source_span.source
            || place.binding.source != place.source
            || place.binding.function_span != place.function_span
            || place.binding.declaration_span != place.declaration_span
            || place.binding.block_span != place.block_span
            || !matches!(&self.cfg_point(destination.point)?.kind,
                CfgPointKind::AssignmentDestination { storage, place: actual }
                    if *storage == row.storage && *actual == place.place)
            || !matches!(&self.cfg_point(snapshot.point)?.kind,
                CfgPointKind::AssignmentSnapshot { destination_point, .. }
                    if *destination_point == destination.point)
            || !matches!(&self.cfg_point(row.result.point)?.kind,
                CfgPointKind::AssignmentResult { snapshot_point, output }
                    if *snapshot_point == snapshot.point && *output == result.output)
        {
            return Err(format!(
                "invalid evaluated-length qualified direct source incidence for {:?}",
                row.result
            ));
        }
        if destination.id != row.destination
            || destination.storage != row.storage
            || snapshot.destination != row.destination
            || !matches!(&snapshot.state, SnapshotState::Saved(id) if *id == row.snapshot)
            || snapshot.point != result.snapshot
            || row.snapshot.point != snapshot.point
            || row.result == row.snapshot
            || result.operation != row.destination.operation
            || evaluation.results.get(result.output) != Some(*result)
            || destination.shape != shape.id
            || snapshot_shape != shape
            || snapshot_fact.shape != fact.shape
            || !matches!(shape.ty, ScalarType::RuntimeArray { .. })
            || !matches!(
                shape.payload,
                crate::scalar::ValuePayloadShape::RuntimeArray { .. }
            )
            || row.length.get_type().get_bit_width() != 64
        {
            return Err(format!("invalid evaluated-length snapshot/result/destination/shape/storage/i64 incidence for {:?}", row.result));
        }
        Ok(())
    }

    pub(super) fn validate_saved_assignment_runtime_lengths(
        &self,
        rows: &[SavedAssignmentRuntimeLength<'ctx>],
    ) -> Result<(), String> {
        for (position, row) in rows.iter().enumerate() {
            self.validate_saved_assignment_runtime_length(row)?;
            if self
                .saved_assignment_runtime_lengths
                .iter()
                .chain(&rows[..position])
                .any(|known| known.result == row.result || known.snapshot == row.snapshot)
            {
                return Err(format!(
                    "duplicate emitted saved assignment runtime length {:?}",
                    row.result
                ));
            }
        }
        Ok(())
    }

    pub(super) fn record_saved_assignment_runtime_lengths(
        &mut self,
        rows: Vec<SavedAssignmentRuntimeLength<'ctx>>,
    ) -> Result<(), String> {
        self.validate_saved_assignment_runtime_lengths(&rows)?;
        self.saved_assignment_runtime_lengths.extend(rows);
        Ok(())
    }

    pub(super) fn saved_assignment_runtime_length(
        &self,
        id: &EvaluatedValueId,
    ) -> Result<&SavedAssignmentRuntimeLength<'ctx>, String> {
        let rows = self
            .saved_assignment_runtime_lengths
            .iter()
            .filter(|row| row.result == *id)
            .collect::<Vec<_>>();
        let [row] = rows.as_slice() else {
            return Err(format!(
                "missing unique emitted saved assignment evaluated-length {id:?}"
            ));
        };
        self.validate_saved_assignment_runtime_length(row)?;
        let saved = self.saved_assignment_output(id)?;
        let (fact, shape) = self.assignment_value(id)?;
        if saved.fact != *fact
            || saved.shape != *shape
            || saved.result.value != row.result
            || saved.snapshot.destination != row.destination
            || saved.snapshot.point != row.snapshot.point
            || !self
                .cfg
                .evaluated_values
                .assignment_evaluations
                .iter()
                .filter(|evaluation| evaluation.operation == row.destination.operation)
                .any(|evaluation| {
                    evaluation.results.contains(&saved.result)
                        && evaluation.previous.contains(&saved.snapshot)
                })
            || !matches!(&saved.snapshot.state, SnapshotState::Saved(snapshot) if *snapshot == row.snapshot)
        {
            return Err(format!(
                "emitted saved payload/evaluated-length incidence differs for {id:?}"
            ));
        }
        Ok(row)
    }

    pub(super) fn assignment_runtime_length(
        &self,
        assignment: &ScalarAssignment,
        output: usize,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let (_, evaluation) = self.assignment_evaluation(assignment)?;
        let result = evaluation
            .results
            .get(output)
            .filter(|result| result.output == output && result.operation == evaluation.operation)
            .ok_or_else(|| {
                format!(
                    "missing saved evaluated-length output {output} for {:?}",
                    evaluation.operation
                )
            })?;
        let row = self.saved_assignment_runtime_length(&result.value)?;
        Ok(row.length)
    }

    pub(super) fn cfg_point(&self, id: CfgPointId) -> Result<&ReferenceFlowPoint, String> {
        self.cfg
            .points
            .get(id.0)
            .filter(|point| point.id == id)
            .ok_or_else(|| format!("missing source-qualified holder point {id:?}"))
    }
    fn installed_index_key(
        &self,
        key: &crate::scalar::CallableIndexKeyFact,
        span: &wosy_syntax::SourceSpan,
        binding: &ReferenceBindingId,
    ) -> Result<(), String> {
        match key {
            crate::scalar::CallableIndexKeyFact::Constant(_) => Ok(()),
            crate::scalar::CallableIndexKeyFact::LivePlace(place) => {
                if place.source != binding.source || place.function_span != binding.function_span {
                    return Err(format!(
                        "Capture at {span:?} has an unrelated installed index place for {binding:?}"
                    ));
                }
                Ok(())
            }
            crate::scalar::CallableIndexKeyFact::Evaluated {
                point,
                occurrence,
                function_span,
            } => {
                self.cfg
                    .points
                    .get(point.0)
                    .filter(|candidate| candidate.id == *point)
                    .ok_or_else(|| {
                        format!(
                            "Capture at {span:?} lacks connected installed index for {binding:?}"
                        )
                    })?;
                if occurrence.source != binding.source || *function_span != self.function_span() {
                    return Err(format!(
                        "Capture at {span:?} has an installed index from another function for {binding:?}"
                    ));
                }
                Ok(())
            }
        }
    }
    pub(super) fn capture_descriptor(
        &self,
        point: &ReferenceFlowPoint,
        target: crate::scalar::ScalarTargetLayout,
        storage: &BTreeMap<ReferenceBindingId, super::ownership::CaptureCellResidence>,
    ) -> Result<Option<CaptureDescriptor>, String> {
        let CfgPointKind::Capture {
            literal,
            environment,
            signature,
            captures,
            ..
        } = &point.kind
        else {
            return Err(format!(
                "expected source-qualified Capture at {:?}",
                point.source_span
            ));
        };
        if point.id.0 >= self.cfg.points.len() || self.cfg.points[point.id.0] != *point {
            return Err(format!("unconnected Capture at {:?}", point.source_span));
        }
        let fact = self
            .cfg
            .environments
            .get(environment.index)
            .filter(|fact| fact.literal == point.source_span)
            .ok_or_else(|| {
                format!(
                    "Capture at {:?} lacks validated environment",
                    point.source_span
                )
            })?;
        if *literal != point.source_span.range || !matches!(signature, ScalarType::Callable { .. })
        {
            return Err(format!(
                "Capture at {:?} has inconsistent signature or source",
                point.source_span
            ));
        }
        if captures.is_empty() {
            return Ok(None);
        }
        let mut distinct = BTreeMap::new();
        for capture in captures {
            if capture.binding.source != point.source_span.source {
                return Err(format!(
                    "Capture at {:?} has foreign binding {:?}",
                    point.source_span, capture.binding
                ));
            }
            match distinct.entry(capture.binding.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(capture);
                }
                std::collections::btree_map::Entry::Occupied(entry)
                    if entry.get().ty == capture.ty && entry.get().name == capture.name => {}
                _ => {
                    return Err(format!(
                        "Capture at {:?} has conflicting binding {:?}",
                        point.source_span, capture.binding
                    ))
                }
            }
        }
        if target.pointer_size != target.pointer_alignment || !matches!(target.pointer_size, 4 | 8)
        {
            return Err(format!(
                "Capture at {:?} has unsupported pointer layout",
                point.source_span
            ));
        }
        let wide = distinct
            .values()
            .any(|capture| matches!(capture.ty, ScalarType::RuntimeArray { .. }));
        let slot_width = if wide { 8 } else { target.pointer_size };
        let mut offset = 0u64;
        let mut cells = Vec::new();
        for (binding, capture) in distinct {
            let inputs = fact
                .inputs
                .iter()
                .filter(|input| input.binding == binding)
                .collect::<Vec<_>>();
            let [input] = inputs.as_slice() else {
                return Err(format!(
                    "Capture at {:?} lacks unique validated backing/owner for {:?}",
                    point.source_span, binding
                ));
            };
            for id in &input.environments {
                self.cfg
                    .environments
                    .get(id.index)
                    .filter(|fact| fact.literal == id.literal)
                    .ok_or_else(|| {
                        format!(
                            "Capture at {:?} lacks source-qualified nested producer {:?} for {:?}",
                            point.source_span, id.literal, binding
                        )
                    })?;
            }
            for origin in &input.origins {
                if origin.0 >= self.cfg.facts.len() {
                    return Err(format!(
                        "Capture at {:?} lacks connected origin for {:?}",
                        point.source_span, binding
                    ));
                }
            }
            for projected in &input.projected {
                if projected.place.source != binding.source
                    || projected.place.function_span != binding.function_span
                {
                    return Err(format!(
                        "Capture at {:?} has an unrelated projected place for {:?}",
                        point.source_span, binding
                    ));
                }
                for id in &projected.environments {
                    self.cfg
                        .environments
                        .get(id.index)
                        .filter(|fact| fact.literal == id.literal)
                        .ok_or_else(|| {
                            format!(
                                "Capture at {:?} lacks source-qualified projected producer {:?} for {:?}",
                                point.source_span, id.literal, binding
                            )
                        })?;
                }
            }
            for projected in &input.projected_backings {
                if projected.place.source != binding.source
                    || projected.place.function_span != binding.function_span
                {
                    return Err(format!(
                        "Capture at {:?} has an unrelated projected backing place for {:?}",
                        point.source_span, binding
                    ));
                }
            }
            let residence = *storage.get(&binding).ok_or_else(|| {
                format!(
                    "Capture at {:?} lacks classified LLVM storage for {:?}",
                    point.source_span, binding
                )
            })?;
            if residence == super::ownership::CaptureCellResidence::External {
                return Err(format!("Capture at {:?} of {:?} lacks a validated external ownership and trace/release contract", point.source_span, binding));
            }
            let global_binding = self.cfg.points.iter().any(|candidate| {
                matches!(&candidate.kind, CfgPointKind::Bind { binding: known } if known == &binding)
                    && self.cfg.top_level.bindings.iter().any(|entry| {
                        matches!(&entry.execution, ScalarTopLevelBindingExecution::Connected { points } if points.contains(&candidate.id))
                            && self.cfg.top_level.points.iter().any(|record| {
                                record.point == candidate.id
                                    && matches!(&record.execution, ScalarTopLevelPointExecution::Connected { occurrence: ScalarTopLevelOccurrence::Binding(known), .. } if known == &entry.occurrence)
                            })
                    })
            });
            if residence == super::ownership::CaptureCellResidence::Global && !global_binding {
                return Err(format!(
                    "Capture at {:?} lacks typed top-level binding for global storage {:?}",
                    point.source_span, binding
                ));
            }
            if residence == super::ownership::CaptureCellResidence::Promoted && global_binding {
                return Err(format!(
                    "Capture at {:?} classifies typed top-level binding {:?} as promoted storage",
                    point.source_span, binding
                ));
            }
            if residence == super::ownership::CaptureCellResidence::Promoted
                && !self.cfg.points.iter().any(|candidate| {
                    matches!(&candidate.kind,
                    CfgPointKind::Bind { binding: known } if known == &binding)
                })
                && !self.cfg.reference_parameters.contains(&binding)
                && !self.cfg.backing_parameters.contains(&binding)
                && !self.cfg.inputs.iter().any(|input| input.binding == binding)
            {
                return Err(format!(
                    "Capture at {:?} lacks typed local/parameter provenance for {:?}",
                    point.source_span, binding
                ));
            }
            let replacements = self
                .cfg
                .holder_edges
                .iter()
                .flat_map(|edge| {
                    edge.actions.iter().filter_map(|action| match action {
                        CallableHolderAction::ReplaceCaptureCellValue {
                            binding: actual,
                            previous,
                            incoming,
                            ..
                        } if actual == &binding => Some((
                            HolderEmissionEdge::Outgoing {
                                from: edge.from,
                                to: edge.to,
                            },
                            previous.clone(),
                            incoming.clone(),
                        )),
                        _ => None,
                    })
                })
                .collect::<Vec<_>>();
            let owner_exits = self.cfg.holder_edges.iter().flat_map(|edge| {
                edge.actions.iter().filter_map(|action| matches!(action,
                    CallableHolderAction::DropCaptureCellOwner { binding: actual, .. } if actual == &binding)
                    .then_some(HolderEmissionEdge::Outgoing { from: edge.from, to: edge.to }))
            }).chain(self.cfg.holder_exits.iter().flat_map(|exit| {
                exit.actions.iter().filter_map(|action| matches!(action,
                    CallableHolderAction::DropCaptureCellOwner { binding: actual, .. } if actual == &binding)
                    .then_some(HolderEmissionEdge::Terminal { from: exit.from }))
            })).collect::<Vec<_>>();
            let mut backings = input.backings.clone();
            for (edge, previous, incoming) in &replacements {
                let actions = self.connected_actions(*edge)?;
                if !actions.iter().any(|action| matches!(action,
                    CallableHolderAction::ReplaceCaptureCellValue { binding: actual, previous: old, incoming: new, .. }
                    if actual == &binding && old == previous && new == incoming)) {
                    return Err(format!("Capture at {:?} lacks connected replacement for {:?} on {edge:?}", point.source_span, binding));
                }
                for action in actions.iter().filter(|action| matches!(action,
                    CallableHolderAction::ReplaceCaptureCellValue { binding: actual, previous: old, incoming: new, .. }
                    if actual == &binding && old == previous && new == incoming)) {
                    self.action(*edge, action)?;
                }
                for backing in [previous, incoming] {
                    if !backings.contains(backing) {
                        backings.push(backing.clone());
                    }
                }
            }
            if backings
                .iter()
                .any(|backing| backing.kind == crate::scalar::RuntimeBackingKind::OwnedAllocation)
                && residence == super::ownership::CaptureCellResidence::Promoted
                && owner_exits.is_empty()
            {
                return Err(format!(
                    "Capture at {:?} lacks connected cell owner exit for {:?}",
                    point.source_span, binding
                ));
            }
            for edge in &owner_exits {
                let actions = self.connected_actions(*edge)?;
                for action in actions.iter().filter(|action| matches!(action,
                    CallableHolderAction::DropCaptureCellOwner { binding: actual, .. } if actual == &binding)) {
                    self.action(*edge, action)?;
                    if let CallableHolderAction::DropCaptureCellOwner { backing, .. } = action {
                        if backing.owner.source != binding.source
                            || backing.owner.function_span != binding.function_span
                            || backing.owner.declaration_span != binding.declaration_span
                            || backing.value.source != binding.source
                            || backing.value.function_span != binding.function_span {
                            return Err(format!("Capture at {:?} has an unrelated cell owner backing for {:?}: {:?}", point.source_span, binding, backing));
                        }
                        if !backings.contains(backing) {
                            backings.push(backing.clone());
                        }
                    }
                }
            }
            let mut initialized_origins = Vec::new();
            let mut initialized_cases = Vec::new();
            for backing in &backings {
                if backing.origin.source != binding.source {
                    return Err(format!(
                        "Capture at {:?} has an unrelated backing origin for {:?}",
                        point.source_span, binding
                    ));
                }
                match &backing.length {
                    crate::scalar::RuntimeArrayLengthFact::Fixed { origin, .. } => {
                        if origin.source != binding.source {
                            return Err(format!(
                                "Capture at {:?} lacks source-qualified backing length origin for {:?}",
                                point.source_span, binding
                            ));
                        }
                    }
                    crate::scalar::RuntimeArrayLengthFact::Evaluated {
                        point: index_point,
                        occurrence,
                        function_span,
                        ..
                    } => {
                        self.cfg
                            .points
                            .get(index_point.0)
                            .filter(|candidate| candidate.id == *index_point)
                            .ok_or_else(|| {
                                format!(
                                    "Capture at {:?} lacks connected backing length for {:?}",
                                    point.source_span, binding
                                )
                            })?;
                        if occurrence.source != binding.source
                            || *function_span != self.function_span()
                        {
                            return Err(format!(
                                "Capture at {:?} has a backing length from another function for {:?}",
                                point.source_span, binding
                            ));
                        }
                    }
                    crate::scalar::RuntimeArrayLengthFact::Parameter {
                        binding: length_binding,
                    } => {
                        if length_binding.source != binding.source
                            || length_binding.function_span != binding.function_span
                        {
                            return Err(format!(
                                "Capture at {:?} lacks source-qualified backing length parameter for {:?}",
                                point.source_span, binding
                            ));
                        }
                    }
                }
                if backing.owner.source != binding.source
                    || backing.owner.function_span != binding.function_span
                    || backing.owner.declaration_span != binding.declaration_span
                    || backing.value.source != binding.source
                    || backing.value.function_span != binding.function_span
                {
                    return Err(format!(
                        "Capture at {:?} has unrelated backing alternative for {:?}: {:?}",
                        point.source_span, binding, backing
                    ));
                }
                for family in &backing.callable_family {
                    let element = match &capture.ty {
                        ScalarType::RuntimeArray { element, .. } | ScalarType::Array { element, .. } => element,
                        _ => return Err(format!("Capture at {:?} has callable-array family on a different captured type for {:?}", point.source_span, binding)),
                    };
                    if !crate::scalar::scalar_type_equal(&family.element, element) {
                        return Err(format!(
                            "Capture at {:?} has a different callable element layout for {:?}",
                            point.source_span, binding
                        ));
                    }
                    for element in &family.elements {
                        if element.place.source != binding.source
                            || element.place.function_span != binding.function_span
                        {
                            return Err(format!(
                                "Capture at {:?} has a callable element with an unrelated direct place for {:?}",
                                point.source_span, binding
                            ));
                        }
                        if let crate::scalar::CallableArrayIndexFact::Selected {
                            point: index_point,
                            occurrence,
                            function_span,
                        } = &element.index
                        {
                            self.cfg
                                .points
                                .get(index_point.0)
                                .filter(|candidate| candidate.id == *index_point)
                                .ok_or_else(|| {
                                    format!(
                                        "Capture at {:?} lacks connected direct index for {:?}",
                                        point.source_span, binding
                                    )
                                })?;
                            if occurrence.source != binding.source
                                || *function_span != self.function_span()
                            {
                                return Err(format!(
                                    "Capture at {:?} has a direct index from another function for {:?}",
                                    point.source_span, binding
                                ));
                            }
                        }
                        let element_producer = self
                            .cfg
                            .environments
                            .get(element.environment.index)
                            .filter(|fact| fact.literal == element.environment.literal)
                            .ok_or_else(|| {
                                format!(
                                    "Capture at {:?} lacks source-qualified callable element producer {:?} for {:?}",
                                    point.source_span, element.environment.literal, binding
                                )
                            })?;
                        for loan in &element.loans {
                            if loan.origin.0 >= self.cfg.facts.len() {
                                return Err(format!(
                                    "Capture at {:?} has a callable element with an unrelated loan for {:?}",
                                    point.source_span, binding
                                ));
                            }
                            if !element_producer.loans.contains(loan) {
                                return Err(format!(
                                    "Capture at {:?} has a callable element loan outside its producer for {:?}",
                                    point.source_span, binding
                                ));
                            }
                        }
                    }
                    for index in &family.unselected.selected {
                        if let crate::scalar::CallableArrayIndexFact::Selected {
                            point: index_point,
                            occurrence,
                            function_span,
                        } = index
                        {
                            self.cfg
                                .points
                                .get(index_point.0)
                                .filter(|candidate| candidate.id == *index_point)
                                .ok_or_else(|| {
                                    format!(
                                        "Capture at {:?} lacks connected direct index for {:?}",
                                        point.source_span, binding
                                    )
                                })?;
                            if occurrence.source != binding.source
                                || *function_span != self.function_span()
                            {
                                return Err(format!(
                                    "Capture at {:?} has a direct index from another function for {:?}",
                                    point.source_span, binding
                                ));
                            }
                        }
                    }
                    for partition in &family.partitions {
                        for relation in &partition.relations {
                            for key in [&relation.left, &relation.right] {
                                self.installed_index_key(key, &point.source_span, &binding)?;
                            }
                        }
                        for representative in &partition.representatives {
                            self.installed_index_key(
                                &representative.key,
                                &point.source_span,
                                &binding,
                            )?;
                            for anchor in &representative.anchors {
                                match anchor {
                                    crate::scalar::CallableIndexAnchorFact::ValuePlace(place) => {
                                        if place.source != binding.source
                                            || place.function_span != binding.function_span
                                        {
                                            return Err(format!(
                                                "Capture at {:?} has an unrelated installed anchor place for {:?}",
                                                point.source_span, binding
                                            ));
                                        }
                                    }
                                    crate::scalar::CallableIndexAnchorFact::ActiveLoan {
                                        loan,
                                        owner,
                                        origin,
                                        ..
                                    } => {
                                        if loan.origin_place.source != binding.source {
                                            return Err(format!(
                                                "Capture at {:?} has an installed anchor with an unrelated loan place for {:?}",
                                                point.source_span, binding
                                            ));
                                        }
                                        if owner.source != binding.source {
                                            return Err(format!(
                                                "Capture at {:?} has an unrelated installed loan owner for {:?}",
                                                point.source_span, binding
                                            ));
                                        }
                                        if origin.source != binding.source {
                                            return Err(format!(
                                                "Capture at {:?} has an unrelated installed loan origin for {:?}",
                                                point.source_span, binding
                                            ));
                                        }
                                    }
                                    crate::scalar::CallableIndexAnchorFact::EscapedHolder(_) => {}
                                    crate::scalar::CallableIndexAnchorFact::InFlightSelection(
                                        index_point,
                                    ) => {
                                        self.cfg
                                            .points
                                            .get(index_point.0)
                                            .filter(|candidate| candidate.id == *index_point)
                                            .ok_or_else(|| {
                                                format!(
                                                    "Capture at {:?} lacks connected installed anchor for {:?}",
                                                    point.source_span, binding
                                                )
                                            })?;
                                    }
                                }
                            }
                        }
                    }
                    let alternatives = family
                        .partitions
                        .iter()
                        .flat_map(|partition| {
                            partition
                                .representatives
                                .iter()
                                .flat_map(|representative| &representative.cell.alternatives)
                                .chain(&partition.remainder.alternatives)
                        })
                        .collect::<Vec<_>>();
                    for case in &alternatives {
                        if let crate::scalar::CallableCellCaseFact::Initialized {
                            environment_origin,
                            backing: value,
                            owner,
                            capture_paths,
                            instance,
                            holder_aliases,
                            loans,
                            origin,
                            ..
                        } = case
                        {
                            if value != &backing.value || owner != &backing.owner {
                                return Err(format!("Capture at {:?} has a callable element with unrelated backing owner for {:?}", point.source_span, binding));
                            }
                            let producer = self
                                .cfg
                                .environments
                                .get(environment_origin.index)
                                .filter(|fact| fact.literal == environment_origin.literal)
                                .ok_or_else(|| {
                                    format!(
                                        "Capture at {:?} lacks callable element producer {:?}",
                                        point.source_span, environment_origin.literal
                                    )
                                })?;
                            if !capture_paths
                                .iter()
                                .all(|path| producer.capture_paths.contains(path))
                            {
                                return Err(format!("Capture at {:?} has callable element capture layout differing from producer {:?}", point.source_span, environment_origin.literal));
                            }
                            if origin.source != binding.source {
                                return Err(format!(
                                    "Capture at {:?} has an unrelated installed case origin for {:?}",
                                    point.source_span, binding
                                ));
                            }
                            if let crate::scalar::CallableEnvironmentInstanceClassFact::SharedWith(
                                place,
                            ) = instance
                            {
                                let qualified = match place {
                                    crate::scalar::CallableHolderPlace::Binding(candidate) => {
                                        candidate.source == binding.source
                                    }
                                    crate::scalar::CallableHolderPlace::Projection(place) => {
                                        place.source == binding.source
                                    }
                                    crate::scalar::CallableHolderPlace::Temporary(index_point) => {
                                        self.cfg
                                            .points
                                            .get(index_point.0)
                                            .is_some_and(|candidate| candidate.id == *index_point)
                                    }
                                    _ => true,
                                };
                                if !qualified {
                                    return Err(format!(
                                        "Capture at {:?} has an unrelated installed case instance for {:?}",
                                        point.source_span, binding
                                    ));
                                }
                            }
                            for alias in holder_aliases {
                                match alias {
                                    crate::scalar::CallableHolderPlace::Binding(candidate)
                                        if candidate.source != binding.source =>
                                    {
                                        return Err(format!(
                                            "Capture at {:?} has an unrelated installed case alias for {:?}",
                                            point.source_span, binding
                                        ));
                                    }
                                    crate::scalar::CallableHolderPlace::Projection(place)
                                        if place.source != binding.source =>
                                    {
                                        return Err(format!(
                                            "Capture at {:?} has an unrelated installed case alias for {:?}",
                                            point.source_span, binding
                                        ));
                                    }
                                    crate::scalar::CallableHolderPlace::Temporary(index_point)
                                        if self.cfg.points.get(index_point.0).is_none_or(
                                            |candidate| candidate.id != *index_point,
                                        ) =>
                                    {
                                        return Err(format!(
                                            "Capture at {:?} has an unconnected installed case alias for {:?}",
                                            point.source_span, binding
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                            for loan in loans {
                                if loan.origin.0 >= self.cfg.facts.len() {
                                    return Err(format!(
                                        "Capture at {:?} has an installed case with an unrelated loan for {:?}",
                                        point.source_span, binding
                                    ));
                                }
                                if !producer.loans.contains(loan) {
                                    return Err(format!(
                                        "Capture at {:?} has an installed case loan outside its producer for {:?}",
                                        point.source_span, binding
                                    ));
                                }
                            }
                            if !initialized_cases.contains(*case) {
                                initialized_cases.push((*case).clone());
                            }
                        }
                    }
                    for origin in family
                        .elements
                        .iter()
                        .map(|element| &element.environment)
                        .chain(alternatives.into_iter().filter_map(|case| match case {
                            crate::scalar::CallableCellCaseFact::Initialized {
                                environment_origin,
                                ..
                            } => Some(environment_origin),
                            crate::scalar::CallableCellCaseFact::Uninitialized => None,
                        }))
                    {
                        self.cfg.environments.get(origin.index).filter(|fact| fact.literal == origin.literal)
                            .ok_or_else(|| format!("Capture at {:?} lacks source-qualified callable element producer {:?} for {:?}", point.source_span, origin.literal, binding))?;
                        if !initialized_origins.contains(origin) {
                            initialized_origins.push(origin.clone());
                        }
                    }
                }
            }
            let cell_offset = offset;
            offset = offset
                .checked_add(slot_width)
                .ok_or_else(|| format!("Capture at {:?} overflows payload", point.source_span))?;
            let length_offset = if matches!(capture.ty, ScalarType::RuntimeArray { .. }) {
                let length = offset;
                offset = offset.checked_add(slot_width).ok_or_else(|| {
                    format!("Capture at {:?} overflows length slot", point.source_span)
                })?;
                Some(length)
            } else {
                None
            };
            cells.push(CaptureDescriptorCell {
                binding,
                ty: capture.ty.clone(),
                cell_offset,
                length_offset,
                residence,
                backings,
                owner_exits,
                replacements,
                initialized_origins,
                initialized_cases,
            });
        }
        Ok(Some(CaptureDescriptor {
            literal: point.source_span.clone(),
            signature: signature.clone(),
            cells,
            slot_width,
            payload_size: offset,
        }))
    }
    pub(super) fn counted_environment(&self, environment: &CallableEnvironmentId) -> bool {
        self.cfg
            .holder_edges
            .iter()
            .flat_map(|edge| &edge.actions)
            .chain(self.cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
            .any(|action| {
                matches!(action,
            CallableHolderAction::MoveSelected { environment: actual, .. }
            | CallableHolderAction::RetainSelected { environment: actual, .. }
            if actual == environment)
                    || matches!(action,
                        CallableHolderAction::RetainCheckedSelected {
                            old: crate::scalar::CallableCellCaseFact::Initialized { environment_origin, .. }, ..
                        } if environment_origin == environment)
                    || matches!(action,
                        CallableHolderAction::Retain {
                            environment: actual,
                            from: CallableHolderPlace::Projection(place),
                            ..
                        } if actual == environment
                            && place.source == environment.literal.source
                            && place.projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Field(_))))
                    || matches!(action,
                        CallableHolderAction::Drop {
                            environment: actual,
                            holder: CallableHolderPlace::Projection(place),
                            ..
                        } if actual == environment
                            && place.source == environment.literal.source
                            && place.projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Index(_))))
            })
            || self.covered_cells.values().any(|coverage| {
                coverage
                    .environments
                    .iter()
                    .any(|candidate| candidate.literal == environment.literal)
            })
            || self.imported_holders.values().any(|holder| {
                holder.owned.iter().any(|candidate| {
                    matches!(&candidate.ownership,
                    super::ownership::ImportedCaptureOwnership::Counted(coverage)
                        if coverage.environment == *environment)
                })
            })
    }

    pub(super) fn moved_to_selected(
        &self,
        environment: &CallableEnvironmentId,
        holder: &CallableHolderPlace,
    ) -> bool {
        self.cfg
            .holder_edges
            .iter()
            .flat_map(|edge| &edge.actions)
            .any(|action| {
                matches!(action,
            CallableHolderAction::MoveSelected { environment: actual, from, .. }
            if actual == environment && from == holder)
            })
    }

    pub(super) fn counted_action(
        &self,
        edge: HolderEmissionEdge,
        action: &CallableHolderAction,
        environment: &CallableEnvironmentId,
    ) -> Result<(), String> {
        for (key, holder) in &self.imported_holders {
            for candidate in &holder.owned {
                let super::ownership::ImportedCaptureOwnership::Counted(coverage) =
                    &candidate.ownership
                else {
                    continue;
                };
                if coverage.environment != *environment {
                    continue;
                }
                if coverage.key != *key
                    || candidate.literal != environment.literal
                    || coverage.backing != candidate.backing.fact
                    || coverage.producer.literal != candidate.literal
                    || !coverage.producer.cells.iter().any(|cell| {
                        cell.binding == coverage.binding
                            && cell.cell_offset == candidate.backing.cell_offset
                            && cell.backings.contains(&coverage.backing)
                    })
                    || !coverage.actions.contains(&(edge, action.clone()))
                    || (matches!(action, CallableHolderAction::Drop { .. })
                        && !coverage.release_edges.contains(&edge))
                {
                    return Err(format!("source-qualified imported output {key:?} lacks connected counted action {action:?} on {edge:?}"));
                }
            }
        }
        Ok(())
    }

    pub(super) fn imported_environment_covered(
        &self,
        key: &super::ownership::ImportedHolderKey,
        producer: &CaptureDescriptor,
        binding: &ReferenceBindingId,
        backing: &crate::scalar::RuntimeBackingFact,
    ) -> Result<Option<super::ownership::ImportedCaptureCoverage>, String> {
        let literal = &producer.literal;
        let cell = producer.cells.iter().find(|cell| &cell.binding == binding)
            .ok_or_else(|| format!("source-qualified imported output {key:?} lacks producer binding {binding:?} at {literal:?}"))?;
        if !cell.backings.contains(backing) {
            return Err(format!("source-qualified imported output {key:?} lacks producer backing for {binding:?} at {literal:?}"));
        }
        if key.source != self.cfg.points[key.call_point.0].source_span.source
            || key.function_span != self.function_span()
            || !matches!(&self.cfg.points[key.call_point.0].kind, CfgPointKind::Call { output, .. } if output == &key.output)
        {
            return Err(format!("source-qualified imported output {key:?} has disconnected call/output producer at {literal:?}"));
        }
        let holder = &self
            .imported_holders
            .get(key)
            .ok_or_else(|| format!("missing source-qualified imported output {key:?}"))?
            .holder;
        let selected = &self.cfg.points[key.call_point.0];
        let possible = if key.fields.is_empty() {
            selected
                .possible_environments
                .iter()
                .any(|environment| &environment.literal == literal)
        } else {
            selected
                .possible_projected_environments
                .iter()
                .any(|projection| {
                    projection
                        .place
                        .projections
                        .iter()
                        .map(|step| match step {
                            ReferencePlaceProjection::Field(
                                crate::scalar::ScalarFieldReference::Resolved(field),
                            ) => Some(field.index),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()
                        .as_deref()
                        == Some(key.fields.as_slice())
                        && projection
                            .environments
                            .iter()
                            .any(|environment| &environment.literal == literal)
                })
        };
        if !possible {
            return Ok(None);
        }
        let mut moved = self
            .cfg
            .holder_edges
            .iter()
            .flat_map(|fact| &fact.actions)
            .filter_map(|action| match action {
                CallableHolderAction::Move {
                    environment, from, ..
                } if from == holder && &environment.literal == literal => Some(environment.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        moved.sort();
        moved.dedup();
        let [environment] = moved.as_slice() else {
            return Err(format!("source-qualified imported output {key:?} has no unique connected selected producer at {literal:?}: {moved:?}"));
        };
        if environment.literal != producer.literal {
            return Err(format!(
                "source-qualified imported output {key:?} selects another producer at {literal:?}"
            ));
        }
        let mut actions = Vec::new();
        let mut release_edges = Vec::new();
        for fact in &self.cfg.holder_edges {
            for action in &fact.actions {
                if matches!(action,
                    CallableHolderAction::Move { environment: actual, .. }
                    | CallableHolderAction::Retain { environment: actual, .. }
                    | CallableHolderAction::Drop { environment: actual, .. }
                    | CallableHolderAction::ReturnTransfer { environment: actual, .. }
                    if actual == environment)
                {
                    let edge = HolderEmissionEdge::Outgoing {
                        from: fact.from,
                        to: fact.to,
                    };
                    self.action(edge, action)?;
                    let point = &self.cfg.points[fact.from.0];
                    if point.successors.len() != 1
                        || !matches!(
                            point.kind,
                            CfgPointKind::Bind { .. }
                                | CfgPointKind::CopyCallableFields { .. }
                                | CfgPointKind::Assign { .. }
                                | CfgPointKind::ReturnOutput { .. }
                                | CfgPointKind::ScopeExit
                        )
                    {
                        return Err(format!("source-qualified imported output {key:?} has unhandled counted action {action:?} at {:?}", point.source_span));
                    }
                    if matches!(action, CallableHolderAction::Drop { .. }) {
                        release_edges.push(edge);
                    }
                    actions.push((edge, action.clone()));
                }
            }
        }
        for fact in &self.cfg.holder_exits {
            for action in &fact.actions {
                if matches!(action, CallableHolderAction::Drop { environment: actual, .. }
                    if actual == environment)
                {
                    let edge = HolderEmissionEdge::Terminal { from: fact.from };
                    self.action(edge, action)?;
                    release_edges.push(edge);
                    actions.push((edge, action.clone()));
                }
            }
        }
        if release_edges.is_empty() {
            return Ok(None);
        }
        Ok(Some(super::ownership::ImportedCaptureCoverage {
            key: key.clone(),
            environment: environment.clone(),
            backing: backing.clone(),
            producer: producer.clone(),
            binding: binding.clone(),
            actions,
            release_edges,
        }))
    }

    pub(super) fn imported_holder_has_literal(
        &self,
        key: &super::ownership::ImportedHolderKey,
        literal: &wosy_syntax::SourceSpan,
    ) -> bool {
        let point = &self.cfg.points[key.call_point.0];
        if key.fields.is_empty() {
            point
                .possible_environments
                .iter()
                .any(|environment| &environment.literal == literal)
        } else {
            point
                .possible_projected_environments
                .iter()
                .any(|projection| {
                    projection
                        .place
                        .projections
                        .iter()
                        .map(|step| match step {
                            ReferencePlaceProjection::Field(
                                crate::scalar::ScalarFieldReference::Resolved(field),
                            ) => Some(field.index),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()
                        .as_deref()
                        == Some(key.fields.as_slice())
                        && projection
                            .environments
                            .iter()
                            .any(|environment| &environment.literal == literal)
                })
        }
    }
    pub(super) fn connected_actions(
        &self,
        edge: HolderEmissionEdge,
    ) -> Result<Vec<CallableHolderAction>, String> {
        self.edge_relations(edge)?;
        match edge {
            HolderEmissionEdge::Outgoing { from, to } => Ok(self
                .outgoing(from, to)?
                .iter()
                .flat_map(|fact| fact.actions.iter().cloned())
                .collect()),
            HolderEmissionEdge::Terminal { from } => Ok(self
                .exits(from)?
                .iter()
                .flat_map(|fact| fact.actions.iter().cloned())
                .collect()),
        }
    }

    pub(super) fn binding_action_edges(
        &self,
        binding: &ScalarBinding,
    ) -> Result<Vec<HolderEmissionEdge>, String> {
        let mut result = Vec::new();
        for event in self
            .cfg
            .binding_points
            .iter()
            .filter(|event| event.occurrence == *binding.occurrence)
        {
            let point = self.cfg.points.get(event.point.0).ok_or_else(|| {
                format!(
                    "missing binding action point {:?} for {:?}",
                    event.point, binding.occurrence
                )
            })?;
            for to in &point.successors {
                let edge = HolderEmissionEdge::Outgoing {
                    from: point.id,
                    to: *to,
                };
                self.connected_actions(edge)?;
                result.push(edge);
            }
        }
        result.sort_by_key(|edge| edge.from().0);
        result.dedup();
        Ok(result)
    }

    pub(super) fn fixed_copy_action_edges(
        &self,
        target: &ReferenceBindingId,
        source: &ReferenceBindingId,
        span: wosy_syntax::ByteSpan,
    ) -> Result<Vec<HolderEmissionEdge>, String> {
        let points = self.cfg.points.iter().filter(|point| point.source_span.source == target.source && point.source_span.range == span && matches!(
            &point.kind,
            CfgPointKind::Backing { target: actual, transfer: crate::scalar::RuntimeBackingTransfer::CopyFixed { source: copied }, .. }
                if actual == target && copied == source
        )).collect::<Vec<_>>();
        let [point] = points.as_slice() else {
            return Err(format!("fixed copy {target:?} from {source:?} lacks a unique source-qualified Backing at {span:?}"));
        };
        let mut edges = Vec::new();
        let mut alternatives = Vec::new();
        for to in &point.successors {
            let edge = HolderEmissionEdge::Outgoing {
                from: point.id,
                to: *to,
            };
            let actions = self.connected_actions(edge)?;
            if !actions.iter().any(|action| {
                matches!(action,
                    CallableHolderAction::RetainArrayElements { source: previous, backing, .. }
                        if previous.owner == *source && backing.owner == *target
                )
            }) {
                return Err(format!(
                    "fixed copy {target:?} from {source:?} lacks connected element retains at {:?}",
                    point.source_span
                ));
            }
            let retains = actions
                .into_iter()
                .filter(|action| {
                    matches!(action,
                        CallableHolderAction::RetainArrayElements { source: previous, backing, .. }
                            if previous.owner == *source && backing.owner == *target
                    )
                })
                .collect::<Vec<_>>();
            if let Some(first) = alternatives.first() {
                if first != &retains {
                    return Err(format!("fixed copy {target:?} from {source:?} has ambiguous source-qualified element alternatives at {:?}", point.source_span));
                }
            }
            alternatives.push(retains);
            edges.push(edge);
        }
        if edges.is_empty() {
            return Err(format!(
                "fixed copy {target:?} from {source:?} has no connected successor at {:?}",
                point.source_span
            ));
        }
        Ok(vec![edges[0]])
    }

    pub(super) fn scope_action_edges(
        &self,
        point: &ReferenceFlowPoint,
    ) -> Result<Vec<HolderEmissionEdge>, String> {
        if !matches!(point.kind, CfgPointKind::ScopeExit) {
            return Err(format!(
                "expected connected ScopeExit at {:?}",
                point.source_span
            ));
        }
        if point.successors.is_empty() {
            let edge = HolderEmissionEdge::Terminal { from: point.id };
            self.connected_actions(edge)?;
            return Ok(vec![edge]);
        }
        point
            .successors
            .iter()
            .map(|to| {
                let edge = HolderEmissionEdge::Outgoing {
                    from: point.id,
                    to: *to,
                };
                self.connected_actions(edge)?;
                Ok(edge)
            })
            .collect()
    }

    pub(super) fn replacement_edges(
        &self,
        assignment: &ScalarAssignment,
        binding: &ReferenceBindingId,
    ) -> Result<Vec<HolderEmissionEdge>, String> {
        let matches = self.cfg.points.iter().filter(|point| {
            point.source_span.source == assignment.occurrence.source_span.source
                && assignment.targets.iter().any(|target| target.target_span == point.source_span.range)
                && matches!(&point.kind, CfgPointKind::Backing { target, .. } if target == binding)
        }).collect::<Vec<_>>();
        let [point] = matches.as_slice() else {
            let candidates = self
                .cfg
                .points
                .iter()
                .filter_map(|point| {
                    matches!(&point.kind,
                CfgPointKind::Backing { target, .. } if target == binding)
                    .then_some(point.source_span.clone())
                })
                .collect::<Vec<_>>();
            return Err(format!(
                "captured cell {binding:?} lacks unique connected Backing for {:?}: {candidates:?}",
                assignment.occurrence
            ));
        };
        point
            .successors
            .iter()
            .map(|to| {
                let edge = HolderEmissionEdge::Outgoing {
                    from: point.id,
                    to: *to,
                };
                if !self.connected_actions(edge)?.iter().any(|action| {
                    matches!(action,
                CallableHolderAction::ReplaceCaptureCellValue { binding: actual, .. }
                if actual == binding)
                }) {
                    return Err(format!(
                        "captured cell {binding:?} lacks connected replacement on {edge:?}"
                    ));
                }
                Ok(edge)
            })
            .collect()
    }

    pub(super) fn assignment_binding(
        &self,
        assignment: &ScalarAssignment,
        output: usize,
    ) -> Result<ReferenceBindingId, String> {
        let matches = self
            .cfg
            .assignment_points
            .iter()
            .filter(|event| {
                event.occurrence == *assignment.occurrence
                    && event.target == output
                    && event.role == ScalarAssignmentPointRole::Assign
            })
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "missing unique typed Assign for {:?} target {output}",
                assignment.occurrence
            ));
        };
        let point = self.structural_point(event.point, HolderPointKind::Assign)?;
        let CfgPointKind::Assign { target, .. } = &point.kind else {
            return Err(format!(
                "invalid typed Assign for {:?} target {output}",
                assignment.occurrence
            ));
        };
        if target.source != assignment.occurrence.source_span.source {
            return Err(format!(
                "assignment {:?} targets a different source",
                assignment.occurrence
            ));
        }
        Ok(target.binding.clone())
    }

    pub(super) fn assignment_copy_edges(
        &self,
        assignment: &ScalarAssignment,
        output: usize,
    ) -> Result<Vec<HolderEmissionEdge>, String> {
        let events = self
            .cfg
            .assignment_points
            .iter()
            .filter(|event| {
                event.occurrence == *assignment.occurrence
                    && event.target == output
                    && event.role == ScalarAssignmentPointRole::CopyCallableFields
            })
            .collect::<Vec<_>>();
        let mut edges = Vec::new();
        for event in events {
            let point = self.structural_point(event.point, HolderPointKind::CopyCallableFields)?;
            self.validated(point)?;
            for to in &point.successors {
                let edge = HolderEmissionEdge::Outgoing {
                    from: point.id,
                    to: *to,
                };
                self.connected_actions(edge)?;
                edges.push(edge);
            }
        }
        edges.sort_by_key(|edge| (edge.from().0, edge.to().map(|to| to.0)));
        edges.dedup();
        Ok(edges)
    }

    pub(super) fn function_span(&self) -> wosy_syntax::ByteSpan {
        self.cfg.points[0].source_span.range
    }

    pub(super) fn scope_exit_edge(&self, edge: HolderEmissionEdge) -> bool {
        matches!(self.cfg.points[edge.from().0].kind, CfgPointKind::ScopeExit)
    }

    pub(super) fn source_span(
        &self,
        edge: HolderEmissionEdge,
    ) -> Result<&wosy_syntax::SourceSpan, String> {
        self.cfg
            .points
            .get(edge.from().0)
            .map(|point| &point.source_span)
            .ok_or_else(|| format!("missing source for connected holder edge {edge:?}"))
    }

    pub(super) fn array_cleanup_owner(
        &self,
        binding: &ReferenceBindingId,
    ) -> Result<Vec<crate::scalar::RuntimeBackingFact>, String> {
        let backings = self
            .cfg
            .holder_edges
            .iter()
            .flat_map(|edge| &edge.actions)
            .chain(self.cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
            .filter_map(|action| match action {
                CallableHolderAction::DropArrayElements { backing, .. }
                    if &backing.owner == binding =>
                {
                    Some(backing.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if backings.is_empty() {
            return Err(format!(
                "local callable owner {binding:?} lacks connected cleanup"
            ));
        }
        Ok(backings)
    }
    pub(super) fn register_binding(
        &mut self,
        binding: &ScalarBinding,
        output: usize,
        address: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let matches = self
            .cfg
            .binding_points
            .iter()
            .filter(|event| {
                event.occurrence == *binding.occurrence
                    && event.output == output
                    && event.role == ScalarBindingPointRole::Bind
            })
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "missing unique typed Bind for {:?} output {output}",
                binding.occurrence
            ));
        };
        let point = self.structural_point(event.point, HolderPointKind::Bind)?;
        let CfgPointKind::Bind { binding: id } = &point.kind else {
            return Err(format!("invalid typed Bind {:?}", point.id));
        };
        if id.source != binding.occurrence.source_span.source
            || id.declaration_span != binding.receivers[output].name_span
        {
            return Err(format!("typed Bind {:?} has another receiver", point.id));
        }
        let id = id.clone();
        self.bindings.insert(id, address);
        Ok(())
    }

    pub(super) fn binding_id(
        &self,
        binding: &ScalarBinding,
        output: usize,
    ) -> Result<ReferenceBindingId, String> {
        let matches = self
            .cfg
            .binding_points
            .iter()
            .filter(|event| {
                event.occurrence == *binding.occurrence
                    && event.output == output
                    && event.role == ScalarBindingPointRole::Bind
            })
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "binding {:?} output {output} lacks unique Bind",
                binding.occurrence
            ));
        };
        let point = self.structural_point(event.point, HolderPointKind::Bind)?;
        let CfgPointKind::Bind { binding: target } = &point.kind else {
            return Err(format!(
                "binding {:?} output {output} has another point",
                binding.occurrence
            ));
        };
        Ok(target.clone())
    }

    pub(super) fn connected_holder_edge(
        &self,
        edge: HolderEmissionEdge,
        holder: &CallableHolderPlace,
    ) -> Result<(), String> {
        let relations = self.edge_relations(edge)?;
        if !relations.iter().any(|(before, after)| {
            before
                .iter()
                .chain(*after)
                .any(|relation| relation.holders.contains(holder))
        }) {
            return Err(format!("holder {holder:?} absent on {edge:?}"));
        }
        let source = &self.cfg.points[edge.from().0].source_span.source;
        let holder_binding = match holder {
            CallableHolderPlace::Binding(binding) => Some(binding),
            CallableHolderPlace::Projection(place) => Some(&place.binding),
            CallableHolderPlace::Temporary(_)
            | CallableHolderPlace::Output(_)
            | CallableHolderPlace::OutputProjection { .. } => None,
        };
        if let Some(binding) = holder_binding {
            let is_module_storage = binding.function_span == wosy_syntax::ByteSpan::new(0, 0)
                && binding.kind == ReferenceBindingKind::Declared;
            if !is_module_storage && &binding.source != source {
                return Err(format!(
                    "holder {holder:?} belongs to another source on {edge:?}"
                ));
            }
        }
        Ok(())
    }

    fn edge_relations(
        &self,
        edge: HolderEmissionEdge,
    ) -> Result<
        Vec<(
            &[crate::scalar::CallableHolderRelation],
            &[crate::scalar::CallableHolderRelation],
        )>,
        String,
    > {
        let from = edge.from();
        let point = self
            .cfg
            .points
            .get(from.0)
            .filter(|point| point.id == from)
            .ok_or_else(|| format!("missing callable holder point {from:?}"))?;
        match edge {
            HolderEmissionEdge::Outgoing { to, .. } => {
                if !point.successors.contains(&to) {
                    return Err(format!("holder edge {from:?} -> {to:?} is disconnected"));
                }
                Ok(self
                    .outgoing(from, to)?
                    .iter()
                    .map(|fact| (fact.before.as_slice(), fact.after.as_slice()))
                    .collect())
            }
            HolderEmissionEdge::Terminal { .. } => {
                if !matches!(point.kind, CfgPointKind::ScopeExit) || !point.successors.is_empty() {
                    return Err(format!("holder exit {from:?} requires terminal ScopeExit"));
                }
                Ok(self
                    .exits(from)?
                    .iter()
                    .map(|fact| (fact.before.as_slice(), fact.after.as_slice()))
                    .collect())
            }
        }
    }

    pub(super) fn action(
        &self,
        edge: HolderEmissionEdge,
        action: &CallableHolderAction,
    ) -> Result<(), String> {
        self.edge_relations(edge)?;
        let actions = match edge {
            HolderEmissionEdge::Outgoing { from, to } => self
                .outgoing(from, to)?
                .iter()
                .flat_map(|fact| &fact.actions)
                .collect::<Vec<_>>(),
            HolderEmissionEdge::Terminal { from } => self
                .exits(from)?
                .iter()
                .flat_map(|fact| &fact.actions)
                .collect::<Vec<_>>(),
        };
        let matches = actions
            .iter()
            .filter(|candidate| ***candidate == *action)
            .count();
        if matches == 0 {
            return Err(format!(
                "expected one connected action {action:?} on {edge:?}, found {matches}"
            ));
        }
        let source = &self.cfg.points[edge.from().0].source_span.source;
        let span = match action {
            CallableHolderAction::Retain { span, .. }
            | CallableHolderAction::Move { span, .. }
            | CallableHolderAction::Drop { span, .. }
            | CallableHolderAction::ReturnTransfer { span, .. }
            | CallableHolderAction::DropCaptureCellOwner { span, .. }
            | CallableHolderAction::ReplaceCaptureCellValue { span, .. }
            | CallableHolderAction::DropSelected { span, .. }
            | CallableHolderAction::SkipUninitializedSelected { span, .. }
            | CallableHolderAction::RetainCheckedSelected { span, .. }
            | CallableHolderAction::RetainSelected { span, .. }
            | CallableHolderAction::MoveSelected { span, .. }
            | CallableHolderAction::DropArrayElements { span, .. } => span,
            CallableHolderAction::RetainArrayElements { span, .. } => span,
            CallableHolderAction::DropCapturedValueCellOwner { span, .. } => span,
            _ => {
                return Err(format!(
                    "{action:?} is not a scalar holder/cell owner action"
                ))
            }
        };
        if &span.source != source {
            return Err(format!(
                "action {action:?} belongs to another source on {edge:?}"
            ));
        }
        Ok(())
    }

    pub(super) fn capture_cell_owner(
        &self,
        edge: HolderEmissionEdge,
        action: &CallableHolderAction,
        cells: &BTreeMap<ReferenceBindingId, PointerValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, String> {
        self.action(edge, action)?;
        let (binding, cell, backing) = match action {
            CallableHolderAction::DropCaptureCellOwner {
                binding,
                cell,
                backing,
                ..
            } => (binding, cell, backing),
            CallableHolderAction::ReplaceCaptureCellValue {
                binding,
                cell,
                previous,
                incoming,
                ..
            } => {
                if previous.value.source != incoming.value.source
                    || previous.value.function_span != incoming.value.function_span
                    || incoming.owner.source != binding.source
                    || incoming.value.source != binding.source
                    || incoming.value.function_span != binding.function_span
                {
                    return Err(format!("capture cell replacement {binding:?} has unrelated incoming backing on {edge:?}"));
                }
                (binding, cell, previous)
            }
            _ => return Err(format!("{action:?} is not a capture cell owner action")),
        };
        let point = &self.cfg.points[edge.from().0];
        if binding.source != point.source_span.source
            || binding.function_span != self.function_span()
            || cell.source != binding.source
            || cell.range != binding.declaration_span
            || backing.owner.source != binding.source
            || backing.value.source != binding.source
            || backing.value.function_span != binding.function_span
        {
            return Err(format!(
                "capture cell owner {binding:?} is unrelated to {edge:?}"
            ));
        }
        cells.get(binding).copied().ok_or_else(|| {
            format!("capture cell owner {binding:?} lacks emitted storage on {edge:?}")
        })
    }

    pub(super) fn return_holder_point(
        &self,
        from: CfgPointId,
        to: CfgPointId,
        output: usize,
    ) -> Result<CfgPointId, String> {
        let point = self.structural_point(from, HolderPointKind::ReturnOutput)?;
        if !matches!(point.kind, CfgPointKind::ReturnOutput { output: position, .. } if position == output)
            || !point.successors.contains(&to)
        {
            return Err(format!(
                "output {output} lacks connected ReturnOutput {from:?} -> {to:?}"
            ));
        }
        self.outgoing(from, to)?;
        Ok(from)
    }

    pub(super) fn selected_projection(
        &self,
        from: CfgPointId,
        to: CfgPointId,
        place: &crate::scalar::ReferencePlaceId,
    ) -> Result<(CfgPointId, CfgPointId), String> {
        let matches = self
            .outgoing(from, to)?
            .iter()
            .flat_map(|edge| &edge.actions)
            .filter_map(|action| match action {
                crate::scalar::CallableHolderAction::SelectElement {
                    selected,
                    index_point,
                    ..
                } if selected == place => Some(*index_point),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [index_point] = matches.as_slice() else {
            return Err(format!(
                "selected projection {place:?} lacks one connected index edge"
            ));
        };
        let occurrences = self
            .cfg
            .assignment_points
            .iter()
            .filter(|event| event.point == from && event.role == ScalarAssignmentPointRole::Assign)
            .collect::<Vec<_>>();
        let [assignment] = occurrences.as_slice() else {
            return Err(format!(
                "selected projection {place:?} lacks typed assignment edge"
            ));
        };
        let selected = self
            .cfg
            .assignment_points
            .iter()
            .filter(|event| {
                event.occurrence == assignment.occurrence
                    && event.target == assignment.target
                    && event.role == ScalarAssignmentPointRole::SelectElement
            })
            .collect::<Vec<_>>();
        let [selected] = selected.as_slice() else {
            return Err(format!(
                "selected projection {place:?} lacks typed SelectElement"
            ));
        };
        Ok((selected.point, *index_point))
    }
    pub(super) fn read_place(
        &self,
        expression: &ScalarExpression,
    ) -> Result<&ReferenceFlowPoint, String> {
        let (place, occurrence, role) = match expression {
            ScalarExpression::Dereference {
                place, occurrence, ..
            } => (place, occurrence.as_ref(), ScalarReadRole::Dereference),
            ScalarExpression::IndexedRead {
                place, occurrence, ..
            } => (place, occurrence.as_ref(), ScalarReadRole::IndexedRead),
            _ => return Err("ReadPlace requires a typed dereference or indexed read".to_owned()),
        };
        if occurrence.role != role {
            return Err(format!("ReadPlace {occurrence:?} has a different AST role"));
        }
        let matches = self
            .cfg
            .read_points
            .iter()
            .filter(|event| event.occurrence == *occurrence)
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "expected one ReadPlace for {occurrence:?}, found {}",
                matches.len()
            ));
        };
        if event.function_span != self.cfg.points[0].source_span.range {
            return Err(format!(
                "ReadPlace {occurrence:?} belongs to a different function"
            ));
        }
        let point = self.structural_point(event.point, HolderPointKind::ReadPlace)?;
        if point.source_span != occurrence.source_span
            || !matches!(&point.kind, CfgPointKind::ReadPlace { place: recorded, .. } if recorded == place)
        {
            return Err(format!(
                "ReadPlace {occurrence:?} targets a different place"
            ));
        }
        self.validated(point)?;
        Ok(point)
    }

    pub(super) fn selected_assignment(
        &self,
        assignment: &ScalarAssignment,
        output: usize,
    ) -> Result<(CfgPointId, CfgPointId), String> {
        let matches = self
            .cfg
            .assignment_points
            .iter()
            .filter(|event| {
                event.occurrence == *assignment.occurrence
                    && event.target == output
                    && event.role == ScalarAssignmentPointRole::SelectElement
            })
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "expected one connected SelectElement for {:?} target {output}, found {}",
                assignment.occurrence,
                matches.len()
            ));
        };
        let point = self
            .cfg
            .points
            .get(event.point.0)
            .ok_or_else(|| format!("missing selected element point {:?}", event.point))?;
        let CfgPointKind::SelectArrayElement {
            target,
            index_point,
        } = &point.kind
        else {
            return Err(format!("invalid SelectElement point {:?}", event.point));
        };
        if point.id != event.point
            || target.place != assignment.targets[output].place
            || target.source != assignment.occurrence.source_span.source
        {
            return Err(format!(
                "SelectElement {:?} targets a different assignment",
                event.point
            ));
        }
        if !self
            .cfg
            .points
            .iter()
            .any(|candidate| candidate.id == *index_point)
        {
            return Err(format!("missing evaluated index point {index_point:?}"));
        }
        if !self.cfg.top_level.points.is_empty()
            && !self.cfg.top_level.points.iter().any(|entry| {
                entry.point == event.point
                    && matches!(&entry.execution, ScalarTopLevelPointExecution::Connected {
                    occurrence: ScalarTopLevelOccurrence::Assignment(known), ..
                } if known == assignment.occurrence.as_ref())
            })
        {
            return Err(format!(
                "SelectElement {:?} lacks connected execution",
                event.point
            ));
        }
        Ok((event.point, *index_point))
    }
    pub(super) fn capture(
        &self,
        occurrence: &ScalarCaptureOccurrence,
    ) -> Result<&ReferenceFlowPoint, String> {
        let matches = self
            .cfg
            .capture_points
            .iter()
            .filter(|event| {
                event.occurrence == *occurrence && event.role == ScalarCapturePointRole::Capture
            })
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "expected one callable Capture for {occurrence:?}, found {}",
                matches.len()
            ));
        };
        let point = self.structural_point(event.point, HolderPointKind::Capture)?;
        if point.source_span != occurrence.source_span {
            return Err(format!("callable Capture {occurrence:?} has wrong source"));
        }
        if !point.successors.is_empty() {
            self.validated(point)?;
        }
        Ok(point)
    }

    pub(super) fn capture_initial_handoff(&self, point: &ReferenceFlowPoint) -> Result<(), String> {
        let CfgPointKind::Capture { environment, .. } = &point.kind else {
            return Err(format!(
                "expected source-qualified Capture at {:?}",
                point.source_span
            ));
        };
        let moves = self
            .cfg
            .holder_edges
            .iter()
            .flat_map(|fact| {
                fact.actions.iter().filter_map(move |action| match action {
                    CallableHolderAction::Move {
                        environment: actual,
                        from: CallableHolderPlace::Temporary(source),
                        ..
                    } if actual == environment && *source == point.id => Some((fact, action)),
                    CallableHolderAction::MoveSelected {
                        environment: actual,
                        from: CallableHolderPlace::Temporary(source),
                        ..
                    } if actual == environment && *source == point.id => Some((fact, action)),
                    _ => None,
                })
            })
            .collect::<Vec<_>>();
        if moves.is_empty() {
            return Err(format!(
                "Capture at {:?} lacks a connected initial-root Move from {:?}",
                point.source_span, point.id
            ));
        }
        for (fact, action) in moves {
            self.action(
                HolderEmissionEdge::Outgoing {
                    from: fact.from,
                    to: fact.to,
                },
                action,
            )?;
        }
        Ok(())
    }

    pub(super) fn literal_cfg(&self, capture: CfgPointId) -> Result<&ScalarReferenceCfg, String> {
        let matches = self
            .cfg
            .literal_cfgs
            .iter()
            .filter(|literal| literal.capture_point == capture)
            .collect::<Vec<_>>();
        let [literal] = matches.as_slice() else {
            return Err(format!(
                "expected one literal CFG for Capture {capture:?}, found {}",
                matches.len()
            ));
        };
        Ok(&literal.reference_cfg)
    }

    pub(super) fn binding(
        &self,
        binding: &ScalarBinding,
        output: usize,
        copy_required: bool,
    ) -> Result<(), String> {
        let occurrence = binding.occurrence.as_ref();
        let receiver = &binding.receivers[output];
        let facts = self
            .cfg
            .binding_points
            .iter()
            .filter(|event| event.occurrence == *occurrence && event.output == output)
            .collect::<Vec<_>>();
        let executions = self
            .cfg
            .top_level
            .bindings
            .iter()
            .filter(|entry| entry.occurrence == *occurrence && entry.output == output)
            .collect::<Vec<_>>();
        match executions.as_slice() {
            [] => {
                if facts
                    .iter()
                    .filter(|event| event.role == ScalarBindingPointRole::Bind)
                    .count()
                    != 1
                {
                    return Err(format!(
                        "expected local CFG Bind for {occurrence:?} output {output}"
                    ));
                }
            }
            [entry] => match &entry.execution {
                ScalarTopLevelBindingExecution::Connected { points } => {
                    if points.len() != facts.len()
                        || !facts.iter().all(|fact| points.contains(&fact.point))
                    {
                        return Err(format!("top-level binding {occurrence:?} output {output} has disconnected roles"));
                    }
                    for point in points {
                        if !matches!(&self.cfg.top_level.points[point.0].execution, ScalarTopLevelPointExecution::Connected { occurrence: ScalarTopLevelOccurrence::Binding(known), .. } if known == occurrence)
                        {
                            return Err(format!("top-level binding {occurrence:?} output {output} lacks connected execution at {point:?}"));
                        }
                    }
                }
                ScalarTopLevelBindingExecution::EmittedWithoutHolderAction
                    if facts.is_empty() && !copy_required =>
                {
                    return Ok(())
                }
                ScalarTopLevelBindingExecution::EmittedWithoutHolderAction
                | ScalarTopLevelBindingExecution::UnrepresentedHolderAction => {
                    return Err(format!("top-level binding {occurrence:?} output {output} lacks required holder role"));
                }
            },
            _ => {
                return Err(format!(
                    "ambiguous binding execution for {occurrence:?} output {output}"
                ))
            }
        }
        if copy_required
            && facts
                .iter()
                .filter(|event| event.role == ScalarBindingPointRole::CopyCallableFields)
                .count()
                != 1
        {
            return Err(format!(
                "binding {occurrence:?} output {output} lacks callable field copy"
            ));
        }
        for fact in facts {
            let kind = match &fact.role {
                ScalarBindingPointRole::Bind => HolderPointKind::Bind,
                ScalarBindingPointRole::CopyCallableFields => HolderPointKind::CopyCallableFields,
                ScalarBindingPointRole::FieldAssign(_) => HolderPointKind::Assign,
                ScalarBindingPointRole::ArrayElement(_) => HolderPointKind::BindArrayElement,
            };
            let point = self.structural_point(fact.point, kind)?;
            let target = match &point.kind {
                CfgPointKind::Bind { binding } => binding,
                CfgPointKind::CopyCallableFields { target, .. } => target,
                CfgPointKind::Assign { target, .. }
                | CfgPointKind::BindArrayElement { target, .. } => &target.binding,
                _ => return Err(format!("invalid binding role {fact:?}")),
            };
            if target.source != occurrence.source_span.source
                || target.declaration_span != receiver.name_span
            {
                return Err(format!(
                    "binding role {fact:?} targets a different receiver"
                ));
            }
            if let ScalarBindingPointRole::FieldAssign(field) = &fact.role {
                if !matches!(&point.kind, CfgPointKind::Assign { target, .. } if target.projections.contains(&ReferencePlaceProjection::Field(crate::scalar::ScalarFieldReference::Resolved(field.clone()))))
                {
                    return Err(format!(
                        "binding field role {fact:?} targets a different field"
                    ));
                }
            }
            if let ScalarBindingPointRole::ArrayElement(index) = &fact.role {
                if !matches!(&point.kind, CfgPointKind::BindArrayElement { target, .. } if target.projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }) if value == &(*index).into())))
                {
                    return Err(format!(
                        "binding array role {fact:?} targets a different element"
                    ));
                }
            }
            self.validated(point)?;
        }
        Ok(())
    }

    pub(super) fn evaluated_call(
        &self,
        occurrence: &ScalarCallOccurrence,
        output_types: &[ScalarType],
    ) -> Result<(), String> {
        let matches = self
            .cfg
            .call_points
            .iter()
            .filter(|call| call.occurrence == *occurrence)
            .collect::<Vec<_>>();
        let [call] = matches.as_slice() else {
            return Err(format!(
                "expected one evaluated callable occurrence {occurrence:?}, found {}",
                matches.len()
            ));
        };
        let evaluated = self.structural_point(call.evaluated_point, HolderPointKind::Call)?;
        if !matches!(&evaluated.kind, CfgPointKind::Call { expression: ScalarExpression::Call { occurrence: point_occurrence, .. }, output: 0, .. } if point_occurrence.as_ref() == occurrence)
        {
            return Err(format!(
                "evaluated callable occurrence {occurrence:?} has an unrelated point"
            ));
        }
        self.validated(evaluated)?;
        let mut outputs = Vec::with_capacity(output_types.len());
        for (index, ty) in output_types.iter().enumerate() {
            let matches = call
                .projections
                .iter()
                .filter(|projection| projection.output == index)
                .collect::<Vec<_>>();
            let output = match matches.as_slice() {
                [] => ScalarCallEmissionOutput::SignatureOnly { output: index, ty },
                [projection] => {
                    let point = self.structural_point(projection.point, HolderPointKind::Call)?;
                    if !matches!(&point.kind, CfgPointKind::Call { expression: ScalarExpression::Call { occurrence: point_occurrence, .. }, output, .. } if point_occurrence.as_ref() == occurrence && *output == index)
                    {
                        return Err(format!(
                            "callable projection {index} for {occurrence:?} has an unrelated point"
                        ));
                    }
                    if point.id != evaluated.id {
                        self.validated(point)?;
                    }
                    ScalarCallEmissionOutput::Projected {
                        output: index,
                        ty,
                        point,
                    }
                }
                _ => {
                    return Err(format!(
                        "ambiguous callable projection {index} for {occurrence:?}"
                    ))
                }
            };
            outputs.push(output);
        }
        if !matches!(outputs.first(), Some(ScalarCallEmissionOutput::Projected { point, .. }) if point.id == evaluated.id)
            || call.projections.len()
                != outputs
                    .iter()
                    .filter(|output| matches!(output, ScalarCallEmissionOutput::Projected { .. }))
                    .count()
        {
            return Err(format!(
                "invalid callable projection set for {occurrence:?}"
            ));
        }
        for (index, output) in outputs.iter().enumerate() {
            match output {
                ScalarCallEmissionOutput::Projected { output, ty, point } => {
                    if *output != index
                        || *ty != &output_types[index]
                        || !matches!(point.kind, CfgPointKind::Call { .. })
                    {
                        return Err(format!(
                            "invalid produced callable output {index} for {occurrence:?}"
                        ));
                    }
                }
                ScalarCallEmissionOutput::SignatureOnly { output, ty } => {
                    if *output != index || *ty != &output_types[index] {
                        return Err(format!(
                            "invalid signature-only callable output {index} for {occurrence:?}"
                        ));
                    }
                }
            }
        }
        self.last_evaluated_call.set(Some(call.evaluated_point.0));
        Ok(())
    }

    pub(super) fn imported_output_holder(
        &self,
        output: usize,
        fields: &[usize],
    ) -> Result<(crate::scalar::CfgPointId, CallableHolderPlace), String> {
        let evaluated = self
            .last_evaluated_call
            .get()
            .ok_or_else(|| "imported capture lacks a typed evaluated call occurrence".to_owned())?;
        let call = self
            .cfg
            .call_points
            .iter()
            .find(|call| call.evaluated_point.0 == evaluated)
            .ok_or_else(|| format!("imported capture lacks connected call {evaluated}"))?;
        let projections = call
            .projections
            .iter()
            .filter(|projection| projection.output == output)
            .collect::<Vec<_>>();
        let [projection] = projections.as_slice() else {
            return Err(format!(
                "imported capture call {:?} output {output} lacks one typed projection",
                call.occurrence
            ));
        };
        let point = self.structural_point(projection.point, HolderPointKind::Call)?;
        self.validated(point)?;
        let holder = if fields.is_empty() {
            CallableHolderPlace::Temporary(point.id)
        } else {
            let matches = point
                .possible_projected_environments
                .iter()
                .filter(|projection| {
                    let indices = projection
                        .place
                        .projections
                        .iter()
                        .map(|projection| match projection {
                            ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(
                                field,
                            )) => Ok(field.index),
                            _ => Err(()),
                        })
                        .collect::<Result<Vec<_>, _>>();
                    indices.as_deref() == Ok(fields)
                })
                .collect::<Vec<_>>();
            let [projection] = matches.as_slice() else {
                return Err(format!("imported capture call {:?} output {output} field {fields:?} lacks one scalar projected holder: found {}", call.occurrence, matches.len()));
            };
            CallableHolderPlace::Projection(projection.place.clone())
        };
        if !point.successors.iter().any(|successor| {
            self.outgoing(point.id, *successor).is_ok_and(|edges| {
                edges.iter().any(|edge| {
                    edge.after
                        .iter()
                        .any(|relation| relation.holders.contains(&holder))
                })
            })
        }) {
            return Err(format!("imported capture call {:?} output {output} field {fields:?} lacks a connected holder", call.occurrence));
        }
        Ok((point.id, holder))
    }

    fn structural_point(
        &self,
        id: CfgPointId,
        kind: HolderPointKind,
    ) -> Result<&ReferenceFlowPoint, String> {
        let point = self
            .cfg
            .points
            .get(id.0)
            .ok_or_else(|| format!("missing callable CFG point {id:?}"))?;
        if point.id != id || HolderPointKind::of(&point.kind) != Some(kind) {
            return Err(format!(
                "callable CFG point {id:?} has the wrong structural role {kind:?}"
            ));
        }
        let execution = self
            .cfg
            .top_level
            .points
            .iter()
            .filter(|entry| entry.point == id)
            .collect::<Vec<_>>();
        match execution.as_slice() {
            [] if self.cfg.top_level.points.is_empty() => {}
            [entry] => match &entry.execution {
                ScalarTopLevelPointExecution::Connected { outgoing, .. }
                    if outgoing == &point.successors => {}
                ScalarTopLevelPointExecution::Connected { .. } => {
                    return Err(format!(
                        "callable CFG point {id:?} has disconnected top-level edges"
                    ))
                }
                ScalarTopLevelPointExecution::SupplementalAnalysisOnly => {
                    return Err(format!(
                        "callable CFG point {id:?} is supplemental analysis only"
                    ))
                }
            },
            _ => {
                return Err(format!(
                    "callable CFG point {id:?} lacks unique top-level execution"
                ))
            }
        }
        Ok(point)
    }

    pub(super) fn while_point(
        &self,
        occurrence: &ScalarWhileOccurrence,
        role: ScalarWhilePointRole,
    ) -> Result<&ReferenceFlowPoint, String> {
        let matches = self
            .cfg
            .while_points
            .iter()
            .filter(|event| event.occurrence == *occurrence && event.role == role)
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "expected one callable while {role:?} for {occurrence:?}, found {}",
                matches.len()
            ));
        };
        let kind = match role {
            ScalarWhilePointRole::LoopTest => HolderPointKind::LoopTest,
            ScalarWhilePointRole::Branch => HolderPointKind::Branch,
            ScalarWhilePointRole::BodyScopeExit => HolderPointKind::ScopeExit,
        };
        self.structural_point(event.point, kind)
    }

    pub(super) fn conditional_point(
        &self,
        then_block: &ScalarBlockOccurrence,
        role: ScalarConditionalPointRole,
    ) -> Result<&ReferenceFlowPoint, String> {
        let then_block = self.source_block_occurrence(then_block)?;
        let matches = self
            .cfg
            .conditional_points
            .iter()
            .filter(|event| event.then_block == *then_block && event.role == role)
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "expected one callable conditional {role:?} for {then_block:?}, found {}",
                matches.len()
            ));
        };
        let kind = match role {
            ScalarConditionalPointRole::Branch => HolderPointKind::Branch,
            ScalarConditionalPointRole::ThenScopeExit
            | ScalarConditionalPointRole::ElseScopeExit => HolderPointKind::ScopeExit,
        };
        self.structural_point(event.point, kind)
    }

    pub(super) fn block_exit(
        &self,
        occurrence: &ScalarBlockOccurrence,
    ) -> Result<&ReferenceFlowPoint, String> {
        let occurrence = self.source_block_occurrence(occurrence)?;
        let matches = self
            .cfg
            .block_scope_exits
            .iter()
            .filter(|event| event.occurrence == *occurrence)
            .collect::<Vec<_>>();
        let [event] = matches.as_slice() else {
            return Err(format!(
                "expected one callable scope exit for {occurrence:?}, found {}",
                matches.len()
            ));
        };
        self.structural_point(event.point, HolderPointKind::ScopeExit)
    }

    fn source_block_occurrence<'a>(
        &'a self,
        selected: &'a ScalarBlockOccurrence,
    ) -> Result<&'a ScalarBlockOccurrence, String> {
        let mut source = selected;
        loop {
            let aliases = self.cfg.evaluated_values.block_output_aliases.iter()
                .filter(|alias| alias.selected == *source).collect::<Vec<_>>();
            match aliases.as_slice() {
                [] => return Ok(source),
                [alias] if alias.selected.source_span == alias.source.source_span
                    && alias.selected.output_projection.len() == alias.source.output_projection.len() + 1
                    && alias.selected.output_projection[1..] == alias.source.output_projection => {
                    source = &alias.source;
                }
                _ => return Err(format!("invalid selected block output correspondence for {selected:?}")),
            }
        }
    }

    pub(super) fn new(cfg: &ScalarReferenceCfg) -> Result<Self, String> {
        let mut covered_cells = HashMap::new();
        let returned_literals = cfg
            .holder_edges
            .iter()
            .flat_map(|fact| &fact.actions)
            .filter_map(|action| match action {
                CallableHolderAction::ReturnTransfer { environment, .. } => {
                    Some(&environment.literal)
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        for (edge, action) in cfg
            .holder_edges
            .iter()
            .flat_map(|fact| {
                fact.actions.iter().cloned().map(|action| {
                    (
                        HolderEmissionEdge::Outgoing {
                            from: fact.from,
                            to: fact.to,
                        },
                        action,
                    )
                })
            })
            .chain(cfg.holder_exits.iter().flat_map(|fact| {
                fact.actions
                    .iter()
                    .cloned()
                    .map(|action| (HolderEmissionEdge::Terminal { from: fact.from }, action))
            }))
        {
            let CallableHolderAction::DropCaptureCellOwner {
                binding, backing, ..
            } = &action
            else {
                continue;
            };
            let literals = cfg
                .literal_cfgs
                .iter()
                .filter(|literal| {
                    literal
                        .captures
                        .iter()
                        .any(|capture| capture.binding == *binding)
                })
                .map(|literal| literal.literal.clone())
                .collect::<Vec<_>>();
            let owner_actions = cfg.holder_edges.iter().flat_map(|fact| &fact.actions)
                .chain(cfg.holder_exits.iter().flat_map(|fact| &fact.actions))
                .filter(|action| matches!(action, CallableHolderAction::DropCaptureCellOwner { binding: actual, .. } if actual == binding))
                .count();
            if literals.is_empty() || returned_literals.len() < 2 || owner_actions != 1 {
                continue;
            }
            let mut environments = cfg
                .holder_edges
                .iter()
                .flat_map(|fact| &fact.actions)
                .filter_map(|action| match action {
                    CallableHolderAction::ReturnTransfer { environment, .. }
                        if literals.contains(&environment.literal) =>
                    {
                        Some(environment.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            environments.sort();
            environments.dedup();
            if environments.is_empty() {
                continue;
            }
            let mut actions = cfg
                .holder_edges
                .iter()
                .flat_map(|fact| {
                    fact.actions.iter().filter_map(|action| {
                        let environment = match action {
                            CallableHolderAction::Retain { environment, .. }
                            | CallableHolderAction::Move { environment, .. }
                            | CallableHolderAction::Drop { environment, .. }
                            | CallableHolderAction::ReturnTransfer { environment, .. } => {
                                environment
                            }
                            _ => return None,
                        };
                        environments.contains(environment).then(|| {
                            (
                                HolderEmissionEdge::Outgoing {
                                    from: fact.from,
                                    to: fact.to,
                                },
                                action.clone(),
                            )
                        })
                    })
                })
                .collect::<Vec<_>>();
            actions.extend(cfg.holder_exits.iter().flat_map(|fact| {
                fact.actions.iter().filter_map(|action| match action {
                    CallableHolderAction::Drop { environment, .. }
                        if environments.contains(environment) =>
                    {
                        Some((
                            HolderEmissionEdge::Terminal { from: fact.from },
                            action.clone(),
                        ))
                    }
                    _ => None,
                })
            }));
            let mut backings = vec![backing.clone()];
            for action in cfg.holder_edges.iter().flat_map(|fact| &fact.actions) {
                if let CallableHolderAction::ReplaceCaptureCellValue {
                    binding: target,
                    previous,
                    incoming,
                    ..
                } = action
                {
                    if target == binding {
                        for fact in [previous, incoming] {
                            if !backings.contains(fact) {
                                backings.push(fact.clone());
                            }
                        }
                    }
                }
            }
            for environment in &environments {
                let producer = cfg.environments.get(environment.index)
                    .filter(|fact| fact.literal == environment.literal)
                    .ok_or_else(|| format!("captured cell {binding:?} lacks source-qualified producer {environment:?}"))?;
                if !producer.inputs.iter().any(|input| {
                    input.binding == *binding
                        && input
                            .backings
                            .iter()
                            .any(|candidate| backings.contains(candidate))
                }) {
                    return Err(format!(
                        "captured cell {binding:?} lacks producer backing for {environment:?}"
                    ));
                }
                if !actions.iter().any(|(_, action)| matches!(action,
                    CallableHolderAction::Move { environment: actual, from: CallableHolderPlace::Temporary(_), .. }
                    if actual == environment)) {
                    return Err(format!("captured cell {binding:?} lacks connected literal-holder move for {environment:?}"));
                }
                if !actions.iter().any(|(_, action)| {
                    matches!(action,
                    CallableHolderAction::ReturnTransfer { environment: actual, .. }
                    if actual == environment)
                }) {
                    return Err(format!(
                        "captured cell {binding:?} lacks return transfer for {environment:?}"
                    ));
                }
            }
            if let Some(coverage) = covered_cells.get_mut(binding) {
                let coverage: &mut CaptureCoverage = coverage;
                if !coverage.backings.contains(backing) {
                    return Err(format!(
                        "capture-cell owner exit for {binding:?} has unrelated backing"
                    ));
                }
                if !coverage.owner_edges.contains(&edge) {
                    coverage.owner_edges.push(edge);
                }
                for environment in environments {
                    if !coverage.environments.contains(&environment) {
                        coverage.environments.push(environment);
                    }
                }
                for action in actions {
                    if !coverage.actions.contains(&action) {
                        coverage.actions.push(action);
                    }
                }
                for fact in backings {
                    if !coverage.backings.contains(&fact) {
                        coverage.backings.push(fact);
                    }
                }
            } else {
                covered_cells.insert(
                    binding.clone(),
                    CaptureCoverage {
                        backing: backing.clone(),
                        backings,
                        environments,
                        owner_edges: vec![edge],
                        actions,
                    },
                );
            }
        }
        let mut points: HashMap<_, Vec<CfgPointId>> = HashMap::new();
        for point in &cfg.points {
            if let Some(kind) = HolderPointKind::of(&point.kind) {
                points
                    .entry((point.source_span.clone(), kind))
                    .or_default()
                    .push(point.id);
            }
        }
        let mut edges: HashMap<_, Vec<CallableHolderEdgeFact>> = HashMap::new();
        for edge in &cfg.holder_edges {
            edges
                .entry((edge.from, edge.to))
                .or_default()
                .push(edge.clone());
        }
        let mut exits: HashMap<_, Vec<CallableHolderExitFact>> = HashMap::new();
        for exit in &cfg.holder_exits {
            exits.entry(exit.from).or_default().push(exit.clone());
        }
        let events = Self {
            cfg: cfg.clone(),
            points,
            edges,
            exits,
            selected: HashMap::new(),
            reads: HashMap::new(),
            outputs: HashMap::new(),
            bindings: HashMap::new(),
            imported_holders: HashMap::new(),
            last_evaluated_call: Cell::new(None),
            covered_cells,
            saved_assignment_outputs: Vec::new(),
            saved_assignment_runtime_lengths: Vec::new(),
        };
        for (binding, coverage) in &events.covered_cells {
            for edge in &coverage.owner_edges {
                let owner = events.connected_actions(*edge)?;
                if !owner.iter().any(|action| {
                    matches!(action,
                    CallableHolderAction::DropCaptureCellOwner { binding: actual, backing, .. }
                    if actual == binding && coverage.backings.contains(backing))
                }) {
                    return Err(format!(
                        "captured cell {binding:?} lacks connected release owner at {edge:?}"
                    ));
                }
            }
            for (edge, action) in &coverage.actions {
                events.action(*edge, action)?;
            }
        }
        Ok(events)
    }

    pub(super) fn point(
        &self,
        source: &wosy_syntax::SourceIdentity,
        span: wosy_syntax::ByteSpan,
        kind: HolderPointKind,
    ) -> Result<&ReferenceFlowPoint, String> {
        let key = (wosy_syntax::SourceSpan::new(source.clone(), span), kind);
        let ids = self
            .points
            .get(&key)
            .ok_or_else(|| format!("missing callable CFG {kind:?} at {:?}", key.0))?;
        let [id] = ids.as_slice() else {
            return Err(format!(
                "ambiguous callable CFG {kind:?} at {:?}: {ids:?}",
                key.0
            ));
        };
        self.cfg
            .points
            .get(id.0)
            .ok_or_else(|| format!("missing callable CFG point {id:?}"))
    }

    pub(super) fn return_output(
        &self,
        source: &wosy_syntax::SourceIdentity,
        span: wosy_syntax::ByteSpan,
        output: usize,
    ) -> Result<&ReferenceFlowPoint, String> {
        let key = (
            wosy_syntax::SourceSpan::new(source.clone(), span),
            HolderPointKind::ReturnOutput,
        );
        let ids = self
            .points
            .get(&key)
            .ok_or_else(|| format!("missing callable CFG return at {:?} output {output}", key.0))?;
        let matches = ids.iter().filter(|id| matches!(self.cfg.points[id.0].kind, CfgPointKind::ReturnOutput { output: index, .. } if index == output)).collect::<Vec<_>>();
        let [id] = matches.as_slice() else {
            return Err(format!(
                "ambiguous callable CFG return at {:?} output {output}: {matches:?}",
                key.0
            ));
        };
        Ok(&self.cfg.points[id.0])
    }

    pub(super) fn validated(&self, point: &ReferenceFlowPoint) -> Result<(), String> {
        if point.successors.is_empty() {
            self.exits(point.id)?;
        } else {
            for successor in &point.successors {
                self.outgoing(point.id, *successor)?;
            }
        }
        Ok(())
    }

    pub(super) fn outgoing(
        &self,
        from: CfgPointId,
        to: CfgPointId,
    ) -> Result<&[CallableHolderEdgeFact], String> {
        self.edges
            .get(&(from, to))
            .map(Vec::as_slice)
            .ok_or_else(|| format!("missing callable holder edge {from:?} -> {to:?}"))
    }

    pub(super) fn arm(
        &self,
        branch: CfgPointId,
        arm: usize,
    ) -> Result<&[CallableHolderEdgeFact], String> {
        let point = self
            .cfg
            .points
            .get(branch.0)
            .ok_or_else(|| format!("missing callable branch {branch:?}"))?;
        if !matches!(point.kind, CfgPointKind::Branch { .. }) || point.successors.len() != 2 {
            return Err(format!(
                "callable branch {branch:?} lacks two CFG successors"
            ));
        }
        let successor = point
            .successors
            .get(arm)
            .ok_or_else(|| format!("invalid callable branch arm {arm}"))?;
        self.outgoing(branch, *successor)
    }

    pub(super) fn exits(&self, point: CfgPointId) -> Result<&[CallableHolderExitFact], String> {
        self.exits
            .get(&point)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("missing callable holder exit {point:?}"))
    }
}
