use super::*;
use evaluated_value::{RawAddressOperandInput, RawAddressOperandRole, RawAddressOperandStep};

fn invalid(source_span: &SourceSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

impl reference::ReferenceFlowBuilder {
    fn raw_address_operands(&mut self, place: &ScalarPlace) -> Result<Vec<RawAddressOperandInput>, ReferenceAnalysisError> {
        fn operand(builder: &mut reference::ReferenceFlowBuilder, expression: &ScalarExpression, point: CfgPointId, role: RawAddressOperandRole, path: &[RawAddressOperandStep]) -> Result<RawAddressOperandInput, ReferenceAnalysisError> {
            let source = SourceSpan::new(builder.source.clone(), expression_span(expression));
            let sequence = builder.bind_final_output_values(expression, point)?;
            let [entry] = sequence.entries.as_slice() else { return Err(invalid(&source)); };
            if entry.value.resolved.producer != builder.cfg.producer_owner || entry.value.point != point
                || entry.value.resolved.occurrence != source || entry.value.resolved.output != 0
                || !entry.value.resolved.output_projection.is_empty() { return Err(invalid(&source)); }
            let facts = builder.cfg.evaluated_values.values.iter().filter(|fact| fact.id == entry.value).collect::<Vec<_>>();
            let [fact] = facts.as_slice() else { return Err(invalid(&source)); };
            let shapes = builder.cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == entry.shape).collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else { return Err(invalid(&source)); };
            if fact.shape != shape.id { return Err(invalid(&source)); }
            Ok(RawAddressOperandInput { role, path: path.to_vec(), expression: expression.clone(), value: entry.value.clone() })
        }
        fn collect(builder: &mut reference::ReferenceFlowBuilder, place: &ScalarPlace, path: &mut Vec<RawAddressOperandStep>, inputs: &mut Vec<RawAddressOperandInput>) -> Result<(), ReferenceAnalysisError> {
            match place {
                ScalarPlace::Name { .. } => {}
                ScalarPlace::Field { base, .. } => {
                    path.push(RawAddressOperandStep::Base);
                    collect(builder, base, path, inputs)?;
                    path.pop();
                }
                ScalarPlace::Dereference { pointer, span } => {
                    let source = SourceSpan::new(builder.source.clone(), expression_span(pointer));
                    let point = *builder.dereference_points.get(span).ok_or_else(|| invalid(&source))?;
                    path.push(RawAddressOperandStep::Pointer);
                    inputs.push(operand(builder, pointer, point, RawAddressOperandRole::Pointer, path)?);
                    path.pop();
                }
                ScalarPlace::Index { base, index, index_span, .. } => {
                    path.push(RawAddressOperandStep::Base);
                    collect(builder, base, path, inputs)?;
                    path.pop();
                    let source = SourceSpan::new(builder.source.clone(), expression_span(index));
                    let point = *builder.index_points.get(index_span).ok_or_else(|| invalid(&source))?;
                    path.push(RawAddressOperandStep::Index);
                    inputs.push(operand(builder, index, point, RawAddressOperandRole::Index, path)?);
                    path.pop();
                }
            }
            Ok(())
        }
        let mut inputs = Vec::new();
        collect(self, place, &mut Vec::new(), &mut inputs)?;
        Ok(inputs)
    }

    pub(super) fn register_raw_address_input(
        &mut self,
        expression: &ScalarExpression,
        evaluation: CfgPointId,
        read: CfgPointId,
        place: ReferencePlaceId,
    ) -> Result<(), ReferenceAnalysisError> {
        let source = SourceSpan::new(self.source.clone(), expression_span(expression));
        let sequence = self.bind_final_output_values(expression, evaluation)?;
        let [entry] = sequence.entries.as_slice() else {
            return Err(invalid(&source));
        };
        let shapes = self
            .cfg
            .evaluated_values
            .shapes
            .iter()
            .filter(|row| row.id == entry.shape)
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err(invalid(&source));
        };
        let ValuePayloadShape::Raw { pointee } = &shape.payload else {
            return Err(invalid(&source));
        };
        let input = RawAddressSourceInput {
            value: entry.value.clone(),
            read,
            place,
            pointee: pointee.clone(),
        };
        validate_input(&self.cfg, &input)?;
        if self
            .cfg
            .evaluated_values
            .raw_address_inputs
            .iter()
            .any(|row| row.value == input.value)
        {
            return Err(invalid(&source));
        }
        let ScalarExpression::RawAddress { place: raw_place, .. } = expression else { return Err(invalid(&source)); };
        let operands = self.raw_address_operands(raw_place)?;
        self.cfg
            .evaluated_values
            .raw_address_productions
            .push(RawAddressProduction {
                value: input.value.clone(),
                source: RawAddressInputSource::CompilerStorage,
                operands,
            });
        self.cfg.evaluated_values.raw_address_inputs.push(input);
        Ok(())
    }

    pub(super) fn register_raw_pointer_input(
        &mut self,
        expression: &ScalarExpression,
        evaluation: CfgPointId,
        pointer: EvaluatedValueId,
        place: ScalarPlace,
    ) -> Result<(), ReferenceAnalysisError> {
        let source = SourceSpan::new(self.source.clone(), expression_span(expression));
        let sequence = self.bind_final_output_values(expression, evaluation)?;
        let [entry] = sequence.entries.as_slice() else {
            return Err(invalid(&source));
        };
        let operands = self.raw_address_operands(&place)?;
        let production = RawAddressProduction {
            value: entry.value.clone(),
            source: RawAddressInputSource::ThroughPointer { pointer, place },
            operands,
        };
        validate_production(&self.cfg, &production)?;
        if self
            .cfg
            .evaluated_values
            .raw_address_productions
            .iter()
            .any(|row| row.value == production.value)
        {
            return Err(invalid(&source));
        }
        self.cfg
            .evaluated_values
            .raw_address_productions
            .push(production);
        Ok(())
    }
}

