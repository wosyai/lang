use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar::evaluated_value::remap::input_counterpart) struct OriginalOutputIncidencePair {
    source_location: OriginalAstLocation,
    destination_location: OriginalAstLocation,
    source_block: ScalarBlockOccurrence,
    destination_block: ScalarBlockOccurrence,
    source_exits: Vec<ScalarBlockScopeExit>,
    destination_exits: Vec<ScalarBlockScopeExit>,
    source_aliases: Vec<BlockOutputAlias>,
    destination_aliases: Vec<BlockOutputAlias>,
    source_inputs: Vec<FinalOutputInput>,
    destination_inputs: Vec<FinalOutputInput>,
    source_productions: Vec<EvaluatedOutputProduction>,
    destination_productions: Vec<EvaluatedOutputProduction>,
}

fn source_type<'a>(visitor: &InputVisitor<'_>, cfg: &'a ScalarReferenceCfg, value: &EvaluatedValueId) -> Result<&'a ScalarType, ValueSchemaRemapError> {
    let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *value).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(visitor.invalid()); };
    let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape && shape.id.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(fact.shape.clone())); };
    if value.resolved.producer != cfg.producer_owner || value.resolved.occurrence.source != *lexical(&cfg.producer_owner) || cfg.initial_points.iter().filter(|point| point.id == value.point).count() != 1 { return Err(visitor.invalid()); }
    Ok(&shape.ty)
}

#[cfg(test)]
pub(super) fn proof_candidates(checked: &CheckedBoundaryInputTypes) -> Vec<CheckedBoundaryInputTypes> {
    let mut candidates = Vec::new();
    for (index, row) in checked.output_incidences.iter().enumerate() {
        for defect in 0..7 {
            let mut candidate = checked.clone();
            let actual = &mut candidate.output_incidences[index];
            match defect {
                0 if !row.destination_inputs.is_empty() => actual.destination_inputs[0].receiving_type = ScalarType::Error,
                1 if !row.destination_inputs.is_empty() => actual.destination_inputs[0].source.point = CfgPointId(usize::MAX),
                2 if !row.destination_inputs.is_empty() => actual.destination_inputs[0].consumer.point = CfgPointId(usize::MAX),
                3 if !row.destination_aliases.is_empty() => actual.destination_aliases[0].source.output_projection.push(usize::MAX),
                4 if !row.destination_productions.is_empty() => actual.destination_productions[0].evaluation_point = CfgPointId(usize::MAX),
                5 if !row.destination_productions.is_empty() => actual.destination_productions[0].outputs.clear(),
                6 if !row.destination_exits.is_empty() => actual.destination_exits[0].point = CfgPointId(usize::MAX),
                _ => continue,
            }
            candidates.push(candidate);
        }
    }
    candidates
}

pub(super) fn value(visitor: &mut InputVisitor<'_>, source: &EvaluatedValueId, destination: &EvaluatedValueId) -> Result<(), ValueSchemaRemapError> {
    visitor.require(visitor.point(source.point)? == destination.point && source.resolved.output == destination.resolved.output
        && source.resolved.output_projection == destination.resolved.output_projection && visitor.span(&source.resolved.occurrence, &destination.resolved.occurrence))?;
    let a = source_type(visitor, visitor.source, source)?.clone();
    let b = source_type(visitor, visitor.destination, destination)?.clone();
    let facts = visitor.source.evaluated_values.values.iter().filter(|fact| fact.id == *source).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(visitor.invalid()); };
    let actual_facts = visitor.destination.evaluated_values.values.iter().filter(|fact| fact.id == *destination).collect::<Vec<_>>();
    let [actual_fact] = actual_facts.as_slice() else { return Err(visitor.invalid()); };
    visitor.require(visitor.checked.qualification(visitor.remap, &visitor.source.producer_owner)?.shape(visitor.remap, &fact.shape)? == actual_fact.shape)?;
    let enclosing = visitor.root.clone();
    visitor.root = fact.shape.clone();
    visitor.ty(&a, &b)?;
    visitor.root = enclosing;
    Ok(())
}

