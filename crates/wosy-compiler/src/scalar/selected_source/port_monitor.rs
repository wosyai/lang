use super::*;

fn resolve<'a>(
    tables: &'a EvaluatedValueTables,
    id: &JointPortMonitorRef,
    source: &SourceSpan,
) -> Result<&'a JointPortMonitorFact, ReferenceAnalysisError> {
    tables
        .source_port_monitors
        .get(id.index)
        .filter(|row| row.id == *id)
        .ok_or_else(|| invalid(source))
}

fn resolve_rule<'a>(
    tables: &'a EvaluatedValueTables,
    id: &JointProducerTupleRef,
    source: &SourceSpan,
) -> Result<&'a JointProducerTupleFact, ReferenceAnalysisError> {
    tables
        .source_contributors
        .get(id.index)
        .filter(|row| row.id == *id)
        .ok_or_else(|| invalid(source))
}

fn intern(tables: &mut EvaluatedValueTables, mut row: JointPortMonitorFact) -> JointPortMonitorRef {
    row.ports.sort_by_key(|port| port.coordinate.index);
    row.pending.sort_by_key(|port| port.index);
    row.pending.dedup();
    if let Some(previous) = tables.source_port_monitors.iter().find(|previous| {
        let mut candidate = row.clone();
        candidate.id = previous.id.clone();
        candidate == **previous
    }) {
        return previous.id.clone();
    }
    row.id.index = tables.source_port_monitors.len();
    let id = row.id.clone();
    tables.source_port_monitors.push(row);
    id
}

pub(super) fn root(
    tables: &mut EvaluatedValueTables,
    contributor: &JointProducerTupleRef,
) -> JointPortMonitorRef {
    let rule = &tables.source_contributors[contributor.index];
    let pending = tables.source_configurations[rule.output.index]
        .slots
        .iter()
        .map(|slot| slot.coordinate.clone())
        .collect();
    intern(
        tables,
        JointPortMonitorFact {
            id: JointPortMonitorRef {
                owner: contributor.owner.clone(),
                index: 0,
            },
            contributor: contributor.clone(),
            ports: Vec::new(),
            pending,
        },
    )
}

fn source_case(
    tables: &EvaluatedValueTables,
    observation: &SourceCaseViewFact,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    if !tables.source_case_views.contains(observation) {
        return Err(invalid(source));
    }
    let rule = resolve_rule(tables, &observation.contributor, source)?;
    let path = tables
        .source_paths
        .iter()
        .find(|path| path.id == observation.path)
        .ok_or_else(|| invalid(source))?;
    let value = tables
        .values
        .iter()
        .find(|value| value.id == observation.case.value)
        .ok_or_else(|| invalid(source))?;
    let case = value
        .cases
        .get(observation.case.alternative)
        .filter(|case| case.id == observation.case)
        .ok_or_else(|| invalid(source))?;
    let slot = tables.source_configurations[rule.output.index]
        .slots
        .iter()
        .find(|slot| slot.coordinate == observation.port)
        .ok_or_else(|| invalid(source))?;
    if path.contributor != rule.id
        || path.configuration != observation.configuration
        || observation.configuration != rule.output
        || path.event != rule.event
        || case.path != path.id
        || slot.view != observation.view
        || slot.availability != case.availability
        || !rule.outputs.contains(&observation.case.value)
    {
        return Err(invalid(source));
    }
    Ok(())
}

fn append_sources(sources: &mut Vec<SourceCaseViewFact>, port: &JointSelectedPortFact) {
    if !sources.contains(&port.observation) {
        sources.push(port.observation.clone());
    }
    for source in &port.sources {
        if !sources.contains(source) {
            sources.push(source.clone());
        }
    }
}

