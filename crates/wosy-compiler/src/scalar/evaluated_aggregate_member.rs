use super::*;

impl reference::ReferenceFlowBuilder {
    pub(super) fn register_literal_aggregate_root(
        &mut self,
        expression: &ScalarExpression,
        actual_root: CfgPointId,
    ) -> Result<(), ReferenceAnalysisError> {
        let source = SourceSpan::new(self.source.clone(), expression_span(expression));
        let sequence = self.bind_final_output_values(expression, actual_root)?;
        let [entry] = sequence.entries.as_slice() else {
            return Err(invalid(&source));
        };
        let root = self
            .cfg
            .points
            .get(actual_root.0)
            .ok_or_else(|| invalid(&source))?;
        let root_shape = shape(&self.cfg, &entry.shape, &source)?;
        if root.id != actual_root
            || root.source_span != source
            || !matches!(&root.kind, CfgPointKind::Evaluate(actual) if actual == expression)
            || entry.value.point != actual_root
            || entry.value.resolved.occurrence != source
            || entry.value.resolved.producer != self.cfg.producer_owner
            || entry.value.resolved.output != 0
            || !entry.value.resolved.output_projection.is_empty()
            || self
                .cfg
                .evaluated_values
                .values
                .iter()
                .filter(|row| row.id.resolved == entry.value.resolved)
                .count()
                != 1
        {
            return Err(invalid(&source));
        }
        let valid = match (expression, &root_shape.ty, &root_shape.payload) {
            (
                ScalarExpression::StructLiteral { .. },
                ScalarType::Struct(id),
                ValuePayloadShape::Struct { structure, .. },
            ) => id == structure,
            (
                ScalarExpression::ArrayLiteral { elements, .. },
                ScalarType::Array { length, .. },
                ValuePayloadShape::FixedArray { length: actual, .. },
            ) => length == actual && *actual == elements.len() as u64,
            _ => false,
        };
        if !valid {
            return Err(invalid(&source));
        }
        Ok(())
    }
}

fn invalid(source_span: &SourceSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

fn shape<'a>(
    cfg: &'a ScalarReferenceCfg,
    id: &ValueShapeId,
    source: &SourceSpan,
) -> Result<&'a ResolvedValueShape, ReferenceAnalysisError> {
    let matches = cfg
        .evaluated_values
        .shapes
        .iter()
        .filter(|row| row.id == *id)
        .collect::<Vec<_>>();
    let [shape] = matches.as_slice() else {
        return Err(invalid(source));
    };
    if id.producer != cfg.producer_owner {
        return Err(invalid(source));
    }
    Ok(shape)
}

fn point<'a>(
    cfg: &'a ScalarReferenceCfg,
    value: &EvaluatedValueId,
) -> Result<&'a ReferenceFlowPoint, ReferenceAnalysisError> {
    let source = &value.resolved.occurrence;
    let matches = cfg
        .initial_points
        .iter()
        .filter(|row| row.id == value.point)
        .collect::<Vec<_>>();
    let [point] = matches.as_slice() else {
        return Err(invalid(source));
    };
    if value.resolved.producer != cfg.producer_owner
        || cfg.initial_points.get(value.point.0) != Some(*point)
        || point.source_span != *source
        || value.resolved.output != 0
    {
        return Err(invalid(source));
    }
    Ok(point)
}

fn fact<'a>(
    cfg: &'a ScalarReferenceCfg,
    value: &EvaluatedValueId,
) -> Result<&'a EvaluatedValueFact, ReferenceAnalysisError> {
    let matches = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|row| row.id == *value)
        .collect::<Vec<_>>();
    let [fact] = matches.as_slice() else {
        return Err(invalid(&value.resolved.occurrence));
    };
    shape(cfg, &fact.shape, &value.resolved.occurrence)?;
    Ok(fact)
}