fn aliases(visitor: &InputVisitor<'_>, cfg: &ScalarReferenceCfg, occurrence: &ScalarBlockOccurrence) -> Result<Vec<BlockOutputAlias>, ValueSchemaRemapError> {
    let mut records = Vec::new();
    let mut current = occurrence.clone();
    loop {
        let rows = cfg.evaluated_values.block_output_aliases.iter().filter(|alias| alias.selected == current).collect::<Vec<_>>();
        match rows.as_slice() {
            [] => return Ok(records),
            [alias] if alias.selected.source_span == alias.source.source_span && alias.selected.output_projection.len() == alias.source.output_projection.len() + 1
                && alias.selected.output_projection[1..] == alias.source.output_projection => { records.push((*alias).clone()); current = alias.source.clone(); }
            _ => return Err(visitor.invalid()),
        }
    }
}

pub(super) fn block(visitor: &mut InputVisitor<'_>, source: &ScalarBlock, destination: &ScalarBlock) -> Result<(), ValueSchemaRemapError> {
    let source_aliases = related_aliases(visitor, visitor.source, &source.occurrence)?;
    let destination_aliases = related_aliases(visitor, visitor.destination, &destination.occurrence)?;
    visitor.require(source_aliases.len() == destination_aliases.len())?;
    for (a, b) in source_aliases.iter().zip(&destination_aliases) {
        visitor.require(a.selected.output_projection == b.selected.output_projection && a.source.output_projection == b.source.output_projection
            && visitor.span(&a.selected.source_span, &b.selected.source_span) && visitor.span(&a.source.source_span, &b.source.source_span))?;
    }
    let source_exits = visitor.source.block_scope_exits.iter().filter(|row| row.occurrence == *source.occurrence || source_aliases.iter().any(|alias| alias.selected == row.occurrence || alias.source == row.occurrence)).cloned().collect::<Vec<_>>();
    let destination_exits = visitor.destination.block_scope_exits.iter().filter(|row| row.occurrence == *destination.occurrence || destination_aliases.iter().any(|alias| alias.selected == row.occurrence || alias.source == row.occurrence)).cloned().collect::<Vec<_>>();
    visitor.require(source_exits.len() == destination_exits.len())?;
    for (a, b) in source_exits.iter().zip(&destination_exits) { visitor.require(visitor.point(a.point)? == b.point && visitor.source.initial_points.iter().any(|point| point.id == a.point && matches!(point.kind, CfgPointKind::ScopeExit)) && visitor.destination.initial_points.iter().any(|point| point.id == b.point && matches!(point.kind, CfgPointKind::ScopeExit)))?; }
    let source_inputs = visitor.source.evaluated_values.final_inputs.iter().filter(|row| matches!(&row.consumer.kind, ValueConsumerKind::FinalOutput { block, .. } if *block == *source.occurrence)).cloned().collect::<Vec<_>>();
    let destination_inputs = visitor.destination.evaluated_values.final_inputs.iter().filter(|row| matches!(&row.consumer.kind, ValueConsumerKind::FinalOutput { block, .. } if *block == *destination.occurrence)).cloned().collect::<Vec<_>>();
    visitor.require(source_inputs.len() == destination_inputs.len())?;
    for (a, b) in source_inputs.iter().zip(&destination_inputs) {
        let (ValueConsumerKind::FinalOutput { output: x, .. }, ValueConsumerKind::FinalOutput { output: y, .. }) = (&a.consumer.kind, &b.consumer.kind) else { return Err(visitor.invalid()); };
        visitor.require(x == y && a.consumer.producer == visitor.source.producer_owner && b.consumer.producer == visitor.destination.producer_owner
            && visitor.point(a.consumer.point)? == b.consumer.point)?;
        visitor.ty(&a.receiving_type, &b.receiving_type)?;
        value(visitor, &a.source, &b.source)?;
        for (cfg, input) in [(visitor.source, a), (visitor.destination, b)] {
            visitor.require(cfg.initial_points.iter().filter(|point| point.id == input.consumer.point && matches!(point.kind, CfgPointKind::ReturnOutput { output, value_point, .. } if output == *x && value_point == input.source.point)).count() == 1)?;
        }
    }
    let source_productions = productions(visitor.source, &source.occurrence, &source_inputs, &source_exits);
    let destination_productions = productions(visitor.destination, &destination.occurrence, &destination_inputs, &destination_exits);
    visitor.require(source_productions.len() == destination_productions.len())?;
    for source in &source_productions {
        let point = visitor.point(source.evaluation_point)?;
        let targets = destination_productions.iter().filter(|target| target.evaluation_point == point && target.output_projection == source.output_projection && visitor.span(&source.occurrence, &target.occurrence)).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(visitor.invalid()); };
        production(visitor, source, target)?;
    }
    let record = OriginalOutputIncidencePair { source_location: visitor.source_location.clone(), destination_location: visitor.destination_location.clone(), source_block: (*source.occurrence).clone(), destination_block: (*destination.occurrence).clone(), source_exits, destination_exits, source_aliases, destination_aliases, source_inputs, destination_inputs, source_productions, destination_productions };
    if !visitor.checked.output_incidences.contains(&record) { visitor.checked.output_incidences.push(record); }
    Ok(())
}