fn evaluate(
    tables: &EvaluatedValueTables,
    equation: &JointPortMonitorEquation,
    source: &SourceSpan,
) -> Result<JointPortMonitorFact, ReferenceAnalysisError> {
    let rule = resolve_rule(tables, &equation.contributor, source)?;
    let input = resolve(tables, &equation.input, source)?;
    let input_rule = resolve_rule(tables, &input.contributor, source)?;
    if input_rule.output != rule.input {
        return Err(invalid(source));
    }
    let output = tables
        .source_configurations
        .get(rule.output.index)
        .filter(|row| row.id == rule.output)
        .ok_or_else(|| invalid(source))?;
    let before = tables
        .source_configurations
        .get(rule.input.index)
        .filter(|row| row.id == rule.input)
        .ok_or_else(|| invalid(source))?;
    if rule.preserved.iter().any(|instance| {
        !before.instances.contains(instance) || !output.instances.contains(instance)
    }) || rule
        .fresh
        .iter()
        .any(|instance| before.instances.contains(instance) || !output.instances.contains(instance))
    {
        return Err(invalid(source));
    }
    let mut ports = Vec::new();
    for child in &equation.children {
        if equation
            .children
            .iter()
            .filter(|other| other.event == child.event)
            .count()
            != 1
        {
            return Err(invalid(source));
        }
        let valuation = resolve(tables, &child.monitor, source)?;
        let selected_rule = resolve_rule(tables, &valuation.contributor, source)?;
        if selected_rule.event != child.event
            || !rule.children.iter().any(|port| {
                port.event == child.event
                    && port.configuration == selected_rule.output
                    && port.ports == child.ports
            })
        {
            return Err(invalid(source));
        }
    }
    for transport in &rule.transports {
        if rule
            .transports
            .iter()
            .filter(|other| other.destination == transport.destination)
            .count()
            != 1
        {
            return Err(invalid(source));
        }
        let valuation = match &transport.child {
            JointValueChildPort::LocalInput => input,
            JointValueChildPort::SelectedChild { event, port } => {
                let child = equation
                    .children
                    .iter()
                    .find(|child| child.event == *event)
                    .ok_or_else(|| invalid(source))?;
                if child.ports.get(*port) != Some(&transport.source) {
                    return Err(invalid(source));
                }
                resolve(tables, &child.monitor, source)?
            }
        };
        let Some(selected) = valuation
            .ports
            .iter()
            .find(|selected| selected.coordinate == transport.source)
        else {
            continue;
        };
        let destination = output
            .slots
            .iter()
            .find(|slot| slot.coordinate == transport.destination)
            .ok_or_else(|| invalid(source))?;
        if matches!(transport.kind, JointValueTransportKind::Preserve)
            && (transport.source != transport.destination
                || !before.slots.iter().any(|slot| slot == destination))
        {
            return Err(invalid(source));
        }
        if matches!(transport.kind, JointValueTransportKind::Move)
            && output.slots.iter().any(|slot| {
                slot.coordinate == transport.source
                    && slot.availability == ValueAvailability::Available
            })
        {
            return Err(invalid(source));
        }
        let mut selected = selected.clone();
        if let JointValueTransportKind::Project(member) = &transport.kind {
            let view = &tables.source_views[selected.observation.view.index];
            if view.shape != member.shape {
                return Err(invalid(source));
            }
            let SourcePayloadView::Aggregate(members) = &view.payload else {
                return Err(invalid(source));
            };
            let member = members.get(member.member).ok_or_else(|| invalid(source))?;
            let candidates = selected
                .sources
                .iter()
                .filter(|case| case.case.value == member.source && case.view == member.view)
                .collect::<Vec<_>>();
            let [observation] = candidates.as_slice() else {
                return Err(invalid(source));
            };
            selected.observation = (*observation).clone();
            selected.sources = vec![selected.observation.clone()];
            let mut moves = Vec::new();
            for id in &selected.moves {
                let witness = tables
                    .source_move_witnesses
                    .get(id.index)
                    .filter(|witness| witness.id == *id)
                    .ok_or_else(|| invalid(source))?;
                if witness.selection.observation == selected.observation
                    || selected.sources.contains(&witness.selection.observation)
                {
                    moves.push(id.clone());
                }
            }
            selected.moves = moves;
        }
        if selected.observation.view != destination.view {
            return Err(invalid(source));
        }
        let case = tables
            .values
            .iter()
            .find(|value| value.id == selected.observation.case.value)
            .and_then(|value| value.cases.get(selected.observation.case.alternative))
            .ok_or_else(|| invalid(source))?;
        if case.availability != destination.availability {
            return Err(invalid(source));
        }
        selected.coordinate = transport.destination.clone();
        ports.push(selected);
    }
    for production in &equation.productions {
        source_case(tables, &production.observation, source)?;
        if production.observation.contributor != rule.id {
            return Err(invalid(source));
        }
        if !rule.productions.iter().any(|port| {
            port.destination == production.observation.port && port.inputs == production.inputs
        }) {
            return Err(invalid(source));
        }
        let mut sources = Vec::new();
        let mut moves = Vec::new();
        let mut complete = true;
        for coordinate in &production.inputs {
            match input
                .ports
                .iter()
                .find(|port| port.coordinate == *coordinate)
            {
                Some(port) => {
                    append_sources(&mut sources, port);
                    for id in &port.moves {
                        if !moves.contains(id) {
                            moves.push(id.clone());
                        }
                    }
                }
                None => complete = false,
            }
        }
        if !complete {
            continue;
        }
        if sources.is_empty() {
            sources.push(production.observation.clone());
        }
        ports.retain(|port| port.coordinate != production.observation.port);
        ports.push(JointSelectedPortFact {
            coordinate: production.observation.port.clone(),
            observation: production.observation.clone(),
            sources,
            moves,
        });
    }
    for id in &equation.moves {
        let witness = tables
            .source_move_witnesses
            .get(id.index)
            .filter(|witness| witness.id == *id)
            .ok_or_else(|| invalid(source))?;
        if witness.event != rule.event
            || witness.input != rule.input
            || witness.output != rule.output
            || witness.contributor != rule.id
            || witness.selection.moves.contains(id)
            || !input.ports.contains(&witness.selection)
            || witness.selection.observation.case.value != witness.source
            || !tables.source_move_inputs.contains(&SourceMoveInput {
                event: witness.event.clone(),
                source: witness.source.clone(),
                place: witness.place.clone(),
            })
        {
            return Err(invalid(source));
        }
        let port = ports
            .iter_mut()
            .find(|port| port.coordinate == witness.selection.coordinate)
            .ok_or_else(|| invalid(source))?;
        let slot = output
            .slots
            .iter()
            .find(|slot| slot.coordinate == port.coordinate)
            .ok_or_else(|| invalid(source))?;
        if slot.availability != ValueAvailability::Available
            || port.observation != witness.selection.observation
        {
            return Err(invalid(source));
        }
        if !port.moves.contains(id) {
            port.moves.push(id.clone());
        }
    }
    ports.sort_by_key(|port| port.coordinate.index);
    let pending = output
        .slots
        .iter()
        .filter(|slot| !ports.iter().any(|port| port.coordinate == slot.coordinate))
        .map(|slot| slot.coordinate.clone())
        .collect();
    Ok(JointPortMonitorFact {
        id: equation.output.clone(),
        contributor: rule.id.clone(),
        ports,
        pending,
    })
}