pub(super) fn validate_production(
    cfg: &ScalarReferenceCfg,
    production: &RawAddressProduction,
) -> Result<(), ReferenceAnalysisError> {
    let source = &production.value.resolved.occurrence;
    let point = cfg
        .points
        .get(production.value.point.0)
        .ok_or_else(|| invalid(source))?;
    let CfgPointKind::Evaluate(ScalarExpression::RawAddress { place: actual, .. }) = &point.kind
    else {
        return Err(invalid(source));
    };
    let facts = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|fact| fact.id == production.value)
        .collect::<Vec<_>>();
    let [fact] = facts.as_slice() else {
        return Err(invalid(source));
    };
    let shapes = cfg
        .evaluated_values
        .shapes
        .iter()
        .filter(|shape| shape.id == fact.shape)
        .collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else {
        return Err(invalid(source));
    };
    if production.value.resolved.producer != cfg.producer_owner
        || point.source_span != *source
        || production.value.resolved.output != 0
        || !production.value.resolved.output_projection.is_empty()
        || !matches!(shape.payload, ValuePayloadShape::Raw { .. })
    {
        return Err(invalid(source));
    }
    match &production.source {
        RawAddressInputSource::CompilerStorage => {
            let input = source_input(cfg, production.value.point)?;
            if input.value != production.value {
                return Err(invalid(source));
            }
        }
        RawAddressInputSource::ThroughPointer { pointer, place } => {
            let ScalarPlace::Dereference {
                pointer: expression,
                ..
            } = reference::reference_root(place)
            else {
                return Err(invalid(source));
            };
            let facts = cfg
                .evaluated_values
                .values
                .iter()
                .filter(|fact| fact.id == *pointer)
                .collect::<Vec<_>>();
            let [fact] = facts.as_slice() else {
                return Err(invalid(source));
            };
            let shapes = cfg
                .evaluated_values
                .shapes
                .iter()
                .filter(|shape| shape.id == fact.shape)
                .collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else {
                return Err(invalid(source));
            };
            if actual != place
                || pointer.resolved.producer != cfg.producer_owner
                || pointer.resolved.occurrence
                    != SourceSpan::new(source.source.clone(), expression_span(expression))
                || pointer.point >= point.id
                || !matches!(
                    shape.payload,
                    ValuePayloadShape::Checked { .. } | ValuePayloadShape::Raw { .. }
                )
            {
                return Err(invalid(source));
            }
        }
    }
    Ok(())
}

