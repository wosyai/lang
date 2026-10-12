use super::*;

impl ReferenceFlowBuilder {
    pub(super) fn module_literal_members(
        &mut self,
        binding: &ScalarBinding,
        target: &ReferenceBindingId,
    ) -> Result<SourceLiteralMembersFact, ReferenceAnalysisError> {
        let value = source(&self.cfg, target)?;
        let fail = || self.missing_context(binding.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
        let ScalarExpression::StructLiteral { fields, .. } = &binding.value else { return Err(fail()); };
        let mut original = self.cfg.clone();
        original.initial_points = original.points.clone();
        let members = evaluated_aggregate_member::literal_aggregate_members(&original, &self.structs, &value)?;
        if fields.len() != members.len() { return Err(fail()); }
        for (field, member) in fields.iter().zip(&members) {
            let occurrence = SourceSpan::new(self.source.clone(), expression_span(&field.value));
            let candidates = self.cfg.evaluated_values.values.iter().filter(|fact|
                fact.id.resolved.producer == self.cfg.producer_owner && fact.id.resolved.occurrence == occurrence
                    && fact.id.resolved.output == 0 && fact.id.resolved.output_projection.is_empty()).collect::<Vec<_>>();
            let [child] = candidates.as_slice() else { return Err(fail()); };
            let shapes = self.cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == child.shape).collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else { return Err(fail()); };
            if member.value != child.id || shape.id.producer != self.cfg.producer_owner { return Err(fail()); }
            if matches!(field.value, ScalarExpression::Call { .. }) {
                let outputs = self.call_output_sequence(&field.value)?;
                let [output] = outputs.entries.as_slice() else { return Err(fail()); };
                if output.value != child.id || output.shape != child.shape { return Err(fail()); }
            }
        }
        let row = SourceLiteralMembersFact { value, members };
        if self.cfg.evaluated_values.source_literal_members.iter().any(|known| known.value == row.value) { return Err(fail()); }
        self.cfg.evaluated_values.source_literal_members.push(row.clone());
        Ok(row)
    }
}

pub(super) struct AggregateReceiptPlan {
    pub event: CfgPointId,
    pub bind: CfgPointId,
    pub target: ReferenceBindingId,
    pub source: EvaluatedValueId,
    pub policy: CopyPolicy,
    pub shape: ValueShapeId,
    pub members: Vec<(Vec<ValueProjection>, ValueShapeId)>,
    pub first_receipt: ReferenceBindingId,
    pub installations: Vec<CfgPointId>,
}

fn invalid(cfg: &ScalarReferenceCfg, point: CfgPointId) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: cfg.points[point.0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

pub(super) fn source(cfg: &ScalarReferenceCfg, target: &ReferenceBindingId) -> Result<EvaluatedValueId, ReferenceAnalysisError> {
    let events = cfg.points.iter().filter(|point|
        matches!(&point.kind, CfgPointKind::Bind { binding } if binding == target)).collect::<Vec<_>>();
    let [event] = events.as_slice() else { return Err(ReferenceAnalysisError::MissingContext {
        source_span: SourceSpan::new(target.source.clone(), target.declaration_span),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }); };
    let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == event.id).collect::<Vec<_>>();
    let [input] = inputs.as_slice() else { return Err(ReferenceAnalysisError::MissingContext {
        source_span: event.source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }); };
    Ok(input.source.clone())
}