fn corresponding_shapes(
    cfg: &ScalarReferenceCfg,
    actual: &ValueShapeId,
    expected: &ValueShapeId,
    source: &SourceSpan,
    visited: &mut Vec<(ValueShapeId, ValueShapeId)>,
) -> Result<bool, ReferenceAnalysisError> {
    let left = shape(cfg, actual, source)?;
    let right = shape(cfg, expected, source)?;
    if !scalar_type_equal(&left.ty, &right.ty) {
        return Ok(false);
    }
    if visited
        .iter()
        .any(|pair| pair.0 == *actual && pair.1 == *expected)
    {
        return Ok(true);
    }
    visited.push((actual.clone(), expected.clone()));
    match (&left.payload, &right.payload) {
        (ValuePayloadShape::TemplateParameter { scope: a, declaration: x }, ValuePayloadShape::TemplateParameter { scope: b, declaration: y }) => Ok(left.ty == right.ty && a == b && x == y),
        (ValuePayloadShape::Plain(left), ValuePayloadShape::Plain(right)) => Ok(left == right),
        (
            ValuePayloadShape::Callable {
                signature: left_signature,
            },
            ValuePayloadShape::Callable {
                signature: right_signature,
            },
        ) => Ok(scalar_type_equal(left_signature, right_signature)
            && scalar_type_equal(left_signature, &left.ty)
            && scalar_type_equal(right_signature, &right.ty)),
        (
            ValuePayloadShape::Checked {
                mutability: left,
                referent: a,
            },
            ValuePayloadShape::Checked {
                mutability: right,
                referent: b,
            },
        ) => Ok(left == right && corresponding_shapes(cfg, a, b, source, visited)?),
        (ValuePayloadShape::Raw { pointee: a }, ValuePayloadShape::Raw { pointee: b }) => {
            corresponding_shapes(cfg, a, b, source, visited)
        }
        (
            ValuePayloadShape::Struct {
                structure: a,
                fields: left,
            },
            ValuePayloadShape::Struct {
                structure: b,
                fields: right,
            },
        ) => {
            if a != b || left.len() != right.len() {
                return Ok(false);
            }
            for field in left {
                let matches = right
                    .iter()
                    .filter(|row| row.field == field.field)
                    .collect::<Vec<_>>();
                let [expected] = matches.as_slice() else {
                    return Err(invalid(source));
                };
                if left.iter().filter(|row| row.field == field.field).count() != 1
                    || !corresponding_shapes(cfg, &field.shape, &expected.shape, source, visited)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (
            ValuePayloadShape::FixedArray {
                element: a,
                length: left,
            },
            ValuePayloadShape::FixedArray {
                element: b,
                length: right,
            },
        ) => Ok(left == right && corresponding_shapes(cfg, a, b, source, visited)?),
        (
            ValuePayloadShape::RuntimeArray { element: a },
            ValuePayloadShape::RuntimeArray { element: b },
        ) => corresponding_shapes(cfg, a, b, source, visited),
        (
            ValuePayloadShape::Enum {
                enumeration: a,
                variants: left,
            },
            ValuePayloadShape::Enum {
                enumeration: b,
                variants: right,
            },
        ) => {
            if a != b || left.len() != right.len() {
                return Ok(false);
            }
            for variant in left {
                let matches = right
                    .iter()
                    .filter(|row| row.ordinal == variant.ordinal)
                    .collect::<Vec<_>>();
                let [expected] = matches.as_slice() else {
                    return Err(invalid(source));
                };
                if left
                    .iter()
                    .filter(|row| row.ordinal == variant.ordinal)
                    .count()
                    != 1
                    || variant.fields.len() != expected.fields.len()
                {
                    return Ok(false);
                }
                for (a, b) in variant.fields.iter().zip(&expected.fields) {
                    if !corresponding_shapes(cfg, a, b, source, visited)? {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(super) fn literal_aggregate_members(
    cfg: &ScalarReferenceCfg,
    structs: &[ScalarStruct],
    value: &EvaluatedValueId,
) -> Result<Vec<EvaluatedMemberFact>, ReferenceAnalysisError> {
    let source = &value.resolved.occurrence;
    let root = fact(cfg, value)?;
    let root_shape = shape(cfg, &root.shape, source)?;
    let point = point(cfg, value)?;
    if !value.resolved.output_projection.is_empty()
        || cfg
            .evaluated_values
            .values
            .iter()
            .filter(|row| row.id.resolved.occurrence == *source && row.id.resolved.output == 0)
            .count()
            != 1
    {
        return Err(invalid(source));
    }
    let CfgPointKind::Evaluate(expression) = &point.kind else {
        return Err(invalid(source));
    };
    if expression_span(expression) != source.range {
        return Err(invalid(source));
    }
    match (expression, &root_shape.payload, &root_shape.ty) {
        (
            ScalarExpression::StructLiteral {
                fields: literal, ..
            },
            ValuePayloadShape::Struct { structure, fields },
            ScalarType::Struct(ty),
        ) => {
            let declarations = structs
                .iter()
                .filter(|row| row.id == *structure)
                .collect::<Vec<_>>();
            let [declaration] = declarations.as_slice() else {
                return Err(invalid(source));
            };
            if structure != ty
                || literal.len() != fields.len()
                || fields.len() != declaration.fields.len()
            {
                return Err(invalid(source));
            }
            let mut members = Vec::new();
            for field in literal {
                let child_source =
                    SourceSpan::new(source.source.clone(), expression_span(&field.value));
                let declared = declaration
                    .fields
                    .iter()
                    .filter(|row| row.name == field.name)
                    .collect::<Vec<_>>();
                let [declared] = declared.as_slice() else {
                    return Err(invalid(&child_source));
                };
                let registered = fields
                    .iter()
                    .filter(|row| row.field == declared.id)
                    .collect::<Vec<_>>();
                let [registered] = registered.as_slice() else {
                    return Err(invalid(&child_source));
                };
                let expected = shape(cfg, &registered.shape, &child_source)?;
                if declared.id.structure != *structure
                    || declared.id.index != declared.declaration_index
                    || declaration
                        .fields
                        .iter()
                        .filter(|row| row.id == declared.id)
                        .count()
                        != 1
                    || literal.iter().filter(|row| row.name == field.name).count() != 1
                    || !scalar_type_equal(&expected.ty, &declared.ty)
                {
                    return Err(invalid(&child_source));
                }
                members.push(EvaluatedMemberFact {
                    projection: vec![ValueProjection::Field(declared.id.clone())],
                    value: child(cfg, &field.value, &child_source, &registered.shape)?,
                });
            }
            for declared in &declaration.fields {
                if members
                    .iter()
                    .filter(|row| {
                        row.projection == vec![ValueProjection::Field(declared.id.clone())]
                    })
                    .count()
                    != 1
                {
                    return Err(invalid(source));
                }
            }
            Ok(members)
        }
        (
            ScalarExpression::ArrayLiteral { elements, .. },
            ValuePayloadShape::FixedArray { element, length },
            ScalarType::Array {
                element: ty,
                length: declared_length,
                ..
            },
        ) => {
            let expected = shape(cfg, element, source)?;
            if length != declared_length
                || *length != elements.len() as u64
                || !scalar_type_equal(&expected.ty, ty)
            {
                return Err(invalid(source));
            }
            elements
                .iter()
                .enumerate()
                .map(|(ordinal, expression)| {
                    let child_source =
                        SourceSpan::new(source.source.clone(), expression_span(expression));
                    Ok(EvaluatedMemberFact {
                        projection: vec![ValueProjection::FixedIndex(ordinal as u64)],
                        value: child(cfg, expression, &child_source, element)?,
                    })
                })
                .collect()
        }
        _ => Err(invalid(source)),
    }
}

fn child(
    cfg: &ScalarReferenceCfg,
    expression: &ScalarExpression,
    source: &SourceSpan,
    expected: &ValueShapeId,
) -> Result<EvaluatedValueId, ReferenceAnalysisError> {
    let candidates = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|row| row.id.resolved.occurrence == *source && row.id.resolved.output == 0)
        .collect::<Vec<_>>();
    let value = if let ScalarExpression::Assignment(assignment) = expression {
        assignment_child(cfg, assignment, source, &candidates)?
    } else {
        let [value] = candidates.as_slice() else {
            return Err(invalid(source));
        };
        &value.id
    };
    let projection = match expression {
        ScalarExpression::Call { occurrence, .. } => &occurrence.output_projection,
        ScalarExpression::Assignment(assignment) => &assignment.occurrence.output_projection,
        ScalarExpression::Block(block) => &block.occurrence.output_projection,
        ScalarExpression::Dereference { occurrence, .. }
        | ScalarExpression::IndexedRead { occurrence, .. } => &occurrence.output_projection,
        _ => &Vec::new(),
    };
    let fact = fact(cfg, value)?;
    let actual = point(cfg, value)?;
    if !corresponding_shapes(cfg, &fact.shape, expected, source, &mut Vec::new())?
        || value.resolved.output_projection != *projection
    {
        return Err(invalid(source));
    }
    let valid = match (expression, &actual.kind) {
        (
            ScalarExpression::Call { occurrence, .. },
            CfgPointKind::Call {
                expression: actual,
                output: 0,
                ..
            },
        ) if actual == expression => {
            let productions = cfg
                .evaluated_values
                .productions
                .iter()
                .filter(|row| row.occurrence == *source && row.output_projection == *projection)
                .collect::<Vec<_>>();
            let [production] = productions.as_slice() else {
                return Err(invalid(source));
            };
            let OutputProductionSource::Call {
                occurrence: registered,
                projections,
            } = &production.source
            else {
                return Err(invalid(source));
            };
            let calls = cfg
                .call_points
                .iter()
                .filter(|row| row.occurrence == **occurrence)
                .collect::<Vec<_>>();
            let [call] = calls.as_slice() else {
                return Err(invalid(source));
            };
            production.producer == cfg.producer_owner
                && registered == occurrence.as_ref()
                && occurrence.source_span == *source
                && production.outputs == vec![value.clone()]
                && production.evaluation_point == value.point
                && call.evaluated_point == value.point
                && call.projections == *projections
                && projections.len() == 1
                && projections[0].output == 0
                && projections[0].point == value.point
        }
        (ScalarExpression::Assignment(_), CfgPointKind::AssignmentResult { output: 0, .. }) => true,
        (
            ScalarExpression::FunctionLiteral {
                parameters,
                parameter_spans,
                body,
                ..
            },
            CfgPointKind::Capture {
                literal,
                environment,
                signature,
                parameters: names,
                parameter_spans: spans,
                body: actual_body,
                ..
            },
        ) => {
            let captures = cfg
                .capture_points
                .iter()
                .filter(|row| {
                    row.point == value.point
                        || row.occurrence == ScalarCaptureOccurrence::for_literal(*literal, body)
                })
                .collect::<Vec<_>>();
            let [capture] = captures.as_slice() else {
                return Err(invalid(source));
            };
            *literal == source.range
                && environment.literal == *source
                && names == parameters
                && spans == parameter_spans
                && actual_body == body
                && capture.point == value.point
                && capture.occurrence == ScalarCaptureOccurrence::for_literal(*literal, body)
                && matches!(&shape(cfg, expected, source)?.payload, ValuePayloadShape::Callable { signature: ty }
                    if scalar_type_equal(ty, signature))
        }
        (
            ScalarExpression::Dereference {
                occurrence, place, ..
            }
            | ScalarExpression::IndexedRead {
                occurrence, place, ..
            },
            CfgPointKind::ReadPlace { place: actual, .. },
        ) => {
            let reads = cfg
                .read_points
                .iter()
                .filter(|row| row.occurrence == **occurrence || row.point == value.point)
                .collect::<Vec<_>>();
            let [read] = reads.as_slice() else {
                return Err(invalid(source));
            };
            actual == place
                && occurrence.source_span == *source
                && read.point == value.point
                && read.occurrence == **occurrence
        }
        (ScalarExpression::Name { .. }, CfgPointKind::Read { binding }) => {
            binding.source == source.source
        }
        (ScalarExpression::Member { .. }, CfgPointKind::ReadPlace { .. }) => true,
        (
            ScalarExpression::CheckedAddress {
                place, mutability, ..
            },
            CfgPointKind::Borrow { request },
        ) => match request {
            ReferenceBorrowRequest::DirectStorage {
                place: actual,
                loan,
                ..
            } => actual.place == *place && loan.mode == *mutability,
            ReferenceBorrowRequest::Through {
                place: actual,
                mode,
                ..
            } => actual == place && mode == mutability,
        },
        (
            ScalarExpression::Block(_)
            | ScalarExpression::If { .. }
            | ScalarExpression::UnitIf { .. },
            _,
        ) => {
            return Err(invalid(source));
        }
        (_, CfgPointKind::Evaluate(actual)) => actual == expression,
        _ => false,
    };
    if !valid {
        return Err(invalid(source));
    }
    Ok(value.clone())
}

pub(in crate::scalar) fn assignment_child<'a>(
    cfg: &'a ScalarReferenceCfg,
    assignment: &ScalarAssignment,
    source: &SourceSpan,
    candidates: &[&EvaluatedValueFact],
) -> Result<&'a EvaluatedValueId, ReferenceAnalysisError> {
    let evaluations = cfg
        .evaluated_values
        .assignment_evaluations
        .iter()
        .filter(|row| row.operation.occurrence == *assignment.occurrence)
        .collect::<Vec<_>>();
    let [evaluation] = evaluations.as_slice() else {
        return Err(invalid(source));
    };
    let [result] = evaluation.results.as_slice() else {
        return Err(invalid(source));
    };
    let registered = cfg
        .evaluated_values
        .results
        .iter()
        .filter(|row| row.operation == evaluation.operation || row.value == result.value)
        .collect::<Vec<_>>();
    let [registered] = registered.as_slice() else {
        return Err(invalid(source));
    };
    let previous = evaluation
        .previous
        .iter()
        .filter(|row| row.point == result.snapshot)
        .collect::<Vec<_>>();
    let [previous] = previous.as_slice() else {
        return Err(invalid(source));
    };
    let SnapshotState::Saved(saved) = &previous.state else {
        return Err(invalid(source));
    };
    let snapshots = cfg
        .evaluated_values
        .snapshots
        .iter()
        .filter(|row| row.point == result.snapshot)
        .collect::<Vec<_>>();
    let [snapshot] = snapshots.as_slice() else {
        return Err(invalid(source));
    };
    let saved_fact = fact(cfg, saved)?;
    let result_fact = fact(cfg, &result.value)?;
    let snapshots = cfg
        .initial_points
        .iter()
        .filter(|row| row.id == saved.point)
        .collect::<Vec<_>>();
    let [snapshot_point] = snapshots.as_slice() else {
        return Err(invalid(source));
    };
    let result_point = point(cfg, &result.value)?;
    let destinations = evaluation
        .destinations
        .iter()
        .filter(|row| row.id == previous.destination)
        .collect::<Vec<_>>();
    let [destination] = destinations.as_slice() else {
        return Err(invalid(source));
    };
    let registered_destinations = cfg
        .evaluated_values
        .destinations
        .iter()
        .filter(|row| row.id == destination.id)
        .collect::<Vec<_>>();
    let [registered_destination] = registered_destinations.as_slice() else {
        return Err(invalid(source));
    };
    let retained = cfg
        .assignment_points
        .iter()
        .filter(|row| {
            row.occurrence == *assignment.occurrence
                && row.target == 0
                && row.role == ScalarAssignmentPointRole::Result
        })
        .collect::<Vec<_>>();
    let [retained] = retained.as_slice() else {
        return Err(invalid(source));
    };
    if evaluation.operation.producer != cfg.producer_owner
        || evaluation.operation.occurrence.source_span != *source
        || result.operation != evaluation.operation
        || result.output != 0
        || result != *registered
        || *previous != *snapshot
        || previous.destination.operation != evaluation.operation
        || previous.destination.target != 0
        || saved.point != result.snapshot
        || saved == &result.value
        || saved.resolved != result.value.resolved
        || saved_fact.shape != result_fact.shape
        || result.value.resolved.occurrence != *source
        || result.value.resolved.output_projection != assignment.occurrence.output_projection
        || candidates.len() != 2
        || candidates.iter().filter(|row| row.id == *saved).count() != 1
        || candidates
            .iter()
            .filter(|row| row.id == result.value)
            .count()
            != 1
        || saved.resolved.producer != cfg.producer_owner
        || *destination != *registered_destination
        || destination.shape != saved_fact.shape
        || retained.point != result.value.point
        || snapshot_point.source_span != destination.source_span
        || cfg.initial_points.get(saved.point.0) != Some(*snapshot_point)
        || snapshot_point.scope != result_point.scope
        || saved.point >= result.value.point
        || !matches!(&snapshot_point.kind, CfgPointKind::AssignmentSnapshot { destination_point, observed: true, .. }
            if *destination_point == destination.point)
        || !matches!(&result_point.kind, CfgPointKind::AssignmentResult { snapshot_point, output: 0 }
            if *snapshot_point == result.snapshot)
    {
        return Err(invalid(source));
    }
    Ok(&result.value)
}

#[cfg(test)]
#[path = "evaluated_aggregate_member_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "evaluated_aggregate_root_tests.rs"]
mod root_tests;
