use super::*;

pub(super) fn has_pending(tables: &EvaluatedValueTables, view: &SourceValueViewRef) -> bool {
    match &tables.source_views[view.index].payload {
        SourcePayloadView::Checked { referents, .. } => referents
            .iter()
            .any(|referent| matches!(referent.loan, SourceCheckedGrantState::Pending { .. })),
        SourcePayloadView::Aggregate(members) => members
            .iter()
            .any(|member| has_pending(tables, &member.view)),
        _ => false,
    }
}

pub(super) fn introduce(
    row: &mut SourceConfigurationFact,
    event: &SourceEventId,
    referent: &SourceCheckedReferentView,
    parent: &SourceGrantParent,
) -> Result<SourceGrantClassRef, ()> {
    if let SourceGrantParent::Selected { class, .. } = parent {
        let parents = row
            .grants
            .iter()
            .filter(|grant| grant.class == *class)
            .collect::<Vec<_>>();
        let [parent] = parents.as_slice() else {
            return Err(());
        };
        if parent.referent != referent.instance
            || parent.coordinate != referent.coordinate
            || parent.projection != referent.projection
        {
            return Err(());
        }
    }
    let variable = (0..=row.grants.len())
        .find(|variable| {
            !row.grants.iter().any(|grant| {
                grant.class.owner == event.owner
                    && grant.class.variable == SupportedInstanceVariable(*variable)
            })
        })
        .ok_or(())?;
    let child = SourceGrantClassRef {
        owner: event.owner.clone(),
        variable: SupportedInstanceVariable(variable),
    };
    if let SourceGrantParent::Selected { class: parent, .. } = parent {
        let ancestors = row
            .grants
            .iter()
            .filter(|grant| {
                grant.class == *parent
                    || row
                        .grant_ancestry
                        .iter()
                        .any(|pair| pair.ancestor == grant.class && pair.descendant == *parent)
            })
            .map(|grant| grant.class.clone())
            .collect::<Vec<_>>();
        for ancestor in ancestors {
            row.grant_ancestry.push(SourceGrantAncestryFact {
                ancestor,
                descendant: child.clone(),
            });
        }
    }
    row.grants.push(SourceGrantClassFact {
        class: child.clone(),
        site: event.clone(),
        referent: referent.instance.clone(),
        coordinate: referent.coordinate.clone(),
        projection: referent.projection.clone(),
        mode: referent.mode,
    });
    Ok(child)
}

pub(super) fn borrow(
    tables: &mut EvaluatedValueTables,
    row: &mut SourceConfigurationFact,
    point: &ReferenceFlowPoint,
    event: &SourceEventId,
    monitor: &JointPortMonitorRef,
    value: &EvaluatedValueId,
    shape: &ValueShapeId,
    observation: Option<SourceValueViewRef>,
) -> Result<
    (
        Option<SourceValueViewRef>,
        Option<(SourceGrantClassRef, SourceGrantParent)>,
        Vec<SourceSlotRef>,
    ),
    ReferenceAnalysisError,