pub(super) fn members(cfg: &ScalarReferenceCfg, source: &EvaluatedValueId) -> Result<Vec<(Vec<ValueProjection>, ValueShapeId)>, ReferenceAnalysisError> {
    let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *source).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(invalid(cfg, source.point)); };
    let mut pending = vec![(Vec::new(), fact.shape.clone(), Vec::new())];
    let mut members = Vec::new();
    while let Some((projection, id, mut ancestors)) = pending.pop() {
        if ancestors.contains(&id) { return Err(invalid(cfg, source.point)); }
        ancestors.push(id.clone());
        let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == id).collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else { return Err(invalid(cfg, source.point)); };
        if shape.id.producer != source.resolved.producer { return Err(invalid(cfg, source.point)); }
        match &shape.payload {
            ValuePayloadShape::Struct { fields, .. } => {
                let ValuePayloadShape::Struct { structure, .. } = &shape.payload else { unreachable!(); };
                if !matches!(&shape.ty, ScalarType::Struct(actual) if actual == structure)
                    || fields.iter().enumerate().any(|(index, field)| field.field.structure != *structure || field.field.index != index) { return Err(invalid(cfg, source.point)); }
                for field in fields.iter().rev() {
                    let mut path = projection.clone(); path.push(ValueProjection::Field(field.field.clone()));
                    pending.push((path, field.shape.clone(), ancestors.clone()));
                }
            }
            ValuePayloadShape::FixedArray { element, length } => {
                let ScalarType::Array { element: ty, length: actual, .. } = &shape.ty else { return Err(invalid(cfg, source.point)); };
                let child = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == *element).ok_or_else(|| invalid(cfg, source.point))?;
                if !scalar_type_equal(ty, &child.ty) || *actual != *length { return Err(invalid(cfg, source.point)); }
                for index in (0..*length).rev() {
                    let mut path = projection.clone(); path.push(ValueProjection::FixedIndex(index));
                    pending.push((path, element.clone(), ancestors.clone()));
                }
            }
            ValuePayloadShape::Enum { variants, .. } => {
                let ValuePayloadShape::Enum { enumeration, .. } = &shape.payload else { unreachable!(); };
                if !matches!(&shape.ty, ScalarType::Enum(actual) if actual == enumeration)
                    || variants.iter().enumerate().any(|(index, variant)| variant.ordinal != index) { return Err(invalid(cfg, source.point)); }
                for variant in variants.iter().rev() {
                    for (field, id) in variant.fields.iter().enumerate().rev() {
                        let mut path = projection.clone(); path.push(ValueProjection::VariantField { variant: variant.ordinal, field });
                        pending.push((path, id.clone(), ancestors.clone()));
                    }
                }
            }
            ValuePayloadShape::TemplateParameter { .. } => return Err(invalid(cfg, source.point)),
            ValuePayloadShape::Plain(_) => {}
            ValuePayloadShape::Raw { .. } | ValuePayloadShape::Checked { .. }
                | ValuePayloadShape::Callable { .. } | ValuePayloadShape::RuntimeArray { .. } => members.push((projection, id)),
        }
    }
    Ok(members)
}

pub(super) fn plan(cfg: &ScalarReferenceCfg, event: CfgPointId) -> Result<AggregateReceiptPlan, ReferenceAnalysisError> {
    let fail = || invalid(cfg, event);
    let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(source), target } = &cfg.initial_points[event.0].kind else { return Err(fail()); };
    if !matches!(&cfg.points[event.0].kind, CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(actual), target: receiving }
        if actual == source && receiving == target) { return Err(fail()); }
    let rows = cfg.binding_points.iter().filter(|row| row.point == event && row.role == ScalarBindingPointRole::CopyCallableFields).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(fail()); };
    let receivers = cfg.binding_points.iter().filter(|receiver| receiver.role == ScalarBindingPointRole::Bind
        && receiver.occurrence == row.occurrence && receiver.output == row.output).collect::<Vec<_>>();
    let [receiver] = receivers.as_slice() else { return Err(fail()); };
    let CfgPointKind::Bind { binding } = &cfg.initial_points[receiver.point.0].kind else { return Err(fail()); };
    let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == receiver.point).collect::<Vec<_>>();
    let [input] = inputs.as_slice() else { return Err(fail()); };
    if binding != target || input.source != *source || row.source != receiver.source
        || input.occurrence != row.occurrence || input.output != row.output
        || input.producer != cfg.producer_owner || source.resolved.producer != input.producer
        || cfg.assignment_points.iter().any(|assignment| assignment.point == event)
        || cfg.binding_points.iter().filter(|copy| copy.occurrence == row.occurrence && copy.output == row.output
            && copy.role == ScalarBindingPointRole::CopyCallableFields).count() != 1
        || !binding_holder::dominates(&cfg.initial_points, source.point, receiver.point)
        || !binding_holder::dominates(&cfg.initial_points, receiver.point, event) { return Err(fail()); }
    let policy = output_sequence::binding_disposition(&cfg.evaluated_values, input, target)?.policy;
    let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *source).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(fail()); };
    let members = members(cfg, source)?;
    let receiving_shapes = cfg.evaluated_values.source_binding_shapes.iter().filter(|row| row.binding == *target).collect::<Vec<_>>();
    let [receiving_shape] = receiving_shapes.as_slice() else { return Err(fail()); };
    if receiving_shape.shape != fact.shape || members.is_empty() { return Err(fail()); }
    let first = match binding_holder::first(cfg, input) { Some(first) => first, None => input };
    let CfgPointKind::Bind { binding: first_receipt } = &cfg.initial_points[first.consumer.0].kind else { return Err(fail()); };
    let installations = installations(cfg, first, event)?;
    if first.consumer != receiver.point && (!binding_holder::dominates(&cfg.initial_points, first.consumer, event)
        || installations.is_empty() || installations.iter().any(|installation|
            !binding_holder::dominates(&cfg.initial_points, *installation, event))) { return Err(fail()); }
    Ok(AggregateReceiptPlan { event, bind: receiver.point, target: target.clone(), source: source.clone(), policy,
        shape: fact.shape.clone(), members, first_receipt: first_receipt.clone(), installations })
}

