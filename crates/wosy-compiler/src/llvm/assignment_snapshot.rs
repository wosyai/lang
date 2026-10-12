use super::*;
use crate::scalar::{
    AssignmentConsumption, AssignmentDestinationId, AssignmentOperationId, AssignmentResult,
    AssignmentSnapshot, EvaluatedDestinationStorage, EvaluatedValueFact, EvaluatedValueId,
    ResolvedAssignmentPlan, ResolvedValueShape, ScalarAssignmentEvaluation, SnapshotState,
};

pub(super) struct PreparedAssignmentOutputs {
    operation: AssignmentOperationId,
    outputs: Vec<EvaluatedValueId>,
}

pub(super) struct SavedAssignmentOutput<'ctx> {
    pub snapshot: AssignmentSnapshot,
    pub result: AssignmentResult,
    pub shape: ResolvedValueShape,
    pub fact: EvaluatedValueFact,
    pub payload: EmitValue<'ctx>,
}

pub(super) struct SavedAssignmentRuntimeLength<'ctx> {
    pub snapshot: EvaluatedValueId,
    pub result: EvaluatedValueId,
    pub destination: AssignmentDestinationId,
    pub storage: EvaluatedDestinationStorage,
    pub length: inkwell::values::IntValue<'ctx>,
}

fn snapshot_runtime_length<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    plan: &ResolvedAssignmentPlan,
    evaluation: &ScalarAssignmentEvaluation,
    snapshot: &AssignmentSnapshot,
    result: &AssignmentResult,
    frozen: &FrozenAssignmentDestination<'ctx>,
) -> Result<SavedAssignmentRuntimeLength<'ctx>, String> {
    let destination = evaluation
        .destinations
        .get(snapshot.destination.target)
        .filter(|destination| destination.id == snapshot.destination)
        .ok_or_else(|| format!("missing evaluated-length destination at {snapshot:?}"))?;
    let target = &plan.targets[snapshot.destination.target];
    let EvaluatedDestinationStorage::Direct(place) = &destination.storage else {
        return Err(format!(
            "missing evaluated-length for unsupported storage at {snapshot:?}"
        ));
    };
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Err(format!(
            "missing evaluated-length for projected/global storage at {snapshot:?}"
        ));
    };
    if !place.projections.is_empty()
        || place.place != target.place
        || place.source != destination.source_span.source
        || place.source != state.source
        || place.binding.source != place.source
        || place.binding.function_span != place.function_span
        || place.binding.declaration_span != place.declaration_span
        || place.binding.block_span != place.block_span
        || destination.id.operation != plan.operation
        || destination.shape != state.holder_events.assignment_value(&result.value)?.1.id
    {
        return Err(format!(
            "missing evaluated-length qualified direct correspondence at {snapshot:?}"
        ));
    }
    let FrozenAssignmentDestination::Storage {
        address,
        selection: FrozenAssignmentSelection::Ordinary,
    } = frozen
    else {
        return Err(format!(
            "missing evaluated-length frozen storage at {snapshot:?}"
        ));
    };
    let (slot, ty) = state
        .storage
        .get(name)
        .ok_or_else(|| format!("missing evaluated-length direct Name storage at {snapshot:?}"))?;
    if slot != address || *ty != target.ty {
        return Err(format!(
            "missing evaluated-length frozen address/shape correspondence at {snapshot:?}"
        ));
    }
    let length_slot = state
        .runtime_array_lengths
        .get(name)
        .ok_or_else(|| format!("missing evaluated-length slot at {snapshot:?}"))?;
    let length = state
        .builder
        .build_load(
            context.i64_type(),
            *length_slot,
            "assignment_previous_length",
        )
        .map_err(builder_error)?
        .into_int_value();
    let SnapshotState::Saved(id) = &snapshot.state else {
        return Err(format!(
            "missing evaluated-length saved identity at {snapshot:?}"
        ));
    };
    Ok(SavedAssignmentRuntimeLength {
        snapshot: id.clone(),
        result: result.value.clone(),
        destination: destination.id.clone(),
        storage: destination.storage.clone(),
        length,
    })
}

