use super::*;

pub(super) fn member(tables: &EvaluatedValueTables, id: &SourceFormalValueRef, projection: &ReferencePlaceProjection) -> Result<SourceFormalValueRef, ()> {
    let value = source_boundary::formal_value::resolve(tables, id)?;
    let shape = tables.shapes.iter().find(|shape| shape.id == value.shape).ok_or(())?;
    let position = match (&shape.payload, projection) {
        (ValuePayloadShape::Struct { fields, .. }, ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field))) => {
            let positions = fields.iter().enumerate().filter(|(_, row)| row.field == *field).map(|(index, _)| index).collect::<Vec<_>>();
            let [position] = positions.as_slice() else { return Err(()); };
            *position
        },
        (ValuePayloadShape::FixedArray { length, .. }, ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }))
            if *value >= BigInt::from(0) && *value < BigInt::from(*length) => 0,
        (ValuePayloadShape::FixedArray { .. } | ValuePayloadShape::RuntimeArray { .. }, ReferencePlaceProjection::Index(_)) => 0,
        (ValuePayloadShape::Raw { .. } | ValuePayloadShape::Checked { .. }, ReferencePlaceProjection::Dereference(_)) => 0,
        _ => return Err(()),
    };
    let edge = value.members.get(position).ok_or(())?;
    if edge.member.shape != value.shape || edge.member.member != position { return Err(()); }
    let child = source_boundary::formal_value::resolve(tables, &edge.value)?;
    if child.port != value.port { return Err(()); }
    Ok(child.id.clone())
}

pub(super) fn selected_view(tables: &EvaluatedValueTables, id: &SourceFormalValueRef) -> Result<SourceValueViewRef, ()> {
    let formal = source_boundary::formal_value::resolve(tables, id)?;
    let views = tables.source_views.iter().filter(|view| view.shape == formal.shape && view.payload == SourcePayloadView::Formal(id.clone())).collect::<Vec<_>>();
    let [view] = views.as_slice() else { return Err(()); };
    Ok(view.id.clone())
}

pub(super) fn entry(cfg: &mut ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    source_boundary::formal_value::validate(cfg)?;
    for row in cfg.evaluated_values.source_formal_values.clone() {
        intern_view(&mut cfg.evaluated_values, &cfg.producer_owner, row.shape, SourcePayloadView::Formal(row.id), Vec::new());
    }
    Ok(())
}

pub(super) fn input(tables: &EvaluatedValueTables, rule: &JointProducerTupleFact, production: &JointPortProductionFact, coordinate: &SourceSlotRef) -> Result<bool, ()> {
    let row = tables.source_configurations.get(rule.input.index).filter(|row| row.id == rule.input).ok_or(())?;
    let slot = row.slots.iter().find(|slot| slot.coordinate == *coordinate).ok_or(())?;
    let SourcePayloadView::Formal(source) = &tables.source_views.get(slot.view.index).filter(|view| view.id == slot.view).ok_or(())?.payload else { return Ok(false); };
    let value = source_boundary::formal_value::resolve(tables, source)?;
    let selected = tables.source_views.get(production.observation.view.index).filter(|view| view.id == production.observation.view).ok_or(())?;
    let SourcePayloadView::Formal(destination) = &selected.payload else { return Ok(false); };
    let actual = source_boundary::formal_value::resolve(tables, destination)?;
    if value.port != actual.port || slot.availability != ValueAvailability::Available { return Err(()); }
    let root = row.formal_inputs.iter().filter(|input| input.port == value.port).collect::<Vec<_>>();
    let [root] = root.as_slice() else { return Err(()); };
    if root.coordinate.owner != rule.event.owner { return Err(()); }
    Ok(true)
}