pub(super) fn installations(cfg: &ScalarReferenceCfg, input: &BindingOutputInput, event: CfgPointId) -> Result<Vec<CfgPointId>, ReferenceAnalysisError> {
    let fail = || invalid(cfg, event);
    let CfgPointKind::Bind { binding } = &cfg.initial_points[input.consumer.0].kind else { return Err(fail()); };
    let literal = matches!(&cfg.initial_points[input.source.point.0].kind,
        CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }));
    let rows = cfg.binding_points.iter().filter(|row| row.occurrence == input.occurrence && row.output == input.output
        && matches!(row.role, ScalarBindingPointRole::CopyCallableFields | ScalarBindingPointRole::FieldAssign(_)
            | ScalarBindingPointRole::ArrayElement(_))).collect::<Vec<_>>();
    let mut points = vec![input.consumer];
    if literal {
        let registrations = cfg.binding_points.iter().filter(|row| row.point == input.consumer
            && row.role == ScalarBindingPointRole::Bind).collect::<Vec<_>>();
        let [registration] = registrations.as_slice() else { return Err(fail()); };
        let literals = cfg.evaluated_values.source_literal_members.iter().filter(|row| row.value == input.source).collect::<Vec<_>>();
        let [literal] = literals.as_slice() else { return Err(fail()); };
        let expressions = match &cfg.initial_points[input.source.point.0].kind {
            CfgPointKind::Evaluate(ScalarExpression::StructLiteral { fields, .. }) => fields.iter().map(|field| &field.value).collect::<Vec<_>>(),
            CfgPointKind::Evaluate(ScalarExpression::ArrayLiteral { elements, .. }) => elements.iter().collect::<Vec<_>>(),
            _ => return Err(fail()),
        };
        if expressions.len() != literal.members.len() || expressions.iter().zip(&literal.members).any(|(expression, member)|
            member.value.resolved.occurrence != SourceSpan::new(input.source.resolved.occurrence.source.clone(), expression_span(expression))) { return Err(fail()); }
        let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).ok_or_else(fail)?;
        let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).ok_or_else(fail)?;
        let expected = match &shape.payload {
            ValuePayloadShape::Struct { fields, .. } => fields.iter().map(|field|
                (ValueProjection::Field(field.field.clone()), field.shape.clone())).collect::<Vec<_>>(),
            ValuePayloadShape::FixedArray { element, length } => (0..*length).map(|index|
                (ValueProjection::FixedIndex(index), element.clone())).collect::<Vec<_>>(),
            _ => return Err(fail()),
        };
        if literal.members.len() != expected.len() { return Err(fail()); }
        let mut roles = Vec::new();
        for (projection, shape) in &expected {
            let children = literal.members.iter().filter(|member| member.projection == [projection.clone()]).collect::<Vec<_>>();
            let [child] = children.as_slice() else { return Err(fail()); };
            let values = cfg.evaluated_values.values.iter().filter(|value| value.id == child.value).collect::<Vec<_>>();
            let [value] = values.as_slice() else { return Err(fail()); };
            let shapes = cfg.evaluated_values.shapes.iter().filter(|row| row.id == *shape).collect::<Vec<_>>();
            let [member_shape] = shapes.as_slice() else { return Err(fail()); };
            if child.value.resolved.producer != cfg.producer_owner || value.shape != *shape
                || !binding_holder::dominates(&cfg.initial_points, child.value.point, input.consumer) { return Err(fail()); }
            match projection {
                ValueProjection::FixedIndex(index) => roles.push(ScalarBindingPointRole::ArrayElement(usize::try_from(*index).map_err(|_| fail())?)),
                ValueProjection::Field(field) => {
                    if matches!(member_shape.ty, ScalarType::Callable { .. } | ScalarType::CheckedReference { .. }) {
                        roles.push(ScalarBindingPointRole::FieldAssign(field.clone()));
                    }
                    if matches!(member_shape.ty, ScalarType::RuntimeArray { .. }) {
                        let events = cfg.initial_points.iter().filter(|point| matches!(&point.kind,
                            CfgPointKind::BackingPlace { target, .. } if field_target(target, binding, field))).collect::<Vec<_>>();
                        let [installation] = events.as_slice() else { return Err(fail()); };
                        let CfgPointKind::BackingPlace { transfer, .. } = &installation.kind else { unreachable!(); };
                        if !matches!(transfer, RuntimeBackingTransfer::Move { source }
                            if source_binding_matches(cfg, &child.value, source)) { return Err(fail()); }
                        if cfg.points[installation.id.0].kind != installation.kind
                            || !binding_holder::dominates(&cfg.initial_points, input.consumer, installation.id) { return Err(fail()); }
                        points.push(installation.id);
                    }
                    if projected_backing_shape(cfg, shape, event)? {
                        let events = cfg.initial_points.iter().filter(|point| matches!(&point.kind,
                            CfgPointKind::CopyProjectedBackings { target, .. } if field_target(target, binding, field))).collect::<Vec<_>>();
                        let [installation] = events.as_slice() else { return Err(fail()); };
                        let CfgPointKind::CopyProjectedBackings { source, .. } = &installation.kind else { unreachable!(); };
                        if !source_binding_matches(cfg, &child.value, source)
                            || cfg.points[installation.id.0].kind != installation.kind
                            || !binding_holder::dominates(&cfg.initial_points, input.consumer, installation.id) { return Err(fail()); }
                        points.push(installation.id);
                    }
                }
                _ => return Err(fail()),
            }
        }
        if rows.len() != roles.len() || roles.iter().any(|role| rows.iter().filter(|row| row.role == *role).count() != 1) { return Err(fail()); }
        for row in &rows {
            let point = cfg.initial_points.get(row.point.0).ok_or_else(fail)?;
            if row.source != registration.source { return Err(fail()); }
            let (projection, rhs) = match (&row.role, &point.kind) {
                (ScalarBindingPointRole::FieldAssign(field), CfgPointKind::Assign { target, rhs_point, .. }) => {
                    if target.binding != *binding || !matches!(target.projections.as_slice(),
                        [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual))] if actual == field) { return Err(fail()); }
                    (ValueProjection::Field(field.clone()), *rhs_point)
                }
                (ScalarBindingPointRole::ArrayElement(index), CfgPointKind::BindArrayElement { target, rhs_point }) => {
                    if target.binding != *binding || !matches!(target.projections.as_slice(),
                        [ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. })] if *value == BigInt::from(*index)) { return Err(fail()); }
                    (ValueProjection::FixedIndex(u64::try_from(*index).map_err(|_| fail())?), *rhs_point)
                }
                _ => return Err(fail()),
            };
            let members = literal.members.iter().filter(|member| member.projection == [projection.clone()]).collect::<Vec<_>>();
            let [member] = members.as_slice() else { return Err(fail()); };
            let current_matches = match (&cfg.points[row.point.0].kind, &point.kind) {
                (CfgPointKind::Assign { target, rhs_point, .. }, CfgPointKind::Assign { target: original, rhs_point: source, .. }) => target == original && rhs_point == source,
                (CfgPointKind::BindArrayElement { target, rhs_point }, CfgPointKind::BindArrayElement { target: original, rhs_point: source }) => target == original && rhs_point == source,
                _ => false,
            };
            if member.value.point != rhs || member.value.resolved.producer != cfg.producer_owner
                || !current_matches
                || !binding_holder::dominates(&cfg.initial_points, rhs, row.point)
                || !binding_holder::dominates(&cfg.initial_points, input.consumer, row.point) { return Err(fail()); }
        }
    } else if rows.len() != 1 || rows[0].role != ScalarBindingPointRole::CopyCallableFields { return Err(fail()); }
    points.extend(rows.iter().map(|row| row.point));
    points.sort_by_key(|point| point.0);
    if points.windows(2).any(|pair| pair[0] == pair[1] || !binding_holder::dominates(&cfg.initial_points, pair[0], pair[1])) { return Err(fail()); }
    Ok(points)
}