fn related_aliases(visitor: &InputVisitor<'_>, cfg: &ScalarReferenceCfg, occurrence: &ScalarBlockOccurrence) -> Result<Vec<BlockOutputAlias>, ValueSchemaRemapError> {
    let own = aliases(visitor, cfg, occurrence)?;
    let mut records = own.clone();
    for alias in &cfg.evaluated_values.block_output_aliases {
        if alias.selected.source_span != occurrence.source_span { continue; }
        let chain = aliases(visitor, cfg, &alias.selected)?;
        let Some(last) = chain.last() else { return Err(visitor.invalid()); };
        if last.source == *occurrence || own.iter().any(|record| record.source == last.source) {
            for record in chain { if !records.contains(&record) { records.push(record); } }
        }
    }
    Ok(records)
}

pub(super) fn point_values(visitor: &mut InputVisitor<'_>, source: CfgPointId, destination: CfgPointId) -> Result<(), ValueSchemaRemapError> {
    let source_values = visitor.source.evaluated_values.values.iter().filter(|fact| fact.id.point == source).map(|fact| fact.id.clone()).collect::<Vec<_>>();
    let destination_values = visitor.destination.evaluated_values.values.iter().filter(|fact| fact.id.point == destination).map(|fact| fact.id.clone()).collect::<Vec<_>>();
    visitor.require(!source_values.is_empty() && source_values.len() == destination_values.len())?;
    values(visitor, &source_values, &destination_values)
}

pub(super) fn literal(visitor: &mut InputVisitor<'_>, source: &ReferenceFlowPoint, destination: &ReferenceFlowPoint, body: &ScalarBlock, actual_body: &ScalarBlock) -> Result<(), ValueSchemaRemapError> {
    let (CfgPointKind::Capture { signature: ScalarType::Callable { outputs: a, .. }, .. }, CfgPointKind::Capture { signature: ScalarType::Callable { outputs: b, .. }, .. }) = (&source.kind, &destination.kind) else { return Err(visitor.invalid()); };
    visitor.require(a.outputs.len() == b.outputs.len() && a.span == b.span)?;
    let inputs = visitor.source.evaluated_values.final_inputs.iter().filter(|row| matches!(&row.consumer.kind, ValueConsumerKind::FinalOutput { block, .. } if *block == *body.occurrence)).collect::<Vec<_>>();
    let targets = visitor.destination.evaluated_values.final_inputs.iter().filter(|row| matches!(&row.consumer.kind, ValueConsumerKind::FinalOutput { block, .. } if *block == *actual_body.occurrence)).collect::<Vec<_>>();
    visitor.require(inputs.len() == targets.len())?;
    if !body.final_output_values.is_empty() { visitor.require(inputs.len() == a.outputs.len() && targets.len() == b.outputs.len())?; }
    for (input, target) in inputs.iter().zip(&targets) {
        let (ValueConsumerKind::FinalOutput { output: x, .. }, ValueConsumerKind::FinalOutput { output: y, .. }) = (&input.consumer.kind, &target.consumer.kind) else { return Err(visitor.invalid()); };
        let (Some(a), Some(b)) = (a.outputs.get(*x), b.outputs.get(*y)) else { return Err(visitor.invalid()); };
        visitor.require(x == y && input.receiving_type == a.ty && target.receiving_type == b.ty)?;
        visitor.ty(&a.ty, &b.ty)?;
    }
    Ok(())
}