pub(super) fn record(tables: &mut EvaluatedValueTables, point: &ReferenceFlowPoint, registrations: &[ScalarReadCfgPoint], tuple: &JointProducerTupleRef, monitor: &JointPortMonitorRef) -> Result<(), ReferenceAnalysisError> {
    if !matches!(point.kind, CfgPointKind::Read { .. } | CfgPointKind::ReadPlace { .. }) { return Ok(()); }
    let rule = tables.source_contributors.get(tuple.index).filter(|rule| rule.id == *tuple).ok_or_else(|| invalid(&point.source_span))?.clone();
    let row = tables.source_configurations.get(rule.output.index).filter(|row| row.id == rule.output).ok_or_else(|| invalid(&point.source_span))?.clone();
    for value in &rule.outputs {
        let slot = selected_slot(tables, &row, &SourceCoordinateRole::Evaluated(value.clone())).map_err(|_| invalid(&point.source_span))?;
        let SourcePayloadView::Formal(formal) = &tables.source_views[slot.view.index].payload else { continue; };
        let formal = formal.clone();
        let selected = port_monitor::selection(tables, monitor, &slot.coordinate, &point.source_span)?.ok_or_else(|| invalid(&point.source_span))?;
        let (projections, indexes) = match &point.kind {
            CfgPointKind::Read { .. } => (Vec::new(), Vec::new()),
            CfgPointKind::ReadPlace { candidates, index_points, .. } => {
                let [place] = candidates.as_slice() else { return Err(invalid(&point.source_span)); };
                (place.projections.clone(), index_points.clone())
            },
            _ => unreachable!(),
        };
        let witness = SourceFormalReadWitness { event: rule.event.clone(), source: formal, value: value.clone(), input: rule.input.clone(), output: rule.output.clone(), contributor: tuple.clone(), monitor: monitor.clone(), selected, projections, indexes };
        validate_read(tables, point, registrations, &witness)?;
        if !tables.source_formal_reads.contains(&witness) { tables.source_formal_reads.push(witness); }
    }
    Ok(())
}

fn validate_read(tables: &EvaluatedValueTables, point: &ReferenceFlowPoint, registrations: &[ScalarReadCfgPoint], witness: &SourceFormalReadWitness) -> Result<(), ReferenceAnalysisError> {
    let fail = || invalid(&point.source_span);
    let formal = source_boundary::formal_value::resolve(tables, &witness.source).map_err(|_| fail())?;
    let rule = tables.source_contributors.get(witness.contributor.index).filter(|row| row.id == witness.contributor).ok_or_else(fail)?;
    let row = tables.source_configurations.get(witness.input.index).filter(|row| row.id == witness.input).ok_or_else(fail)?;
    if rule.input != witness.input || rule.output != witness.output || rule.event != witness.event || witness.event.point != point.id
        || witness.event.owner != formal.id.owner || !rule.outputs.contains(&witness.value)
        || witness.selected.observation.case.value != witness.value { return Err(fail()); }
    let registrations = registrations.iter().filter(|row| row.point == point.id).collect::<Vec<_>>();
    let (binding, projections, indexes) = match &point.kind {
        CfgPointKind::Read { binding } => (binding, Vec::new(), Vec::new()),
        CfgPointKind::ReadPlace { candidates, index_points, .. } => {
            let [place] = candidates.as_slice() else { return Err(fail()); };
            (&place.binding, place.projections.clone(), index_points.clone())
        },
        _ => return Err(fail()),
    };
    if witness.projections != projections || witness.indexes != indexes { return Err(fail()); }
    if matches!(point.kind, CfgPointKind::ReadPlace { .. }) {
        let [registered] = registrations.as_slice() else { return Err(fail()); };
        if registered.indices.len() != indexes.len() { return Err(fail()); }
        for (registered, actual) in registered.indices.iter().zip(&indexes) { if registered.point != *actual { return Err(fail()); } }
    }
    let values = tables.values.iter().filter(|value| value.id == witness.value && value.shape == formal.shape).collect::<Vec<_>>();
    if values.len() != 1 || witness.value.point != point.id { return Err(fail()); }
    let slot = selected_slot(tables, row, &SourceCoordinateRole::Binding(binding.clone())).map_err(|_| fail())?;
    let SourcePayloadView::Formal(mut source) = tables.source_views[slot.view.index].payload.clone() else { return Err(fail()); };
    for projection in &projections { source = member(tables, &source, projection).map_err(|_| fail())?; }
    if source != witness.source { return Err(fail()); }
    let selected = port_monitor::selection(tables, &witness.monitor, &witness.selected.coordinate, &point.source_span)?.ok_or_else(fail)?;
    if selected != witness.selected || selected.observation.contributor != witness.contributor || selected.observation.configuration != witness.output
        || tables.source_views[selected.observation.view.index].payload != SourcePayloadView::Formal(witness.source.clone()) { return Err(fail()); }
    Ok(())
}

pub(super) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    source_boundary::formal_value::validate(cfg)?;
    for witness in &cfg.evaluated_values.source_formal_reads {
        let point = cfg.initial_points.get(witness.event.point.0).filter(|point| point.id == witness.event.point).ok_or_else(|| invalid(&cfg.points[0].source_span))?;
        validate_read(&cfg.evaluated_values, point, &cfg.read_points, witness)?;
    }
    Ok(())
}