pub(super) fn snapshot_assignment_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    plan: &ResolvedAssignmentPlan,
    evaluation: &ScalarAssignmentEvaluation,
    destinations: &[FrozenAssignmentDestination<'ctx>],
) -> Result<PreparedAssignmentOutputs, String> {
    if plan.operation != evaluation.operation {
        return Err(format!(
            "assignment operation differs for {:?}",
            plan.operation
        ));
    }
    if destinations.len() != evaluation.destinations.len() {
        return Err(format!(
            "frozen destination count differs for {:?}",
            plan.operation
        ));
    }
    if matches!(plan.consumption, AssignmentConsumption::Discarded) {
        let prepared = PreparedAssignmentOutputs {
            operation: plan.operation.clone(),
            outputs: Vec::new(),
        };
        validate_prepared_outputs(plan, evaluation, &prepared)?;
        return Ok(prepared);
    }
    let mut saved = Vec::with_capacity(evaluation.results.len());
    let mut lengths = Vec::new();
    for snapshot in &evaluation.previous {
        match &snapshot.state {
            SnapshotState::Saved(id) => {
                let (_, snapshot_shape) = state.holder_events.assignment_value(id)?;
                let result = evaluation
                    .results
                    .iter()
                    .find(|result| result.snapshot == snapshot.point)
                    .ok_or_else(|| format!("saved snapshot {snapshot:?} lacks its actual result"))?
                    .clone();
                let (fact, shape) = state.holder_events.assignment_value(&result.value)?;
                if shape.id != snapshot_shape.id {
                    return Err(format!(
                        "saved snapshot/result shape differs for {result:?}"
                    ));
                }
                let fact = fact.clone();
                let shape = shape.clone();
                let destination = &destinations[snapshot.destination.target];
                if matches!(shape.ty, ScalarType::RuntimeArray { .. }) {
                    lengths.push(snapshot_runtime_length(
                        context,
                        state,
                        plan,
                        evaluation,
                        snapshot,
                        &result,
                        destination,
                    )?);
                }
                let payload = match destination {
                    FrozenAssignmentDestination::Unit if shape.ty == ScalarType::Unit => {
                        EmitValue::Unit
                    }
                    FrozenAssignmentDestination::Storage { address, .. }
                        if shape.ty != ScalarType::Unit =>
                    {
                        let locked = memory::has_callable_holders(&shape.ty, state.structs)?;
                        if locked {
                            ownership::begin_graph_mutation(&state.builder, state.module)?;
                        }
                        let storage =
                            storage_type(context, &shape.ty, state.structs, state.target_layout)?;
                        let bits = state
                            .builder
                            .build_load(storage, *address, "assignment_previous")
                            .map_err(builder_error)?;
                        let payload =
                            if matches!(shape.ty, ScalarType::Struct(_) | ScalarType::Array { .. })
                            {
                                let slot =
                                    entry_alloca(state, storage, "assignment_previous_storage")?;
                                state
                                    .builder
                                    .build_store(slot, bits)
                                    .map_err(builder_error)?;
                                EmitValue::Basic(slot.into())
                            } else {
                                EmitValue::Basic(bits)
                            };
                        if locked {
                            ownership::end_graph_mutation(&state.builder, state.module)?;
                        }
                        payload
                    }
                    _ => {
                        return Err(format!(
                            "frozen destination has inconsistent saved shape for {snapshot:?}"
                        ))
                    }
                };
                saved.push(SavedAssignmentOutput {
                    snapshot: snapshot.clone(),
                    result,
                    shape,
                    fact,
                    payload,
                });
            }
            SnapshotState::NoPriorDeclaration => {}
            SnapshotState::ElidedDiscard { .. } => {
                return Err(format!(
                    "observed assignment has elided snapshot {snapshot:?}"
                ))
            }
            SnapshotState::Unavailable { .. } => {
                return Err(format!(
                    "observed assignment has unavailable snapshot {snapshot:?}"
                ))
            }
        }
    }
    let prepared = PreparedAssignmentOutputs {
        operation: plan.operation.clone(),
        outputs: saved
            .iter()
            .map(|output| output.result.value.clone())
            .collect(),
    };
    validate_prepared_outputs(plan, evaluation, &prepared)?;
    for (output, result) in saved.iter().zip(&evaluation.results) {
        let SnapshotState::Saved(snapshot_id) = &output.snapshot.state else {
            return Err(format!("emitted result has no saved snapshot {result:?}"));
        };
        let (fact, shape) = state.holder_events.assignment_value(&result.value)?;
        if output.result != *result
            || output.snapshot.destination.operation != plan.operation
            || output.result.operation != plan.operation
            || output.fact != *fact
            || output.shape != *shape
            || output.snapshot.point != result.snapshot
            || snapshot_id.point != output.snapshot.point
            || state.holder_events.assignment_value(snapshot_id)?.1.id != output.shape.id
        {
            return Err(format!(
                "saved snapshot/result correspondence differs for {result:?}"
            ));
        }
        if state
            .holder_events
            .saved_assignment_output(&result.value)
            .is_ok()
        {
            return Err(format!(
                "duplicate emitted saved assignment result {:?}",
                result.value
            ));
        }
    }
    state
        .holder_events
        .validate_saved_assignment_runtime_lengths(&lengths)?;
    for output in saved {
        state.holder_events.record_saved_assignment_output(output)?;
    }
    state
        .holder_events
        .record_saved_assignment_runtime_lengths(lengths)?;
    Ok(prepared)
}

