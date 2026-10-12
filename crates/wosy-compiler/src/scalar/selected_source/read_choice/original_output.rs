use super::*;

fn projection(expression: &ScalarExpression) -> &[usize] {
    match expression {
        ScalarExpression::Call { occurrence, .. } => &occurrence.output_projection,
        ScalarExpression::Assignment(assignment) => &assignment.occurrence.output_projection,
        ScalarExpression::Block(block) => &block.occurrence.output_projection,
        ScalarExpression::Dereference { occurrence, .. } | ScalarExpression::IndexedRead { occurrence, .. } => &occurrence.output_projection,
        _ => &[],
    }
}

pub(super) fn validate(cfg: &ScalarReferenceCfg, tables: &EvaluatedValueTables, structs: &[ScalarStruct], expression: &ScalarExpression, registration: &ScalarIndexCfgPoint) -> Result<EvaluatedValueId, ReferenceAnalysisError> {
    let fail = || invalid(&registration.source_span);
    let original = cfg.initial_points.get(registration.point.0).ok_or_else(fail)?;
    let facts = tables.values.iter().filter(|fact| fact.id.point == registration.point && fact.id.resolved.producer == cfg.producer_owner && fact.id.resolved.output == 0).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(fail()); };
    let value = &fact.id;
    let shapes = tables.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else { return Err(fail()); };
    if original.id != registration.point || original.source_span != registration.source_span
        || value.resolved.occurrence != registration.source_span || value.resolved.output_projection != projection(expression)
        || shape.id.producer != cfg.producer_owner || !matches!(shape.payload, ValuePayloadShape::Plain(PlainValueKind::I8 | PlainValueKind::I16 | PlainValueKind::I32 | PlainValueKind::I64 | PlainValueKind::I128 | PlainValueKind::U8 | PlainValueKind::U16 | PlainValueKind::U32 | PlainValueKind::U64 | PlainValueKind::U128)) {
        return Err(fail());
    }
    let mut candidate = cfg.clone();
    candidate.points = cfg.initial_points.clone();
    candidate.evaluated_values = tables.clone();
    match expression {
        ScalarExpression::Name { .. } => {
            reference::name_resolution::validate(&candidate, structs)?;
            let names = tables.source_name_reads.iter().filter(|row| row.value == *value && row.expression == *expression && row.occurrence == registration.source_span
                && row.event.owner == cfg.producer_owner && row.event.point == original.id).collect::<Vec<_>>();
            let [name] = names.as_slice() else { return Err(fail()); };
            if !matches!(&original.kind, CfgPointKind::Read { binding } if binding == &name.binding) { return Err(fail()); }
        },
        ScalarExpression::Dereference { occurrence, place, .. } | ScalarExpression::IndexedRead { occurrence, place, .. } => {
            let reads = cfg.read_points.iter().filter(|row| row.point == original.id || row.occurrence == **occurrence).collect::<Vec<_>>();
            let [read] = reads.as_slice() else { return Err(fail()); };
            if occurrence.source_span != registration.source_span || read.point != original.id || read.occurrence != **occurrence
                || !matches!(&original.kind, CfgPointKind::ReadPlace { place: actual, .. } if actual == place) { return Err(fail()); }
        },
        ScalarExpression::Member { receiver, name, receiver_span, span, .. } => {
            let CfgPointKind::ReadPlace { place: ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(field), span: field_span }, pointer_point: None, .. } = &original.kind else { return Err(fail()); };
            if *field_span != *span || !matches!(base.as_ref(), ScalarPlace::Name { name: actual, span: actual_span } if actual == receiver && actual_span == receiver_span)
                || structs.iter().filter(|structure| structure.id == field.structure).flat_map(|structure| &structure.fields).filter(|declared| declared.id == *field && declared.name == *name).count() != 1
                || cfg.read_points.iter().any(|row| row.point == original.id) { return Err(fail()); }
        },
        ScalarExpression::Call { occurrence, .. } => {
            let productions = tables.productions.iter().filter(|row| row.producer == cfg.producer_owner && row.occurrence == registration.source_span && row.output_projection == occurrence.output_projection).collect::<Vec<_>>();
            let [production] = productions.as_slice() else { return Err(fail()); };
            let OutputProductionSource::Call { occurrence: actual, projections } = &production.source else { return Err(fail()); };
            let calls = cfg.call_points.iter().filter(|row| row.occurrence == **occurrence).collect::<Vec<_>>();
            let [call] = calls.as_slice() else { return Err(fail()); };
            let anchors = cfg.initial_points.get(production.evaluation_point.0).ok_or_else(fail)?;
            if actual != occurrence.as_ref() || occurrence.source_span != registration.source_span || call.evaluated_point != production.evaluation_point
                || call.projections != *projections || production.outputs.first() != Some(value)
                || projections.iter().filter(|projection| projection.output == 0 && projection.point == value.point).count() != 1
                || !matches!(&anchors.kind, CfgPointKind::Call { expression: actual, output: 0, .. } if actual == expression)
                || !matches!(&original.kind, CfgPointKind::Call { expression: actual, output: 0, .. } if actual == expression) { return Err(fail()); }
        },
        ScalarExpression::If { condition, then_branch, else_branch, .. } => {
            reference::conditional_output_production::validate(&candidate)?;
            let productions = tables.productions.iter().filter(|row| row.producer == cfg.producer_owner && row.occurrence == registration.source_span && row.output_projection == projection(expression)).collect::<Vec<_>>();
            let [production] = productions.as_slice() else { return Err(fail()); };
            let OutputProductionSource::Conditional { branch, .. } = &production.source else { return Err(fail()); };
            let branch_point = cfg.initial_points.get(branch.0).ok_or_else(fail)?;
            if production.outputs.first() != Some(value) || !matches!(&branch_point.kind, CfgPointKind::Branch { condition: actual } if actual == condition.as_ref())
                || cfg.conditional_points.iter().filter(|row| row.point == *branch && row.role == ScalarConditionalPointRole::Branch && row.then_block == *then_branch.occurrence).count() != 1
                || cfg.block_scope_exits.iter().filter(|row| row.occurrence == *else_branch.occurrence).count() != 1 { return Err(fail()); }
        },
        ScalarExpression::Block(block) => {
            reference::conditional_output_production::validate(&candidate)?;
            let productions = tables.productions.iter().filter(|row| row.producer == cfg.producer_owner && row.occurrence == registration.source_span && row.output_projection == block.occurrence.output_projection).collect::<Vec<_>>();
            let [production] = productions.as_slice() else { return Err(fail()); };
            let anchor = cfg.initial_points.get(production.evaluation_point.0).ok_or_else(fail)?;
            if production.outputs.first() != Some(value) || !matches!(&production.source, OutputProductionSource::Block { occurrence, .. } if occurrence == block.occurrence.as_ref())
                || !matches!(&anchor.kind, CfgPointKind::Evaluate(actual) if actual == expression) { return Err(fail()); }
        },
        ScalarExpression::Assignment(assignment) => {
            let candidates = tables.values.iter().filter(|row| row.id.resolved.producer == cfg.producer_owner && row.id.resolved.occurrence == registration.source_span && row.id.resolved.output == 0).collect::<Vec<_>>();
            if evaluated_aggregate_member::assignment_child(&candidate, assignment, &registration.source_span, &candidates)? != value { return Err(fail()); }
            let operation = AssignmentOperationId { producer: cfg.producer_owner.clone(), occurrence: (*assignment.occurrence).clone() };
            let plans = tables.assignment_plans.iter().filter(|plan| plan.operation == operation).collect::<Vec<_>>();
            let [plan] = plans.as_slice() else { return Err(fail()); };
            let productions = tables.productions.iter().filter(|row| row.producer == cfg.producer_owner && row.occurrence == registration.source_span && row.output_projection == assignment.occurrence.output_projection).collect::<Vec<_>>();
            let [production] = productions.as_slice() else { return Err(fail()); };
            if !matches!(&production.source, OutputProductionSource::Assignment { operation: actual } if actual == &operation)
                || production.outputs.first() != Some(value) || plan.targets.len() != assignment.targets.len()
                || plan.targets.iter().zip(&assignment.targets).enumerate().any(|(ordinal, (resolved, target))| resolved.ordinal != ordinal || resolved.place != target.place || resolved.source_span.range != target.target_span || resolved.source_span.source != registration.source_span.source) { return Err(fail()); }
        },
        _ => { if !matches!(&original.kind, CfgPointKind::Evaluate(actual) if actual == expression) { return Err(fail()); } },
    }
    Ok(value.clone())
}