> {
    let CfgPointKind::Borrow { request } = &point.kind else {
        return Err(invalid(&point.source_span));
    };
    if let Some(view) = &observation {
        if matches!(
            tables.source_views[view.index].payload,
            SourcePayloadView::Checked {
                state: CheckedValueState::Null { .. } | CheckedValueState::Expired { .. },
                ..
            }
        ) {
            return Ok((observation, None, Vec::new()));
        }
    }
    match request {
        ReferenceBorrowRequest::DirectStorage { place, loan, .. } => {
            let Some(view) = observation else {
                return Ok((None, None, Vec::new()));
            };
            let original = tables.source_views[view.index].clone();
            let SourcePayloadView::Checked {
                state: CheckedValueState::Live,
                referents,
            } = &original.payload
            else {
                return Ok((Some(view), None, Vec::new()));
            };
            let [referent] = referents.as_slice() else {
                return Ok((Some(view), None, Vec::new()));
            };
            let slot = selected_slot(
                tables,
                row,
                &SourceCoordinateRole::Binding(place.binding.clone()),
            )
            .map_err(|_| invalid(&point.source_span))?;
            if referent.coordinate != slot.coordinate
                || referent.mode != loan.mode
                || loan.origin_place != *place
                || place.projections.iter().any(|projection| {
                    !matches!(
                        projection,
                        ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_))
                    )
                })
            {
                return Ok((Some(view), None, Vec::new()));
            }
            let expected = place
                .projections
                .iter()
                .map(|projection| {
                    let ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) =
                        projection
                    else {
                        unreachable!();
                    };
                    ValueProjection::Field(field.clone())
                })
                .collect::<Vec<_>>();
            if referent.projection != expected {
                return Err(invalid(&point.source_span));
            }
            let mut referent = referent.clone();
            referent.grant = point.source_span.clone();
            let parent = SourceGrantParent::Root;
            let child = introduce(row, event, &referent, &parent)
                .map_err(|_| invalid(&point.source_span))?;
            referent.loan = SourceCheckedGrantState::Selected(child.clone());
            let inputs = vec![referent.coordinate.clone()];
            let view = intern_view(
                tables,
                &event.owner,
                shape.clone(),
                SourcePayloadView::Checked {
                    state: CheckedValueState::Live,
                    referents: vec![referent],
                },
                original.dependencies,
            );
            Ok((Some(view), Some((child, parent)), inputs))
        }
        ReferenceBorrowRequest::Through {
            pointer_point,
            place,
            mode,
            ..
        } => {
            let pointers = tables
                .values
                .iter()
                .filter(|value| {
                    value.id.point == *pointer_point && value.id.resolved.producer == event.owner
                })
                .map(|value| value.id.clone())
                .collect::<Vec<_>>();
            let [pointer] = pointers.as_slice() else {
                return Ok((observation, None, Vec::new()));
            };
            let slot = match selected_slot(
                tables,
                row,
                &SourceCoordinateRole::Evaluated(pointer.clone()),
            ) {
                Ok(slot) => slot.clone(),
                Err(()) => return Ok((observation, None, Vec::new())),
            };
            let selections = port_monitor::query(
                tables,
                monitor,
                &[slot.coordinate.clone()],
                &point.source_span,
            )?;
            let [selection] = selections.as_slice() else {
                return Ok((observation, None, vec![slot.coordinate]));
            };
            let original = tables.source_views[selection.observation.view.index].clone();
            let SourcePayloadView::Checked {
                state: CheckedValueState::Live,
                referents,
            } = &original.payload
            else {
                return Ok((observation, None, vec![slot.coordinate]));
            };
            let [referent] = referents.as_slice() else {
                return Ok((observation, None, vec![slot.coordinate]));
            };
            let mut referent = referent.clone();
            referent.mode = *mode;
            referent.grant = point.source_span.clone();
            referent.loan = SourceCheckedGrantState::Pending {
                event: event.clone(),
            };
            let inputs = vec![slot.coordinate];
            if !matches!(place, ScalarPlace::Dereference { .. }) {
                let view = intern_view(
                    tables,
                    &event.owner,
                    shape.clone(),
                    SourcePayloadView::Checked {
                        state: CheckedValueState::Live,
                        referents: vec![referent],
                    },
                    original.dependencies,
                );
                return Ok((Some(view), None, inputs));
            }
            let SourceCheckedGrantState::Selected(parent_class) = &referents[0].loan else {
                let view = intern_view(
                    tables,
                    &event.owner,
                    shape.clone(),
                    SourcePayloadView::Checked {
                        state: CheckedValueState::Live,
                        referents: vec![referent],
                    },
                    original.dependencies,
                );
                return Ok((Some(view), None, inputs));
            };
            validate_view(tables, row, &selection.observation.view, &point.source_span)?;
            if selection.observation.case.value != *pointer
                || selection.observation.view != slot.view
                || value.point != point.id
            {
                return Err(invalid(&point.source_span));
            }
            let parent = SourceGrantParent::Selected {
                class: parent_class.clone(),
                source: pointer.clone(),
                selection: selection.clone(),
            };
            let child = introduce(row, event, &referent, &parent)
                .map_err(|_| invalid(&point.source_span))?;
            referent.loan = SourceCheckedGrantState::Selected(child.clone());
            let view = intern_view(
                tables,
                &event.owner,
                shape.clone(),
                SourcePayloadView::Checked {
                    state: CheckedValueState::Live,
                    referents: vec![referent],
                },
                original.dependencies,
            );
            Ok((Some(view), Some((child, parent)), inputs))
        }
    }
}

