use super::*;

pub(super) fn connected(cfg: &ScalarReferenceCfg, from: CfgPointId, to: CfgPointId) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![from];
    while let Some(id) = pending.pop() {
        if id == to { return true; }
        if seen.insert(id) {
            let Some(point) = cfg.initial_points.get(id.0) else { return false; };
            pending.extend(&point.successors);
        }
    }
    false
}

pub(super) fn operand(cfg: &ScalarReferenceCfg, inventory: &Inventory, occurrence: &SourceSpan, block: ByteSpan, expression: &ScalarExpression, id: CfgPointId) -> Result<(), ReferenceAnalysisError> {
    let fail = || invalid(occurrence);
    let point = cfg.initial_points.get(id.0).ok_or_else(fail)?;
    if point.id != id || point.source_span.source != occurrence.source || !scope_matches(&cfg.initial_points, point, block) { return Err(fail()); }
    if matches!(expression, ScalarExpression::Block(_) | ScalarExpression::If { .. } | ScalarExpression::Call { .. }) {
        let rows = cfg.evaluated_values.productions.iter().filter(|row| row.producer == cfg.producer_owner
            && row.occurrence == SourceSpan::new(occurrence.source.clone(), expression_span(expression)) && row.output_projection.is_empty()).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { return Err(fail()); };
        let Some(value) = row.outputs.first() else { return Err(fail()); };
        if row.evaluation_point != id || value.point != id || value.resolved.producer != cfg.producer_owner || value.resolved.output != 0
            || value.resolved.occurrence != row.occurrence || value.resolved.output_projection != row.output_projection
            || cfg.evaluated_values.values.iter().filter(|actual| actual.id == *value).count() != 1 { return Err(fail()); }
        match (&row.source, expression) {
            (OutputProductionSource::Call { occurrence, .. }, ScalarExpression::Call { occurrence: actual, .. }) if occurrence == actual.as_ref() => {
                if !matches!(&point.kind, CfgPointKind::Call { expression: actual, output: 0, .. } if actual == expression) { return Err(fail()); }
            },
            (OutputProductionSource::Block { occurrence, .. }, ScalarExpression::Block(body)) if occurrence == body.occurrence.as_ref() => {
                if !matches!(&point.kind, CfgPointKind::Evaluate(actual) if actual == expression) { return Err(fail()); }
                conditional_output_production::validate_original_control_anchor(cfg, row)?;
            },
            (OutputProductionSource::Conditional { branch, .. }, ScalarExpression::If { condition, .. }) => {
                if !cfg.initial_points.get(branch.0).is_some_and(|branch| matches!(&branch.kind, CfgPointKind::Branch { condition: actual } if actual == condition.as_ref()) && connected(cfg, branch.id, id)) { return Err(fail()); }
                conditional_output_production::validate_original_control_anchor(cfg, row)?;
            },
            _ => return Err(fail()),
        }
        return Ok(());
    }
    if point.source_span.range != expression_span(expression) { return Err(fail()); }
    match (&point.kind, expression) {
        (CfgPointKind::Evaluate(actual), _) if actual == expression => {},
        (CfgPointKind::Read { .. }, ScalarExpression::Name { .. }) => {
            let names = inventory.names.iter().filter(|row| row.expression == *expression && row.block == block).collect::<Vec<_>>();
            let [name] = names.as_slice() else { return Err(fail()); };
            let NameSource::Value(declaration) = &name.source else { return Err(fail()); };
            let required = fact(cfg, point, name, declaration)?;
            if cfg.evaluated_values.source_name_reads.iter().filter(|actual| **actual == required).count() != 1 { return Err(fail()); }
        },
        (CfgPointKind::ReadPlace { place, .. }, ScalarExpression::Dereference { place: actual, .. } | ScalarExpression::IndexedRead { place: actual, .. }) if place == actual => {},
        (CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { place, loan, .. } }, ScalarExpression::CheckedAddress { place: actual, mutability, .. }) if place.place == *actual && loan.mode == *mutability => {},
        _ => return Err(fail()),
    }
    Ok(())
}

