use super::*;

fn source_place(
    points: &[ReferenceFlowPoint],
    input: &SourceMoveInput,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    let point = points
        .get(input.source.point.0)
        .ok_or_else(|| invalid(source))?;
    let matches = match &point.kind {
        CfgPointKind::Read { binding } => {
            input.place.projections.is_empty() && *binding == input.place.binding
        }
        CfgPointKind::ReadPlace {
            candidates,
            pointer_point: None,
            ..
        } => candidates.as_slice() == std::slice::from_ref(&input.place),
        _ => false,
    };
    if !matches || point.source_span != input.source.resolved.occurrence {
        return Err(invalid(source));
    }
    Ok(())
}

fn pending(
    tables: &mut EvaluatedValueTables,
    event: &SourceEventId,
    reason: SourceMovePendingReason,
) {
    let fact = SourceMovePendingFact {
        event: event.clone(),
        reason,
    };
    if !tables.pending_source_moves.contains(&fact) {
        tables.pending_source_moves.push(fact);
    }
}

fn moved_availability(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    place: &ReferencePlaceId,
    point: &ReferenceFlowPoint,
) -> Result<Option<ValueAvailability>, ReferenceAnalysisError> {
    let slot = selected_slot(
        tables,
        row,
        &SourceCoordinateRole::Binding(place.binding.clone()),
    )
    .map_err(|_| invalid(&point.source_span))?;
    if place.projections.is_empty() {
        if slot.availability != ValueAvailability::Available {
            return Err(invalid(&point.source_span));
        }
        return Ok(Some(ValueAvailability::Moved {
            place: place.clone(),
            move_point: point.id,
            source: point.source_span.clone(),
        }));
    }
    let [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field))] =
        place.projections.as_slice()
    else {
        return Ok(None);
    };
    let JointValueTransportKind::Project(port) =
        (match port_monitor::place_transport(tables, row, place) {
            Ok(port) => port,
            Err(()) => return Ok(None),
        })
    else {
        return Ok(None);
    };
    let SourcePayloadView::Aggregate(members) = &tables.source_views[slot.view.index].payload
    else {
        return Ok(None);
    };
    let projection = members[port.member].projection.clone();
    if projection != vec![ValueProjection::Field(field.clone())] {
        return Err(invalid(&point.source_span));
    }
    let mut states = match &slot.availability {
        ValueAvailability::Available => Vec::new(),
        ValueAvailability::PartiallyUnavailable { states } => states.clone(),
        _ => return Err(invalid(&point.source_span)),
    };
    let matches = states
        .iter()
        .filter(|state| state.projection == projection)
        .collect::<Vec<_>>();
    if matches.len() > 1
        || matches
            .iter()
            .any(|state| state.state != SubplaceState::Available)
    {
        return Err(invalid(&point.source_span));
    }
    states.retain(|state| state.projection != projection);
    states.push(SubplaceAvailability {
        projection,
        state: SubplaceState::Moved {
            point: point.id,
            source: point.source_span.clone(),
        },
    });
    Ok(Some(ValueAvailability::PartiallyUnavailable { states }))
}

pub(super) fn prepare(
    tables: &mut EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    input: &SourceConfigurationFact,
    output: &mut SourceConfigurationFact,
    point: &ReferenceFlowPoint,
    event: &SourceEventId,
    monitor: &JointPortMonitorRef,
) -> Result<Option<(SourceMoveInput, JointSelectedPortFact)>, ReferenceAnalysisError> {
    let place = match &point.kind {
        CfgPointKind::Move { place, .. } => place,
        CfgPointKind::DeferredMove { .. } | CfgPointKind::DeferredMoveThrough { .. } => {
            pending(tables, event, SourceMovePendingReason::Deferred);
            return Ok(None);
        }
        CfgPointKind::MoveThrough { .. } => {
            pending(tables, event, SourceMovePendingReason::Through);
            return Ok(None);
        }
        _ => return Ok(None),
    };
    let inputs = tables
        .source_move_inputs
        .iter()
        .filter(|input| input.event == *event)
        .cloned()
        .collect::<Vec<_>>();
    let [source] = inputs.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    if source.place != *place {
        return Err(invalid(&point.source_span));
    }
    source_place(points, source, &point.source_span)?;
    let Some(availability) = moved_availability(tables, input, place, point)? else {
        pending(
            tables,
            event,
            SourceMovePendingReason::UnsupportedPlace(place.clone()),
        );
        return Ok(None);
    };
    let selected = match selected_slot(
        tables,
        input,
        &SourceCoordinateRole::Evaluated(source.source.clone()),
    ) {
        Ok(selected) => selected.clone(),
        Err(()) => {
            pending(
                tables,
                event,
                SourceMovePendingReason::SelectedSource(source.clone()),
            );
            return Ok(None);
        }
    };
    let Some(selection) =
        port_monitor::selection(tables, monitor, &selected.coordinate, &point.source_span)?
    else {
        pending(
            tables,
            event,
            SourceMovePendingReason::SelectedSource(source.clone()),
        );
        return Ok(None);
    };
    let original = projected_view(tables, input, place).map_err(|_| invalid(&point.source_span))?;
    if selected.availability != ValueAvailability::Available
        || selected.view != original
        || selection.observation.view != original
        || selection.observation.case.value != source.source
    {
        return Err(invalid(&point.source_span));
    }
    let slot = selected_slot(
        tables,
        input,
        &SourceCoordinateRole::Binding(place.binding.clone()),
    )
    .map_err(|_| invalid(&point.source_span))?;
    let coordinate = slot.coordinate.clone();
    let view = slot.view.clone();
    install(output, coordinate, view, availability);
    Ok(Some((source.clone(), selection)))
}

