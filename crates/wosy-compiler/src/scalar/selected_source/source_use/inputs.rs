use super::*;

fn consume(
    cfg: &ScalarReferenceCfg,
    rule: &JointProducerTupleFact,
    equation: &JointPortMonitorEquation,
    graph: &mut SourceUseGraph,
    role: SourceCoordinateRole,
    projection: Vec<ValueProjection>,
    purpose: SourceUsePurpose,
    storage: bool,
) -> Result<(), ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    let point = &cfg.initial_points[rule.event.point.0];
    let row = tables
        .source_configurations
        .get(rule.input.index)
        .filter(|row| row.id == rule.input)
        .ok_or_else(|| invalid(&point.source_span))?;
    let node = node_ref(tables, &equation.input, &point.source_span)?;
    let nodes = graph
        .nodes
        .iter()
        .filter(|row| row.id == node && row.configuration == rule.input)
        .collect::<Vec<_>>();
    let [actual] = nodes.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    let slot = match selected_slot(tables, row, &role) {
        Ok(slot) => slot,
        Err(()) => {
            coverage(graph, point, SourceUseCoverageReason::MissingInput(role));
            return Ok(());
        }
    };
    if slot.availability != ValueAvailability::Available {
        coverage(
            graph,
            point,
            SourceUseCoverageReason::Unavailable {
                coordinate: slot.coordinate.clone(),
                state: slot.availability.clone(),
            },
        );
        return Ok(());
    }
    let supports = actual
        .supports
        .iter()
        .filter(|support| {
            support.coordinate == slot.coordinate && support.projection.starts_with(&projection)
        })
        .cloned()
        .collect::<Vec<_>>();
    let cell = if storage {
        current_cell(tables, row, &slot.coordinate).ok()
    } else {
        None
    };
    let selection = port_monitor::selection(
        tables,
        &equation.input,
        &slot.coordinate,
        &point.source_span,
    )?;
    if let Some(selection) = &selection {
        selected_grant::read(tables, row, selection, &point.source_span)?;
        selected_move::read(tables, &cfg.initial_points, selection, &point.source_span)?;
    }
    if !storage && selection.is_none() {
        coverage(
            graph,
            point,
            SourceUseCoverageReason::MissingSelection(slot.coordinate.clone()),
        );
        return Ok(());
    }
    for support in supports {
        let cell_support = matches!((&support.subject, &cell), (SourceUseSubject::Payload { instance, .. }, Some(cell)) if instance == cell);
        let cell_owner = matches!((&support.subject, &cell), (SourceUseSubject::Owner { capability, instance }, Some(cell)) if row.owns.iter().any(|owns| owns.payload == *cell && owns.capability == *capability && owns.owner_instance == *instance && owns.state == SourceObligationState::Outstanding));
        if storage && purpose == SourceUsePurpose::Write && !cell_support && !cell_owner {
            continue;
        }
        let input = if cell_support || cell_owner {
            SourceUseInput::Storage(slot.clone())
        } else if let Some(selection) = &selection {
            SourceUseInput::Selected(selection.clone())
        } else {
            coverage(
                graph,
                point,
                SourceUseCoverageReason::MissingSelection(slot.coordinate.clone()),
            );
            continue;
        };
        let usage = SelectedSourceUse {
            event: rule.event.clone(),
            source_span: point.source_span.clone(),
            contributor: rule.id.clone(),
            equation: equation.clone(),
            configuration: row.id.clone(),
            support,
            input,
            purpose,
        };
        if !graph.uses.contains(&usage) {
            graph.uses.push(usage);
        }
    }
    if storage && cell.is_none() {
        coverage(graph, point, SourceUseCoverageReason::MissingInput(role));
    }
    Ok(())
}

fn place_projection(place: &ReferencePlaceId) -> Result<Vec<ValueProjection>, ()> {
    place
        .projections
        .iter()
        .map(|projection| match projection {
            ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) => {
                Ok(ValueProjection::Field(field.clone()))
            }
            ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }) => {
                u64::try_from(value)
                    .map(ValueProjection::FixedIndex)
                    .map_err(|_| ())
            }
            _ => Err(()),
        })
        .collect()
}

