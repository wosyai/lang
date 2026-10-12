use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssignmentCorrespondenceError {
    pub source_span: SourceSpan,
    pub invariant: AssignmentCorrespondenceInvariant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentCorrespondenceInvariant {
    Operation,
    DestinationCount,
    Ordinal,
    Source,
    Shape,
    StorageRole,
    DirectPlace,
    InferredBinding,
    PointerIdentity,
    OperandIdentity,
    OperandOutput,
    OperandType,
    ValueFact,
    ProducerPoint,
    DestinationEvent,
    Connection,
}

fn reaches(
    points: &[ReferenceFlowPoint],
    from: CfgPointId,
    to: CfgPointId,
    excluded: &BTreeSet<usize>,
) -> bool {
    let mut pending = vec![from];
    let mut visited = excluded.clone();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.0) {
            continue;
        }
        if id == to {
            return true;
        }
        if let Some(point) = points.get(id.0) {
            pending.extend(&point.successors);
        }
    }
    false
}

fn projected_type(
    place: &ScalarPlace,
    pointer_type: &ScalarType,
    structs: &[ScalarStruct],
) -> Option<ScalarType> {
    match place {
        ScalarPlace::Dereference { .. } => match pointer_type {
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable,
                inner,
            }
            | ScalarType::RawPointer(inner) => Some(inner.as_ref().clone()),
            _ => None,
        },
        ScalarPlace::Field {
            base,
            field: ScalarFieldReference::Resolved(field),
            ..
        } => {
            let ScalarType::Struct(structure) = projected_type(base, pointer_type, structs)? else {
                return None;
            };
            if structure != field.structure {
                return None;
            }
            structs
                .iter()
                .find(|item| item.id == structure)?
                .fields
                .iter()
                .find(|item| item.id == *field)
                .map(|item| item.ty.clone())
        }
        ScalarPlace::Index { base, .. } => match projected_type(base, pointer_type, structs)? {
            ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                Some(*element)
            }
            _ => None,
        },
        ScalarPlace::Name { .. }
        | ScalarPlace::Field {
            field: ScalarFieldReference::Unresolved { .. },
            ..
        } => None,
    }
}

fn pointer_operand(place: &ScalarPlace) -> Option<&ScalarExpression> {
    match place {
        ScalarPlace::Dereference { pointer, .. } => Some(pointer),
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => pointer_operand(base),
        ScalarPlace::Name { .. } => None,
    }
}