fn productions(cfg: &ScalarReferenceCfg, block: &ScalarBlockOccurrence, inputs: &[FinalOutputInput], exits: &[ScalarBlockScopeExit]) -> Vec<EvaluatedOutputProduction> {
    cfg.evaluated_values.productions.iter().filter(|production| production.outputs.iter().any(|output| inputs.iter().any(|input| input.source == *output)) || match &production.source {
        OutputProductionSource::Block { occurrence, .. } => occurrence == block,
        OutputProductionSource::Conditional { inputs, .. } => inputs.iter().any(|input| exits.iter().any(|exit| exit.point == input.predecessor)),
        _ => false,
    }).cloned().collect()
}

fn values(visitor: &mut InputVisitor<'_>, source: &[EvaluatedValueId], destination: &[EvaluatedValueId]) -> Result<(), ValueSchemaRemapError> {
    visitor.require(source.len() == destination.len())?;
    for (a, b) in source.iter().zip(destination) { value(visitor, a, b)?; }
    Ok(())
}

fn production(visitor: &mut InputVisitor<'_>, source: &EvaluatedOutputProduction, destination: &EvaluatedOutputProduction) -> Result<(), ValueSchemaRemapError> {
    visitor.require(source.producer == visitor.source.producer_owner && destination.producer == visitor.destination.producer_owner
        && visitor.span(&source.occurrence, &destination.occurrence) && source.output_projection == destination.output_projection && visitor.point(source.evaluation_point)? == destination.evaluation_point)?;
    values(visitor, &source.outputs, &destination.outputs)?;
    for (cfg, production) in [(visitor.source, source), (visitor.destination, destination)] {
        visitor.require(cfg.initial_points.iter().filter(|point| point.id == production.evaluation_point).count() == 1)?;
        for (ordinal, output) in production.outputs.iter().enumerate() {
            visitor.require(output.resolved.producer == cfg.producer_owner && output.resolved.occurrence == production.occurrence && output.resolved.output_projection == production.output_projection && output.resolved.output == ordinal)?;
        }
    }
    match (&source.source, &destination.source) {
        (OutputProductionSource::Direct, OutputProductionSource::Direct) => Ok(()),
        (OutputProductionSource::Call { occurrence: a, projections: x }, OutputProductionSource::Call { occurrence: b, projections: y }) => {
            visitor.require(a.output_projection == b.output_projection && visitor.span(&a.source_span, &b.source_span) && x.len() == y.len())?;
            for (a, b) in x.iter().zip(y) { visitor.require(a.output == b.output && visitor.point(a.point)? == b.point)?; }
            Ok(())
        }
        (OutputProductionSource::Conditional { branch: a, inputs: x }, OutputProductionSource::Conditional { branch: b, inputs: y }) => {
            visitor.require(visitor.point(*a)? == *b && x.len() == y.len())?;
            for (a, b) in x.iter().zip(y) { visitor.require(visitor.point(a.predecessor)? == b.predecessor)?; values(visitor, &a.outputs, &b.outputs)?; }
            Ok(())
        }
        (OutputProductionSource::Block { occurrence: a, scope_exit: x, inputs: p }, OutputProductionSource::Block { occurrence: b, scope_exit: y, inputs: q }) => {
            visitor.require(a.output_projection == b.output_projection && visitor.span(&a.source_span, &b.source_span) && visitor.point(*x)? == *y)?;
            values(visitor, p, q)
        }
        (OutputProductionSource::Assignment { operation: a }, OutputProductionSource::Assignment { operation: b }) => {
            visitor.require(a.producer == visitor.source.producer_owner && b.producer == visitor.destination.producer_owner && a.occurrence.output_projection == b.occurrence.output_projection && visitor.span(&a.occurrence.source_span, &b.occurrence.source_span))
        }
        _ => Err(visitor.invalid()),
    }
}