fn storage(
    cfg: &ScalarReferenceCfg,
    rule: &JointProducerTupleFact,
    equation: &JointPortMonitorEquation,
    graph: &mut SourceUseGraph,
    place: &ReferencePlaceId,
    purpose: SourceUsePurpose,
) -> Result<(), ReferenceAnalysisError> {
    let point = &cfg.initial_points[rule.event.point.0];
    if place.source != place.binding.source {
        return Err(invalid(&point.source_span));
    }
    let projection = match place_projection(place) {
        Ok(projection) => projection,
        Err(()) => {
            coverage(
                graph,
                point,
                SourceUseCoverageReason::Projection(place.clone()),
            );
            return Ok(());
        }
    };
    consume(
        cfg,
        rule,
        equation,
        graph,
        SourceCoordinateRole::Binding(place.binding.clone()),
        projection,
        purpose,
        true,
    )
}

fn evaluated(
    cfg: &ScalarReferenceCfg,
    rule: &JointProducerTupleFact,
    equation: &JointPortMonitorEquation,
    graph: &mut SourceUseGraph,
    point: CfgPointId,
    purpose: SourceUsePurpose,
) -> Result<(), ReferenceAnalysisError> {
    let values = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|value| value.id.resolved.producer == cfg.producer_owner && value.id.point == point)
        .collect::<Vec<_>>();
    let [value] = values.as_slice() else {
        coverage(
            graph,
            &cfg.initial_points[rule.event.point.0],
            SourceUseCoverageReason::AssignmentInput,
        );
        return Ok(());
    };
    consume(
        cfg,
        rule,
        equation,
        graph,
        SourceCoordinateRole::Evaluated(value.id.clone()),
        Vec::new(),
        purpose,
        false,
    )
}