pub fn validate_assignment_destinations(
    cfg: &ScalarReferenceCfg,
    plan: &ResolvedAssignmentPlan,
    evaluation: &ScalarAssignmentEvaluation,
    outputs: &[ResolvedExpressionOutputFact],
    structs: &[ScalarStruct],
) -> Result<(), AssignmentCorrespondenceError> {
    let operation_error = |invariant| AssignmentCorrespondenceError {
        source_span: plan.operation.occurrence.source_span.clone(),
        invariant,
    };
    if plan.operation != evaluation.operation || plan.operation.producer != cfg.producer_owner {
        return Err(operation_error(
            AssignmentCorrespondenceInvariant::Operation,
        ));
    }
    if plan.targets.len() != evaluation.destinations.len() {
        return Err(operation_error(
            AssignmentCorrespondenceInvariant::DestinationCount,
        ));
    }
    for (ordinal, (target, destination)) in plan
        .targets
        .iter()
        .zip(&evaluation.destinations)
        .enumerate()
    {
        let error = |invariant| AssignmentCorrespondenceError {
            source_span: target.source_span.clone(),
            invariant,
        };
        if target.ordinal != ordinal
            || destination.id.target != ordinal
            || destination.id.operation != plan.operation
        {
            return Err(error(AssignmentCorrespondenceInvariant::Ordinal));
        }
        if destination.source_span != target.source_span
            || target.source_span.source != plan.operation.occurrence.source_span.source
        {
            return Err(error(AssignmentCorrespondenceInvariant::Source));
        }
        let shape = cfg
            .evaluated_values
            .shapes
            .iter()
            .find(|shape| shape.id == destination.shape)
            .ok_or_else(|| error(AssignmentCorrespondenceInvariant::Shape))?;
        if !scalar_type_equal(&shape.ty, &target.ty) || shape.id.producer != plan.operation.producer {
            return Err(error(AssignmentCorrespondenceInvariant::Shape));
        }
        let event = cfg
            .points
            .get(destination.point.0)
            .filter(|point| point.id == destination.point)
            .ok_or_else(|| error(AssignmentCorrespondenceInvariant::DestinationEvent))?;
        if event.source_span != target.source_span {
            return Err(error(AssignmentCorrespondenceInvariant::Source));
        }
        let CfgPointKind::AssignmentDestination { storage, place } = &event.kind else {
            return Err(error(AssignmentCorrespondenceInvariant::DestinationEvent));
        };
        if *place != target.place || *storage != destination.storage {
            return Err(error(AssignmentCorrespondenceInvariant::DestinationEvent));
        }
        if !reaches(
            &cfg.points,
            CfgPointId(0),
            destination.point,
            &BTreeSet::new(),
        ) {
            return Err(error(AssignmentCorrespondenceInvariant::Connection));
        }
        match (&target.storage, &destination.storage) {
            (
                ResolvedAssignmentStorage::Existing(resolved),
                EvaluatedDestinationStorage::Direct(evaluated),
            ) => {
                if pointer_operand(&target.place).is_some() {
                    return Err(error(AssignmentCorrespondenceInvariant::StorageRole));
                }
                if resolved != evaluated {
                    return Err(error(AssignmentCorrespondenceInvariant::DirectPlace));
                }
            }
            (
                ResolvedAssignmentStorage::InferredDeclaration(resolved),
                EvaluatedDestinationStorage::Inferred(evaluated),
            ) => {
                if !matches!(target.place, ScalarPlace::Name { .. }) {
                    return Err(error(AssignmentCorrespondenceInvariant::StorageRole));
                }
                if resolved != evaluated {
                    return Err(error(AssignmentCorrespondenceInvariant::InferredBinding));
                }
            }
            (
                ResolvedAssignmentStorage::Through { pointer: resolved },
                EvaluatedDestinationStorage::Through { pointer, .. },
            ) => {
                if *resolved != pointer.resolved {
                    return Err(error(AssignmentCorrespondenceInvariant::PointerIdentity));
                }
                let operand = pointer_operand(&target.place)
                    .ok_or_else(|| error(AssignmentCorrespondenceInvariant::StorageRole))?;
                let source_span =
                    SourceSpan::new(target.source_span.source.clone(), expression_span(operand));
                let projection = match operand {
                    ScalarExpression::Assignment(assignment) => {
                        assignment.occurrence.output_projection.clone()
                    }
                    ScalarExpression::Call { occurrence, .. } => {
                        occurrence.output_projection.clone()
                    }
                    ScalarExpression::Block(block) => block.occurrence.output_projection.clone(),
                    ScalarExpression::Dereference { occurrence, .. }
                    | ScalarExpression::IndexedRead { occurrence, .. } => {
                        occurrence.output_projection.clone()
                    }
                    _ => Vec::new(),
                };
                if resolved.producer != plan.operation.producer
                    || resolved.occurrence != source_span
                    || resolved.output_projection != projection
                {
                    return Err(error(AssignmentCorrespondenceInvariant::OperandIdentity));
                }
                let fact = outputs
                    .iter()
                    .find(|fact| fact.source_span == source_span)
                    .ok_or_else(|| error(AssignmentCorrespondenceInvariant::OperandOutput))?;
                if resolved.output != 0 {
                    return Err(error(AssignmentCorrespondenceInvariant::OperandOutput));
                }
                let output = fact
                    .outputs
                    .outputs
                    .get(resolved.output)
                    .ok_or_else(|| error(AssignmentCorrespondenceInvariant::OperandOutput))?;
                if !projected_type(&target.place, &output.ty, structs).is_some_and(|ty| scalar_type_equal(&ty, &target.ty)) {
                    return Err(error(AssignmentCorrespondenceInvariant::OperandType));
                }
                let mut values = cfg
                    .evaluated_values
                    .values
                    .iter()
                    .filter(|value| value.id == *pointer);
                let value = values
                    .next()
                    .ok_or_else(|| error(AssignmentCorrespondenceInvariant::ValueFact))?;
                if values.next().is_some() {
                    return Err(error(AssignmentCorrespondenceInvariant::ValueFact));
                }
                let shape = cfg
                    .evaluated_values
                    .shapes
                    .iter()
                    .find(|shape| shape.id == value.shape)
                    .ok_or_else(|| error(AssignmentCorrespondenceInvariant::OperandType))?;
                if !scalar_type_equal(&shape.ty, &output.ty) || shape.id.producer != plan.operation.producer {
                    return Err(error(AssignmentCorrespondenceInvariant::OperandType));
                }
                let point = cfg
                    .points
                    .get(pointer.point.0)
                    .filter(|point| point.id == pointer.point)
                    .ok_or_else(|| error(AssignmentCorrespondenceInvariant::ProducerPoint))?;
                if point.source_span != source_span {
                    return Err(error(AssignmentCorrespondenceInvariant::ProducerPoint));
                }
                if let ScalarExpression::Assignment(assignment) = operand {
                    let result = cfg
                        .evaluated_values
                        .results
                        .iter()
                        .find(|result| {
                            result.operation.occurrence == *assignment.occurrence
                                && result.output == resolved.output
                        })
                        .ok_or_else(|| error(AssignmentCorrespondenceInvariant::ProducerPoint))?;
                    if result.value != *pointer
                        || !matches!(point.kind, CfgPointKind::AssignmentResult { snapshot_point, output } if snapshot_point == result.snapshot && output == result.output)
                    {
                        return Err(error(AssignmentCorrespondenceInvariant::ProducerPoint));
                    }
                }
                if !reaches(&cfg.points, CfgPointId(0), pointer.point, &BTreeSet::new())
                    || !reaches(
                        &cfg.points,
                        pointer.point,
                        destination.point,
                        &BTreeSet::new(),
                    )
                    || reaches(
                        &cfg.points,
                        CfgPointId(0),
                        destination.point,
                        &BTreeSet::from([pointer.point.0]),
                    )
                {
                    return Err(error(AssignmentCorrespondenceInvariant::Connection));
                }
            }
            _ => return Err(error(AssignmentCorrespondenceInvariant::StorageRole)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