pub(super) fn validate_view(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    view: &SourceValueViewRef,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    match &tables.source_views[view.index].payload {
        SourcePayloadView::Checked { referents, .. } => {
            for referent in referents {
                if let SourceCheckedGrantState::Selected(class) = &referent.loan {
                    let grants = row
                        .grants
                        .iter()
                        .filter(|grant| grant.class == *class)
                        .collect::<Vec<_>>();
                    let [grant] = grants.as_slice() else {
                        return Err(invalid(source));
                    };
                    if grant.referent != referent.instance
                        || grant.coordinate != referent.coordinate
                        || grant.projection != referent.projection
                        || grant.mode != referent.mode
                        || !row.instances.contains(&grant.referent)
                    {
                        return Err(invalid(source));
                    }
                }
            }
        }
        SourcePayloadView::Aggregate(members) => {
            for member in members {
                validate_view(tables, row, &member.view, source)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn read(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    selected: &JointSelectedPortFact,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    validate_view(tables, row, &selected.observation.view, source)?;
    let mut pending = vec![selected.observation.view.clone()];
    let mut visited = Vec::new();
    while let Some(view) = pending.pop() {
        if visited.contains(&view) {
            continue;
        }
        visited.push(view.clone());
        match &tables.source_views[view.index].payload {
            SourcePayloadView::Aggregate(members) => {
                pending.extend(members.iter().map(|member| member.view.clone()))
            }
            SourcePayloadView::Checked { referents, .. } => {
                for referent in referents {
                    let SourceCheckedGrantState::Selected(class) = &referent.loan else {
                        continue;
                    };
                    let grants = row
                        .grants
                        .iter()
                        .filter(|grant| grant.class == *class)
                        .collect::<Vec<_>>();
                    let [grant] = grants.as_slice() else {
                        return Err(invalid(source));
                    };
                    let witnesses = tables
                        .source_grant_creations
                        .iter()
                        .filter(|witness| {
                            witness.child == *class
                                && witness.event == grant.site
                                && tables.source_configurations[witness.output.index]
                                    .grants
                                    .contains(grant)
                                && std::iter::once(&selected.observation)
                                    .chain(selected.sources.iter())
                                    .any(|observation| {
                                        observation.case.value.point == witness.event.point
                                            && observation.case.value.resolved.producer
                                                == witness.event.owner
                                            && observation.configuration == witness.output
                                            && observation.contributor == witness.contributor
                                    })
                        })
                        .collect::<Vec<_>>();
                    if witnesses.is_empty() {
                        return Err(invalid(source));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    for row in &tables.source_configurations {
        let source = &cfg.initial_points[0].source_span;
        for grant in &row.grants {
            if row
                .grants
                .iter()
                .filter(|other| other.class == grant.class)
                .count()
                != 1
                || grant.class.owner != row.id.owner
                || !row.instances.contains(&grant.referent)
            {
                return Err(invalid(source));
            }
        }
        for pair in &row.grant_ancestry {
            if pair.ancestor == pair.descendant
                || !row.grants.iter().any(|grant| grant.class == pair.ancestor)
                || !row
                    .grants
                    .iter()
                    .any(|grant| grant.class == pair.descendant)
                || row
                    .grant_ancestry
                    .iter()
                    .filter(|other| *other == pair)
                    .count()
                    != 1
                || row.grant_ancestry.iter().any(|other| {
                    other.ancestor == pair.descendant && other.descendant == pair.ancestor
                })
            {
                return Err(invalid(source));
            }
        }
        for slot in &row.slots {
            validate_view(tables, row, &slot.view, source)?;
        }
    }
    for witness in &tables.source_grant_creations {
        let point = &cfg.initial_points[witness.event.point.0];
        let source = &point.source_span;
        let input = &tables.source_configurations[witness.input.index];
        let output = &tables.source_configurations[witness.output.index];
        let rule = &tables.source_contributors[witness.contributor.index];
        if rule.id != witness.contributor
            || rule.event != witness.event
            || rule.input != witness.input
            || rule.output != witness.output
            || !matches!(point.kind, CfgPointKind::Borrow { .. })
            || input
                .grants
                .iter()
                .any(|grant| grant.class == witness.child)
            || output.grants.len() != input.grants.len() + 1
            || !input
                .grants
                .iter()
                .all(|grant| output.grants.contains(grant))
            || !input
                .grant_ancestry
                .iter()
                .all(|pair| output.grant_ancestry.contains(pair))
        {
            return Err(invalid(source));
        }
        let children = output
            .grants
            .iter()
            .filter(|grant| grant.class == witness.child && grant.site == witness.event)
            .collect::<Vec<_>>();
        let [child] = children.as_slice() else {
            return Err(invalid(source));
        };
        let parent = match &witness.parent {
            SourceGrantParent::Root => {
                let CfgPointKind::Borrow {
                    request: ReferenceBorrowRequest::DirectStorage { place, loan, .. },
                } = &point.kind
                else {
                    return Err(invalid(source));
                };
                let slot = selected_slot(
                    tables,
                    input,
                    &SourceCoordinateRole::Binding(place.binding.clone()),
                )
                .map_err(|_| invalid(source))?;
                if child.coordinate != slot.coordinate
                    || child.mode != loan.mode
                    || loan.origin_place != *place
                {
                    return Err(invalid(source));
                }
                let mut projection = Vec::new();
                for component in &place.projections {
                    let ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) =
                        component
                    else {
                        return Err(invalid(source));
                    };
                    projection.push(ValueProjection::Field(field.clone()));
                }
                if child.projection != projection {
                    return Err(invalid(source));
                }
                None
            }
            SourceGrantParent::Selected {
                class,
                source: pointer,
                selection,
            } => {
                let CfgPointKind::Borrow {
                    request:
                        ReferenceBorrowRequest::Through {
                            pointer_point,
                            mode,
                            place,
                            ..
                        },
                } = &point.kind
                else {
                    return Err(invalid(source));
                };
                let pointer_slot = selected_slot(
                    tables,
                    input,
                    &SourceCoordinateRole::Evaluated(pointer.clone()),
                )
                .map_err(|_| invalid(source))?;
                if pointer.point != *pointer_point
                    || selection.observation.case.value != *pointer
                    || pointer_slot.coordinate != selection.coordinate
                    || pointer_slot.view != selection.observation.view
                    || child.mode != *mode
                    || !matches!(place, ScalarPlace::Dereference { .. })
                {
                    return Err(invalid(source));
                }
                let parents = input
                    .grants
                    .iter()
                    .filter(|grant| grant.class == *class)
                    .collect::<Vec<_>>();
                let [parent] = parents.as_slice() else {
                    return Err(invalid(source));
                };
                if child.referent != parent.referent
                    || child.coordinate != parent.coordinate
                    || child.projection != parent.projection
                {
                    return Err(invalid(source));
                }
                let equations = tables
                    .source_port_equations
                    .iter()
                    .filter(|equation| equation.contributor == witness.contributor)
                    .collect::<Vec<_>>();
                if equations.is_empty() {
                    return Err(invalid(source));
                }
                for equation in equations {
                    let selected = port_monitor::query(
                        tables,
                        &equation.input,
                        &[selection.coordinate.clone()],
                        source,
                    )?;
                    if selected != vec![selection.clone()] {
                        return Err(invalid(source));
                    }
                }
                let SourcePayloadView::Checked { referents, .. } =
                    &tables.source_views[selection.observation.view.index].payload
                else {
                    return Err(invalid(source));
                };
                let [referent] = referents.as_slice() else {
                    return Err(invalid(source));
                };
                if referent.loan != SourceCheckedGrantState::Selected(class.clone()) {
                    return Err(invalid(source));
                }
                Some(class)
            }
        };
        let mut expected = input.grant_ancestry.clone();
        if let Some(parent) = parent {
            for grant in &input.grants {
                if grant.class == *parent
                    || input
                        .grant_ancestry
                        .iter()
                        .any(|pair| pair.ancestor == grant.class && pair.descendant == *parent)
                {
                    expected.push(SourceGrantAncestryFact {
                        ancestor: grant.class.clone(),
                        descendant: witness.child.clone(),
                    });
                }
            }
        }
        if output.grant_ancestry != expected {
            return Err(invalid(source));
        }
    }
    Ok(())
}