pub(super) fn extract(
    cfg: &ScalarReferenceCfg,
    rule: &JointProducerTupleFact,
    equation: &JointPortMonitorEquation,
    graph: &mut SourceUseGraph,
) -> Result<(), ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    let point = &cfg.initial_points[rule.event.point.0];
    if equation.contributor != rule.id
        || rule.id.owner != cfg.producer_owner
        || rule.event.owner != cfg.producer_owner
    {
        return Err(invalid(&point.source_span));
    }
    match &point.kind {
        CfgPointKind::Read { binding } => consume(
            cfg,
            rule,
            equation,
            graph,
            SourceCoordinateRole::Binding(binding.clone()),
            Vec::new(),
            SourceUsePurpose::Read,
            true,
        )?,
        CfgPointKind::ReadPlace {
            pointer_point: Some(pointer),
            ..
        } => evaluated(cfg, rule, equation, graph, *pointer, SourceUsePurpose::Read)?,
        CfgPointKind::ReadPlace {
            pointer_point: None,
            candidates,
            ..
        } => {
            if let [place] = candidates.as_slice() {
                storage(cfg, rule, equation, graph, place, SourceUsePurpose::Read)?;
            } else {
                coverage(
                    graph,
                    point,
                    SourceUseCoverageReason::Operation((&point.kind).into()),
                );
            }
        }
        CfgPointKind::Borrow { request } => match request {
            ReferenceBorrowRequest::DirectStorage { place, .. } => {
                storage(cfg, rule, equation, graph, place, SourceUsePurpose::Borrow)?
            }
            ReferenceBorrowRequest::Through {
                pointer_point,
                place,
                ..
            } => {
                evaluated(
                    cfg,
                    rule,
                    equation,
                    graph,
                    *pointer_point,
                    SourceUsePurpose::Borrow,
                )?;
                if !matches!(place, ScalarPlace::Dereference { .. }) {
                    coverage(
                        graph,
                        point,
                        SourceUseCoverageReason::Operation((&point.kind).into()),
                    );
                }
            }
        },
        CfgPointKind::Move { place, .. } => {
            storage(cfg, rule, equation, graph, place, SourceUsePurpose::Move)?;
            let inputs = tables
                .source_move_inputs
                .iter()
                .filter(|input| input.event == rule.event && input.place == *place)
                .collect::<Vec<_>>();
            if let [input] = inputs.as_slice() {
                consume(
                    cfg,
                    rule,
                    equation,
                    graph,
                    SourceCoordinateRole::Evaluated(input.source.clone()),
                    Vec::new(),
                    SourceUsePurpose::Move,
                    false,
                )?;
            } else {
                coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
            }
        }
        CfgPointKind::Bind { binding } => {
            let inputs = tables
                .binding_inputs
                .iter()
                .filter(|input| input.producer == cfg.producer_owner && input.consumer == point.id)
                .collect::<Vec<_>>();
            if let [input] = inputs.as_slice() {
                let policy = reference::binding_disposition(tables, input, binding)?.policy;
                let purpose = match policy {
                    CopyPolicy::Copy => SourceUsePurpose::Read,
                    CopyPolicy::Move => SourceUsePurpose::Move,
                    CopyPolicy::Deferred => {
                        coverage(graph, point, SourceUseCoverageReason::Operation((&point.kind).into()));
                        return Ok(());
                    }
                };
                consume(
                    cfg,
                    rule,
                    equation,
                    graph,
                    SourceCoordinateRole::Evaluated(input.source.clone()),
                    Vec::new(),
                    purpose,
                    false,
                )?;
            } else if tables
                .source_binding_shapes
                .iter()
                .any(|shape| shape.binding == *binding)
            {
                coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
            }
        }
        CfgPointKind::Assign {
            target, rhs_point, ..
        } => {
            let commits = tables
                .commits
                .iter()
                .filter(|commit| {
                    commit.destination.operation.producer == cfg.producer_owner
                        && commit.replacement.resolved.producer == cfg.producer_owner
                        && commit.point == point.id
                        && commit.replacement.point == *rhs_point
                })
                .collect::<Vec<_>>();
            if let [commit] = commits.as_slice() {
                let destinations = tables
                    .destinations
                    .iter()
                    .filter(|destination| destination.id == commit.destination)
                    .collect::<Vec<_>>();
                let [destination] = destinations.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                let corresponds = match &destination.storage {
                    EvaluatedDestinationStorage::Direct(place) => *place == *target,
                    EvaluatedDestinationStorage::Inferred(binding) => {
                        *binding == target.binding && target.projections.is_empty()
                    }
                    EvaluatedDestinationStorage::Through { .. } => false,
                };
                if !corresponds {
                    return Err(invalid(&point.source_span));
                }
                consume(
                    cfg,
                    rule,
                    equation,
                    graph,
                    SourceCoordinateRole::Evaluated(commit.replacement.clone()),
                    Vec::new(),
                    SourceUsePurpose::Read,
                    false,
                )?;
                storage(cfg, rule, equation, graph, target, SourceUsePurpose::Write)?;
            } else {
                coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
            }
        }
        CfgPointKind::AssignThrough {
            pointer_point,
            rhs_point,
            ..
        } => {
            evaluated(
                cfg,
                rule,
                equation,
                graph,
                *rhs_point,
                SourceUsePurpose::Read,
            )?;
            evaluated(
                cfg,
                rule,
                equation,
                graph,
                *pointer_point,
                SourceUsePurpose::Write,
            )?;
            coverage(
                graph,
                point,
                SourceUseCoverageReason::Operation((&point.kind).into()),
            );
        }
        CfgPointKind::AssignmentSnapshot {
            destination_point, ..
        } => {
            let destinations = tables
                .destinations
                .iter()
                .filter(|destination| {
                    destination.id.operation.producer == cfg.producer_owner
                        && destination.point == *destination_point
                })
                .collect::<Vec<_>>();
            let snapshots = tables
                .snapshots
                .iter()
                .filter(|snapshot| {
                    snapshot.destination.operation.producer == cfg.producer_owner
                        && snapshot.point == point.id
                })
                .collect::<Vec<_>>();
            if let ([destination], [snapshot]) = (destinations.as_slice(), snapshots.as_slice()) {
                if snapshot.destination != destination.id {
                    return Err(invalid(&point.source_span));
                }
                if let SnapshotState::Saved(value) = &snapshot.state {
                    if value.resolved.producer != cfg.producer_owner || value.point != point.id {
                        return Err(invalid(&point.source_span));
                    }
                }
                match &destination.storage {
                    EvaluatedDestinationStorage::Direct(place) => {
                        storage(cfg, rule, equation, graph, place, SourceUsePurpose::Read)?
                    }
                    EvaluatedDestinationStorage::Through { pointer, .. } => {
                        consume(
                            cfg,
                            rule,
                            equation,
                            graph,
                            SourceCoordinateRole::Evaluated(pointer.clone()),
                            Vec::new(),
                            SourceUsePurpose::Read,
                            false,
                        )?;
                        coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
                    }
                    EvaluatedDestinationStorage::Inferred(_) => {
                        coverage(graph, point, SourceUseCoverageReason::AssignmentInput)
                    }
                }
            } else {
                coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
            }
        }
        CfgPointKind::AssignmentResult {
            snapshot_point,
            output,
        } => {
            let results = tables
                .results
                .iter()
                .filter(|result| {
                    result.operation.producer == cfg.producer_owner
                        && result.value.point == point.id
                        && result.value.resolved.producer == cfg.producer_owner
                        && result.snapshot == *snapshot_point
                        && result.output == *output
                })
                .collect::<Vec<_>>();
            let row = &tables.source_configurations[rule.input.index];
            let saved = row
                .saved
                .iter()
                .filter(|saved| {
                    saved.snapshot.owner == cfg.producer_owner
                        && saved.snapshot.point == *snapshot_point
                })
                .collect::<Vec<_>>();
            if let ([result], [saved]) = (results.as_slice(), saved.as_slice()) {
                if saved.selection.destination.operation != result.operation
                    || saved.selection.application.operation != result.operation
                {
                    return Err(invalid(&point.source_span));
                }
                consume(
                    cfg,
                    rule,
                    equation,
                    graph,
                    SourceCoordinateRole::Saved(saved.selection.clone()),
                    Vec::new(),
                    SourceUsePurpose::Read,
                    false,
                )?;
            } else {
                coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
            }
        }
        CfgPointKind::ReturnOutput {
            output,
            value_point,
            ..
        } => {
            let inputs = tables.final_inputs.iter().filter(|input| input.consumer.producer == cfg.producer_owner && input.consumer.point == point.id && matches!(input.consumer.kind, ValueConsumerKind::FinalOutput { output: ordinal, .. } if ordinal == *output)).collect::<Vec<_>>();
            let [input] = inputs.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            if input.source.point != *value_point {
                return Err(invalid(&point.source_span));
            }
            consume(
                cfg,
                rule,
                equation,
                graph,
                SourceCoordinateRole::Evaluated(input.source.clone()),
                Vec::new(),
                SourceUsePurpose::Return,
                false,
            )?;
        }
        CfgPointKind::AssignmentDestination {
            storage: EvaluatedDestinationStorage::Through { pointer, .. },
            ..
        } => consume(
            cfg,
            rule,
            equation,
            graph,
            SourceCoordinateRole::Evaluated(pointer.clone()),
            Vec::new(),
            SourceUsePurpose::Read,
            false,
        )?,
        CfgPointKind::Entry
        | CfgPointKind::ValueOutputProjection { .. }
        | CfgPointKind::ScopeEnter
        | CfgPointKind::Join
        | CfgPointKind::Return
        | CfgPointKind::AssignmentDispose { .. }
        | CfgPointKind::AssignmentDestination { .. } => {}
        CfgPointKind::Evaluate(expression) => {
            if matches!(
                expression,
                ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }
            ) {
                let literals = tables
                    .source_literal_members
                    .iter()
                    .filter(|literal| {
                        literal.value.resolved.producer == cfg.producer_owner
                            && literal.value.point == point.id
                    })
                    .collect::<Vec<_>>();
                if let [literal] = literals.as_slice() {
                    for member in &literal.members {
                        consume(
                            cfg,
                            rule,
                            equation,
                            graph,
                            SourceCoordinateRole::Evaluated(member.value.clone()),
                            Vec::new(),
                            SourceUsePurpose::Read,
                            false,
                        )?;
                    }
                } else {
                    coverage(graph, point, SourceUseCoverageReason::AssignmentInput);
                }
            } else if !matches!(
                expression,
                ScalarExpression::Integer { .. }
                    | ScalarExpression::Boolean { .. }
                    | ScalarExpression::Float { .. }
                    | ScalarExpression::Char { .. }
            ) {
                coverage(
                    graph,
                    point,
                    SourceUseCoverageReason::Operation((&point.kind).into()),
                );
            }
        }
        _ => coverage(
            graph,
            point,
            SourceUseCoverageReason::Operation((&point.kind).into()),
        ),
    }
    Ok(())
}