fn field_target(target: &ReferencePlaceId, binding: &ReferenceBindingId, field: &ScalarStructFieldId) -> bool {
    target.binding == *binding && target.source == binding.source && target.function_span == binding.function_span
        && target.declaration_span == binding.declaration_span && target.block_span == binding.block_span && matches!(target.projections.as_slice(),
        [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual))] if actual == field)
}

fn source_binding_matches(cfg: &ScalarReferenceCfg, value: &EvaluatedValueId, source: &ReferenceBindingId) -> bool {
    let point = &cfg.initial_points[value.point.0];
    match &point.kind {
        CfgPointKind::Read { binding } => binding == source,
        CfgPointKind::Call { output, .. } => matches!(&source.kind, ReferenceBindingKind::CallOutput { point: call, output: actual }
            if *call == value.point && actual == output && *output == value.resolved.output)
            && source.source == point.source_span.source && source.declaration_span == point.source_span.range,
        _ => false,
    }
}

fn projected_backing_shape(cfg: &ScalarReferenceCfg, id: &ValueShapeId, event: CfgPointId) -> Result<bool, ReferenceAnalysisError> {
    let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == *id).ok_or_else(|| invalid(cfg, event))?;
    let ValuePayloadShape::Struct { fields, .. } = &shape.payload else { return Ok(false); };
    for field in fields {
        let child = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == field.shape).ok_or_else(|| invalid(cfg, event))?;
        if matches!(child.ty, ScalarType::RuntimeArray { .. }) || projected_backing_shape(cfg, &field.shape, event)? { return Ok(true); }
    }
    Ok(false)
}