pub(super) fn transition(
    tables: &mut EvaluatedValueTables,
    contributor: &JointProducerTupleRef,
    input: &JointPortMonitorRef,
    children: Vec<JointSelectedChildFact>,
    productions: Vec<JointPortProductionFact>,
    moves: Vec<SourceMoveWitnessRef>,
    source: &SourceSpan,
) -> Result<JointPortMonitorRef, ReferenceAnalysisError> {
    let mut equation = JointPortMonitorEquation {
        contributor: contributor.clone(),
        input: input.clone(),
        children,
        productions,
        moves,
        output: JointPortMonitorRef {
            owner: contributor.owner.clone(),
            index: 0,
        },
    };
    let output = intern(tables, evaluate(tables, &equation, source)?);
    equation.output = output.clone();
    if !tables.source_port_equations.contains(&equation) {
        tables.source_port_equations.push(equation);
    }
    Ok(output)
}

pub(in crate::scalar) fn query(
    tables: &EvaluatedValueTables,
    monitor: &JointPortMonitorRef,
    requested: &[SourceSlotRef],
    source: &SourceSpan,
) -> Result<Vec<JointSelectedPortFact>, ReferenceAnalysisError> {
    let mut pending = vec![monitor.clone()];
    let mut visited = Vec::new();
    while let Some(id) = pending.pop() {
        if visited.contains(&id) {
            continue;
        }
        let valuation = resolve(tables, &id, source)?;
        let equations = tables
            .source_port_equations
            .iter()
            .filter(|equation| equation.output == id)
            .collect::<Vec<_>>();
        if equations.is_empty() {
            if !matches!(
                resolve_rule(tables, &valuation.contributor, source)?.kind,
                JointProducerRuleKind::Root
            ) || !valuation.ports.is_empty()
            {
                return Err(invalid(source));
            }
            let root = resolve_rule(tables, &valuation.contributor, source)?;
            let expected = tables.source_configurations[root.output.index]
                .slots
                .iter()
                .map(|slot| slot.coordinate.clone())
                .collect::<Vec<_>>();
            if valuation.pending != expected {
                return Err(invalid(source));
            }
        }
        for equation in equations {
            if evaluate(tables, equation, source)? != *valuation {
                return Err(invalid(source));
            }
            pending.push(equation.input.clone());
            pending.extend(equation.children.iter().map(|child| child.monitor.clone()));
        }
        for port in &valuation.ports {
            source_case(tables, &port.observation, source)?;
            for leaf in &port.sources {
                source_case(tables, leaf, source)?;
            }
        }
        visited.push(id);
    }
    let valuation = resolve(tables, monitor, source)?;
    requested
        .iter()
        .map(|coordinate| {
            valuation
                .ports
                .iter()
                .find(|port| port.coordinate == *coordinate)
                .cloned()
                .ok_or_else(|| invalid(source))
        })
        .collect()
}