fn validate_prepared_outputs(
    plan: &ResolvedAssignmentPlan,
    evaluation: &ScalarAssignmentEvaluation,
    prepared: &PreparedAssignmentOutputs,
) -> Result<(), String> {
    if prepared.operation != plan.operation || evaluation.operation != plan.operation {
        return Err(format!(
            "prepared assignment operation differs for {:?}",
            plan.operation
        ));
    }
    if prepared.outputs.len() != evaluation.results.len() {
        return Err(format!(
            "saved output count differs for {:?}",
            plan.operation
        ));
    }
    for (position, (id, result)) in prepared.outputs.iter().zip(&evaluation.results).enumerate() {
        if result.operation != prepared.operation
            || result.output != position
            || *id != result.value
        {
            return Err(format!(
                "prepared assignment output order/ID differs for {result:?}"
            ));
        }
        if prepared.outputs[..position].contains(id) {
            return Err(format!("duplicate emitted saved assignment result {id:?}"));
        }
    }
    Ok(())
}

pub(super) fn finish_assignment_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    plan: &ResolvedAssignmentPlan,
    evaluation: &ScalarAssignmentEvaluation,
    prepared: PreparedAssignmentOutputs,
) -> Result<EmitValue<'ctx>, String> {
    validate_prepared_outputs(plan, evaluation, &prepared)?;
    if matches!(plan.consumption, AssignmentConsumption::Discarded) {
        return Ok(EmitValue::Unit);
    }
    let mut payloads = Vec::with_capacity(prepared.outputs.len());
    let mut outputs = Vec::with_capacity(prepared.outputs.len());
    for (id, result) in prepared.outputs.iter().zip(&evaluation.results) {
        let output = state.holder_events.saved_assignment_output(id)?;
        let (fact, shape) = state.holder_events.assignment_value(id)?;
        if output.result != *result || output.fact != *fact || output.shape != *shape {
            return Err(format!(
                "emitted saved result differs from actual facts {result:?}"
            ));
        }
        if matches!(shape.ty, ScalarType::RuntimeArray { .. }) {
            state.holder_events.saved_assignment_runtime_length(id)?;
        }
        payloads.push(output.payload.clone());
        outputs.push(crate::ScalarOutput {
            ty: output.shape.ty.clone(),
            span: output.result.value.resolved.occurrence.range,
        });
    }
    match payloads.len() {
        0 => Ok(EmitValue::Unit),
        1 => Ok(payloads.remove(0)),
        _ => build_aggregate(
            context,
            state,
            &crate::ScalarOutputSequence {
                outputs,
                span: plan.operation.occurrence.source_span.range,
            },
            payloads,
        ),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "assignment_snapshot/lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "assignment_snapshot/runtime_length_tests.rs"]
mod runtime_length_tests;