pub(super) fn register(
    tables: &mut EvaluatedValueTables,
    input: SourceMoveInput,
    selection: JointSelectedPortFact,
    tuple: &JointProducerTupleRef,
) -> SourceMoveWitnessRef {
    let rule = &tables.source_contributors[tuple.index];
    let mut fact = SourceMoveWitnessFact {
        id: SourceMoveWitnessRef {
            owner: tuple.owner.clone(),
            index: 0,
        },
        event: input.event,
        input: rule.input.clone(),
        output: rule.output.clone(),
        contributor: tuple.clone(),
        source: input.source,
        selection,
        place: input.place,
    };
    if let Some(previous) = tables.source_move_witnesses.iter().find(|previous| {
        let mut candidate = fact.clone();
        candidate.id = previous.id.clone();
        candidate == **previous
    }) {
        return previous.id.clone();
    }
    fact.id.index = tables.source_move_witnesses.len();
    let id = fact.id.clone();
    tables.source_move_witnesses.push(fact);
    id
}

pub(super) fn validate(
    tables: &EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    id: &SourceMoveWitnessRef,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    let fact = tables
        .source_move_witnesses
        .get(id.index)
        .filter(|fact| fact.id == *id)
        .ok_or_else(|| invalid(source))?;
    let rule = tables
        .source_contributors
        .get(fact.contributor.index)
        .filter(|rule| rule.id == fact.contributor)
        .ok_or_else(|| invalid(source))?;
    let point = points
        .get(fact.event.point.0)
        .ok_or_else(|| invalid(source))?;
    let CfgPointKind::Move { place, .. } = &point.kind else {
        return Err(invalid(source));
    };
    source_place(
        points,
        &SourceMoveInput {
            event: fact.event.clone(),
            source: fact.source.clone(),
            place: fact.place.clone(),
        },
        source,
    )?;
    if fact.id.owner != fact.event.owner
        || rule.event != fact.event
        || rule.input != fact.input
        || rule.output != fact.output
        || *place != fact.place
        || fact.selection.observation.case.value != fact.source
        || fact.selection.moves.contains(id)
        || tables
            .source_move_inputs
            .iter()
            .filter(|input| {
                input.event == fact.event
                    && input.source == fact.source
                    && input.place == fact.place
            })
            .count()
            != 1
    {
        return Err(invalid(source));
    }
    let input = tables
        .source_configurations
        .get(fact.input.index)
        .filter(|row| row.id == fact.input)
        .ok_or_else(|| invalid(source))?;
    let output = tables
        .source_configurations
        .get(fact.output.index)
        .filter(|row| row.id == fact.output)
        .ok_or_else(|| invalid(source))?;
    let selected = selected_slot(
        tables,
        input,
        &SourceCoordinateRole::Evaluated(fact.source.clone()),
    )
    .map_err(|_| invalid(source))?;
    let after = selected_slot(
        tables,
        output,
        &SourceCoordinateRole::Evaluated(fact.source.clone()),
    )
    .map_err(|_| invalid(source))?;
    let storage = selected_slot(
        tables,
        output,
        &SourceCoordinateRole::Binding(fact.place.binding.clone()),
    )
    .map_err(|_| invalid(source))?;
    selected_grant::validate_view(tables, input, &selected.view, source)?;
    selected_grant::validate_view(tables, output, &after.view, source)?;
    selected_grant::read(tables, input, &fact.selection, source)?;
    if input.grants != output.grants || input.grant_ancestry != output.grant_ancestry {
        return Err(invalid(source));
    }
    if selected != after
        || selected.availability != ValueAvailability::Available
        || selected.coordinate != fact.selection.coordinate
        || selected.view != fact.selection.observation.view
        || projected_view(tables, input, &fact.place).map_err(|_| invalid(source))? != selected.view
        || moved_availability(tables, input, &fact.place, point)?
            != Some(storage.availability.clone())
    {
        return Err(invalid(source));
    }
    let equations = tables
        .source_port_equations
        .iter()
        .filter(|equation| equation.contributor == fact.contributor && equation.moves.contains(id))
        .collect::<Vec<_>>();
    if equations.is_empty() {
        return Err(invalid(source));
    }
    for equation in equations {
        let monitor = tables
            .source_port_monitors
            .get(equation.input.index)
            .filter(|monitor| monitor.id == equation.input)
            .ok_or_else(|| invalid(source))?;
        let incoming = tables
            .source_contributors
            .get(monitor.contributor.index)
            .filter(|rule| rule.id == monitor.contributor)
            .ok_or_else(|| invalid(source))?;
        if incoming.output != fact.input || !monitor.ports.contains(&fact.selection) {
            return Err(invalid(source));
        }
    }
    Ok(())
}

pub(super) fn read(
    tables: &EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    selection: &JointSelectedPortFact,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    for id in &selection.moves {
        validate(tables, points, id, source)?;
        let witness = &tables.source_move_witnesses[id.index];
        if witness.selection.observation != selection.observation
            && !selection.sources.contains(&witness.selection.observation)
        {
            return Err(invalid(source));
        }
    }
    Ok(())
}