pub(super) fn selection(
    tables: &EvaluatedValueTables,
    monitor: &JointPortMonitorRef,
    coordinate: &SourceSlotRef,
    source: &SourceSpan,
) -> Result<Option<JointSelectedPortFact>, ReferenceAnalysisError> {
    let valuation = resolve(tables, monitor, source)?;
    if valuation.pending.contains(coordinate) {
        return Ok(None);
    }
    let selections = query(tables, monitor, std::slice::from_ref(coordinate), source)?;
    let [selected] = selections.as_slice() else {
        return Err(invalid(source));
    };
    Ok(Some(selected.clone()))
}

pub(super) fn place_transport(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    place: &ReferencePlaceId,
) -> Result<JointValueTransportKind, ()> {
    if place.projections.is_empty() {
        return Ok(JointValueTransportKind::Copy);
    }
    let [projection] = place.projections.as_slice() else {
        return Err(());
    };
    let slot = selected_slot(
        tables,
        row,
        &SourceCoordinateRole::Binding(place.binding.clone()),
    )?;
    let view = &tables.source_views[slot.view.index];
    let SourcePayloadView::Aggregate(members) = &view.payload else {
        return Err(());
    };
    let positions = members
        .iter()
        .enumerate()
        .filter(
            |(_, member)| match (projection, member.projection.as_slice()) {
                (
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)),
                    [ValueProjection::Field(actual)],
                ) => field == actual,
                (
                    ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }),
                    [ValueProjection::FixedIndex(index)],
                ) => value == &BigInt::from(*index),
                _ => false,
            },
        )
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    let [member] = positions.as_slice() else {
        return Err(());
    };
    Ok(JointValueTransportKind::Project(SourceShapeMemberPort {
        shape: view.shape.clone(),
        member: *member,
    }))
}

pub(super) fn retain_saved(
    tables: &mut EvaluatedValueTables,
    row: &SourceConfigurationFact,
    event: &SourceEventId,
    monitor: &JointPortMonitorRef,
    frozen: &[SelectedSnapshotInput],
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    for saved in row.saved.iter().filter(|saved| saved.snapshot == *event) {
        let Some(selected) = selection(tables, monitor, &saved.coordinate, source)? else {
            continue;
        };
        let instances = owner_consumer::selected_instances(tables, row, &saved.view, source)?;
        let duties = row
            .owns
            .iter()
            .filter(|owns| {
                owns.state == SourceObligationState::Outstanding
                    && instances
                        .iter()
                        .any(|(_, instance)| *instance == owns.payload)
            })
            .cloned()
            .collect();
        let inputs = frozen
            .iter()
            .filter(|input| input.selection == saved.selection)
            .collect::<Vec<_>>();
        let [input] = inputs.as_slice() else {
            return Err(invalid(source));
        };
        let fact = SourceSavedPortFact {
            source_place: input.place.clone(),
            snapshot: event.clone(),
            selection: saved.selection.clone(),
            monitor: monitor.clone(),
            selected,
            duties,
        };
        if !tables.source_saved_ports.contains(&fact) {
            tables.source_saved_ports.push(fact);
        }
    }
    Ok(())
}