pub(super) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    for point in &cfg.initial_points {
        if let CfgPointKind::CopyCallableFields { source, .. } = &point.kind {
            match source {
                CallableFieldSource::Evaluated(_) => { plan(cfg, point.id)?; }
                CallableFieldSource::Assignment(source) => {
                    let CfgPointKind::CopyCallableFields { target, .. } = &point.kind else { unreachable!(); };
                    if !matches!(&cfg.points[point.id.0].kind, CfgPointKind::CopyCallableFields {
                        source: CallableFieldSource::Assignment(actual), target: receiving } if actual == source && receiving == target) {
                        return Err(invalid(cfg, point.id));
                    }
                    let rows = cfg.assignment_points.iter().filter(|row| row.point == point.id
                        && row.role == ScalarAssignmentPointRole::CopyCallableFields).collect::<Vec<_>>();
                    if rows.len() != 1 || cfg.binding_points.iter().any(|row| row.point == point.id) { return Err(invalid(cfg, point.id)); }
                }
            }
        }
    }
    for row in &cfg.binding_points {
        if row.role == ScalarBindingPointRole::CopyCallableFields {
            if row.point.0 >= cfg.initial_points.len() { return Err(ReferenceAnalysisError::MissingContext {
                source_span: row.occurrence.source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            }); }
            plan(cfg, row.point)?;
        }
    }
    for input in &cfg.evaluated_values.binding_inputs {
        let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == input.source).collect::<Vec<_>>();
        let [fact] = facts.as_slice() else { return Err(invalid(cfg, input.consumer)); };
        let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else { return Err(invalid(cfg, input.consumer)); };
        if !matches!(shape.payload, ValuePayloadShape::Struct { .. } | ValuePayloadShape::Enum { .. } | ValuePayloadShape::FixedArray { .. })
            || members(cfg, &input.source)?.is_empty() { continue; }
        let literal_first = matches!(&cfg.initial_points[input.source.point.0].kind,
            CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }))
            && binding_holder::first(cfg, input).is_none();
        let copies = cfg.binding_points.iter().filter(|row| row.occurrence == input.occurrence && row.output == input.output
            && row.role == ScalarBindingPointRole::CopyCallableFields).collect::<Vec<_>>();
        if literal_first {
            if !copies.is_empty() { return Err(invalid(cfg, input.consumer)); }
            installations(cfg, input, input.consumer)?;
        } else {
            let [copy] = copies.as_slice() else { return Err(invalid(cfg, input.consumer)); };
            plan(cfg, copy.point)?;
        }
    }
    Ok(())
}