pub(super) fn production(
    cfg: &ScalarReferenceCfg,
    point: CfgPointId,
) -> Result<&RawAddressProduction, ReferenceAnalysisError> {
    let rows = cfg
        .evaluated_values
        .raw_address_productions
        .iter()
        .filter(|row| row.value.point == point)
        .collect::<Vec<_>>();
    let [production] = rows.as_slice() else {
        return Err(invalid(&cfg.points[point.0].source_span));
    };
    validate_production(cfg, production)?;
    Ok(production)
}

pub(super) fn validate_input(
    cfg: &ScalarReferenceCfg,
    input: &RawAddressSourceInput,
) -> Result<(), ReferenceAnalysisError> {
    let source = &input.value.resolved.occurrence;
    let evaluation = cfg
        .points
        .get(input.value.point.0)
        .ok_or_else(|| invalid(source))?;
    let read = cfg
        .points
        .get(input.read.0)
        .ok_or_else(|| invalid(source))?;
    let CfgPointKind::Evaluate(ScalarExpression::RawAddress { place, span }) = &evaluation.kind
    else {
        return Err(invalid(source));
    };
    let CfgPointKind::ReadPlace {
        place: actual,
        pointer_point: None,
        candidates,
        reads_origin: false,
    } = &read.kind
    else {
        return Err(invalid(source));
    };
    let [candidate] = candidates.as_slice() else {
        return Err(invalid(source));
    };
    let facts = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|row| row.id == input.value)
        .collect::<Vec<_>>();
    let [fact] = facts.as_slice() else {
        return Err(invalid(source));
    };
    let shapes = cfg
        .evaluated_values
        .shapes
        .iter()
        .filter(|row| row.id == fact.shape)
        .collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else {
        return Err(invalid(source));
    };
    let pointees = cfg
        .evaluated_values
        .shapes
        .iter()
        .filter(|row| row.id == input.pointee)
        .collect::<Vec<_>>();
    let [pointee] = pointees.as_slice() else {
        return Err(invalid(source));
    };
    if input.value.resolved.producer != cfg.producer_owner
        || input.value.resolved.output != 0
        || !input.value.resolved.output_projection.is_empty()
        || evaluation.id != input.value.point
        || read.id != input.read
        || evaluation.source_span != *source
        || read.source_span != *source
        || *span != source.range
        || read.scope != evaluation.scope
        || read.successors != vec![evaluation.id]
        || candidate != &input.place
        || actual != place
        || candidate.place != *place
        || input.place.source != input.place.binding.source
        || shape.id.producer != cfg.producer_owner
        || pointee.id.producer != cfg.producer_owner
        || !matches!(&shape.payload, ValuePayloadShape::Raw { pointee } if *pointee == input.pointee)
        || !matches!(&shape.ty, ScalarType::RawPointer(ty) if scalar_type_equal(ty, &pointee.ty))
    {
        return Err(invalid(source));
    }
    Ok(())
}

pub(super) fn source_input(
    cfg: &ScalarReferenceCfg,
    point: CfgPointId,
) -> Result<&RawAddressSourceInput, ReferenceAnalysisError> {
    let event = &cfg.points[point.0];
    let rows = cfg
        .evaluated_values
        .raw_address_inputs
        .iter()
        .filter(|row| row.value.point == point)
        .collect::<Vec<_>>();
    let [input] = rows.as_slice() else {
        return Err(invalid(&event.source_span));
    };
    validate_input(cfg, input)?;
    Ok(input)
}

#[cfg(test)]
mod tests;
