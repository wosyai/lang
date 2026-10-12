use super::*;

fn reaches(points: &[ReferenceFlowPoint], from: CfgPointId, to: CfgPointId) -> bool {
    let mut pending = vec![from];
    let mut visited = BTreeSet::new();
    while let Some(point) = pending.pop() {
        if point == to {
            return true;
        }
        if visited.insert(point) {
            pending.extend(&points[point.0].successors);
        }
    }
    false
}

pub(super) fn selected_instances(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    view: &SourceValueViewRef,
    source: &SourceSpan,
) -> Result<Vec<(Vec<ValueProjection>, SourceInstanceRef)>, ReferenceAnalysisError> {
    let mut pending = vec![(Vec::new(), view.clone())];
    let mut visited = Vec::new();
    let mut instances = Vec::new();
    while let Some((projection, view)) = pending.pop() {
        if visited.contains(&(projection.clone(), view.clone())) {
            continue;
        }
        visited.push((projection.clone(), view.clone()));
        let resolved = tables
            .source_views
            .get(view.index)
            .filter(|resolved| resolved.id == view)
            .ok_or_else(|| invalid(source))?;
        match &resolved.payload {
            SourcePayloadView::Raw { referent, .. } => {
                instances.push((projection, referent.clone()))
            }
            SourcePayloadView::Backing { instance, .. } => {
                instances.push((projection, instance.clone()))
            }
            SourcePayloadView::Aggregate(members) => {
                for member in members {
                    let mut child = projection.clone();
                    child.extend(member.projection.clone());
                    pending.push((child, member.view.clone()));
                }
            }
            SourcePayloadView::Checked { referents, .. } => {
                for referent in referents {
                    instances.push((projection.clone(), referent.instance.clone()));
                    if let Some(slot) = row
                        .slots
                        .iter()
                        .find(|slot| slot.coordinate == referent.coordinate)
                    {
                        pending.push((projection.clone(), slot.view.clone()));
                    }
                }
            }
            SourcePayloadView::Formal(_)
            | SourcePayloadView::Plain
            | SourcePayloadView::Unwritten
            | SourcePayloadView::StaticRaw(_) => {}
        }
    }
    Ok(instances)
}

pub(super) fn dependency_instance_matches(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    dependency: &EvaluatedDependencyFact,
    instance: &SourceInstanceRef,
) -> bool {
    match &dependency.source {
        DependencySource::Allocation(family) => row.created.iter().any(|created| {
            created.instance == *instance
                && created.event.owner == family.producer
                && created.event.point == family.creation
                && created.role == SourceObjectRole::Payload
        }),
        DependencySource::BindingCell(binding) => match instance {
            SourceInstanceRef::Supported(id) => tables
                .source_instances
                .iter()
                .find(|class| class.id == *id)
                .is_some_and(|class| {
                    class.role == SourceObjectRole::BindingCell
                        && tables.source_slots.iter().any(|slot| {
                            slot.id == class.support
                                && slot.role == SourceCoordinateRole::Binding(binding.clone())
                        })
                }),
            SourceInstanceRef::Residual(_) => false,
        },
        _ => false,
    }
}

pub(super) fn construct_return(
    tables: &mut EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    event: &SourceEventId,
    row: &SourceConfigurationFact,
    contributor: &JointProducerTupleRef,
    monitor: &JointPortMonitorRef,
) -> Result<(), ReferenceAnalysisError> {
    let point = &points[event.point.0];
    common_loan::require_epoch(tables, &event.owner, points, event)?;
    let inputs = tables
        .final_inputs
        .iter()
        .filter(|input| {
            input.consumer.point == event.point && input.consumer.producer == event.owner
        })
        .cloned()
        .collect::<Vec<_>>();
    let [input] = inputs.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    let selected = match selected_slot(
        tables,
        row,
        &SourceCoordinateRole::Evaluated(input.source.clone()),
    ) {
        Ok(selected) => selected.clone(),
        Err(()) => return Ok(()),
    };
    let receiving = tables
        .owner_capabilities
        .iter()
        .filter(|owner| owner.role == OwnerRole::Result(input.consumer.clone()))
        .collect::<Vec<_>>();
    let [receiving] = receiving.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    let receiving = receiving.id.clone();
    current_owner(tables, row, &receiving).map_err(|_| invalid(&point.source_span))?;
    if let Some(membership) =
        port_monitor::selection(tables, monitor, &selected.coordinate, &point.source_span)?
    {
        let selection = membership.observation;
        if selection.case.value != input.source || selection.view != selected.view {
            return Err(invalid(&point.source_span));
        }
        let value = tables
            .values
            .iter()
            .find(|value| value.id == selection.case.value)
            .ok_or_else(|| invalid(&point.source_span))?;
        let case = value
            .cases
            .get(selection.case.alternative)
            .filter(|case| case.id == selection.case)
            .ok_or_else(|| invalid(&point.source_span))?
            .clone();
        let transitions = tables
            .owner_receiving_transitions
            .iter()
            .filter(|transition| {
                transition.event == *event
                    && transition.output == row.id
                    && transition.receiving == receiving
                    && transition.dependencies.iter().all(|dependency| {
                        dependency.case == case.id && dependency.path == case.path
                    })
            })
            .collect::<Vec<_>>();
        let [transition] = transitions.as_slice() else {
            return Ok(());
        };
        let Some(action) = owner_receiving::disposition(
            tables,
            &input.source,
            &input.consumer,
            &point.source_span,
        )?
        else {
            return Ok(());
        };
        let candidate = EvaluatedOwnerConsumer {
            consumer: input.consumer.clone(),
            input: case.id.clone(),
            path: case.path.clone(),
            receiving: receiving.clone(),
            contributor: contributor.clone(),
            dependencies: transition.dependencies.clone(),
            disposition: ValueDisposition {
                value: input.source.clone(),
                case: case.id.clone(),
                point: event.point,
                action,
            },
        };
        validate_monitored_correspondence(tables, points, row, &candidate, monitor)?;
        let mut requirements = vec![
            OwnerConsumerRequirement::CompleteSourcePredicateProduct(row.id.clone()),
            OwnerConsumerRequirement::FiniteSupportNormalization(row.id.clone()),
            OwnerConsumerRequirement::RemainingDependencyUses(transition.input.clone()),
            OwnerConsumerRequirement::StagedLoanValidation(event.clone()),
        ];
        if matches!(
            candidate.disposition.action,
            ValueDispositionAction::MoveTo(_)
        ) {
            requirements.push(OwnerConsumerRequirement::SourceMoveWitness(
                input.source.clone(),
            ));
        }
        if let SourcePayloadView::Checked { referents, .. } =
            &tables.source_views[selected.view.index].payload
        {
            for referent in referents {
                requirements.push(OwnerConsumerRequirement::FixedReturnMaterialization(
                    referent.instance.clone(),
                ));
            }
        }
        let pending = PendingOwnerConsumer {
            candidate,
            requirements,
            monitor: monitor.clone(),
        };
        if !tables.pending_owner_consumers.contains(&pending) {
            tables.pending_owner_consumers.push(pending);
        }
    }
    Ok(())
}

pub(super) fn validate_correspondence(
    tables: &EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    row: &SourceConfigurationFact,
    consumer: &EvaluatedOwnerConsumer,
) -> Result<(), ReferenceAnalysisError> {
    let monitors = tables
        .pending_owner_consumers
        .iter()
        .filter(|pending| {
            pending.candidate.input == consumer.input
                && pending.candidate.consumer == consumer.consumer
                && pending.candidate.contributor == consumer.contributor
        })
        .map(|pending| pending.monitor.clone())
        .collect::<Vec<_>>();
    if monitors.is_empty() {
        return Err(invalid(&points[consumer.consumer.point.0].source_span));
    }
    for monitor in monitors {
        validate_monitored_correspondence(tables, points, row, consumer, &monitor)?;
    }
    Ok(())
}

pub(super) fn validate_monitored_correspondence(
    tables: &EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    row: &SourceConfigurationFact,
    consumer: &EvaluatedOwnerConsumer,
    monitor: &JointPortMonitorRef,
) -> Result<(), ReferenceAnalysisError> {
    let point = points
        .get(consumer.consumer.point.0)
        .ok_or_else(|| invalid(&points[0].source_span))?;
    common_loan::require_epoch(
        tables,
        &consumer.consumer.producer,
        points,
        &SourceEventId {
            owner: consumer.consumer.producer.clone(),
            point: point.id,
        },
    )?;
    let receiver = tables
        .owner_capabilities
        .iter()
        .find(|owner| owner.id == consumer.receiving)
        .ok_or_else(|| invalid(&point.source_span))?;
    let tuple = tables
        .source_contributors
        .get(consumer.contributor.index)
        .filter(|tuple| tuple.id == consumer.contributor)
        .ok_or_else(|| invalid(&point.source_span))?;
    let value = tables
        .values
        .iter()
        .find(|value| value.id == consumer.input.value)
        .ok_or_else(|| invalid(&point.source_span))?;
    let case = value
        .cases
        .get(consumer.input.alternative)
        .filter(|case| case.id == consumer.input)
        .ok_or_else(|| invalid(&point.source_span))?;
    let action = owner_receiving::disposition(
        tables,
        &consumer.input.value,
        &consumer.consumer,
        &point.source_span,
    )?
    .ok_or_else(|| invalid(&point.source_span))?;
    if consumer.disposition.action != action {
        return Err(invalid(&point.source_span));
    }
    let transitions = tables
        .owner_receiving_transitions
        .iter()
        .filter(|transition| {
            transition.event == tuple.event
                && transition.input == tuple.input
                && transition.output == row.id
                && transition.receiving == consumer.receiving
                && transition.dependencies == consumer.dependencies
        })
        .collect::<Vec<_>>();
    let [transition] = transitions.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    let donor = tables
        .source_configurations
        .get(transition.input.index)
        .filter(|input| input.id == transition.input)
        .ok_or_else(|| invalid(&point.source_span))?;
    if transition.endpoints != receiver.endpoints
        || transition.receiving_instance
            != current_owner(tables, row, &consumer.receiving)
                .map_err(|_| invalid(&point.source_span))?
    {
        return Err(invalid(&point.source_span));
    }
    if tables.source_configurations.get(row.id.index) != Some(row)
        || case.path != consumer.path
        || tuple.output != row.id
        || tuple.event.point != point.id
        || receiver.introduction.point != point.id
        || receiver.role != OwnerRole::Result(consumer.consumer.clone())
        || !tables.final_inputs.iter().any(|input| {
            input.consumer == consumer.consumer && input.source == consumer.input.value
        })
        || consumer.disposition.case != case.id
        || consumer.disposition.point != point.id
        || case.availability != ValueAvailability::Available
        || consumer.dependencies.len() != case.dependencies.len()
    {
        return Err(invalid(&point.source_span));
    }
    if consumer.consumer.producer != row.id.owner
        || tuple.event.owner != row.id.owner
        || consumer.disposition.value != consumer.input.value
        || !matches!(point.kind, CfgPointKind::ReturnOutput { .. })
    {
        return Err(invalid(&point.source_span));
    }
    current_owner(tables, row, &consumer.receiving).map_err(|_| invalid(&point.source_span))?;
    let selected = selected_slot(
        tables,
        row,
        &SourceCoordinateRole::Evaluated(consumer.input.value.clone()),
    )
    .map_err(|_| invalid(&point.source_span))?;
    let valuation = tables
        .source_port_monitors
        .get(monitor.index)
        .filter(|valuation| {
            valuation.id == *monitor && valuation.contributor == consumer.contributor
        })
        .ok_or_else(|| invalid(&point.source_span))?;
    let memberships = port_monitor::query(
        tables,
        &valuation.id,
        std::slice::from_ref(&selected.coordinate),
        &point.source_span,
    )?;
    let [membership] = memberships.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    selected_move::read(tables, points, membership, &point.source_span)?;
    selected_grant::read(tables, row, membership, &point.source_span)?;
    if selected.availability != case.availability
        || membership.observation.case != case.id
        || membership.observation.path != case.path
        || membership.observation.view != selected.view
    {
        return Err(invalid(&point.source_span));
    }
    let instances = selected_instances(tables, row, &selected.view, &point.source_span)?;
    for (selection, source) in consumer.dependencies.iter().zip(&case.dependencies) {
        if !matches!(
            (&source.action, &selection.dependency.action),
            (
                DependencyAction::TransferUnique { .. },
                DependencyAction::TransferUnique { .. }
            ) | (
                DependencyAction::BorrowFixed { .. },
                DependencyAction::BorrowFixed { .. }
            )
        ) {
            return Err(invalid(&point.source_span));
        }
        if selection.dependency.source != source.source
            || selection.dependency.witness != source.witness
            || selection.dependency.projection != source.projection
        {
            return Err(invalid(&point.source_span));
        }
        if selection.case != case.id
            || selection.path != case.path
            || selection.projection != selection.dependency.projection
            || !row.instances.contains(&selection.source_instance)
            || !row.instances.contains(&selection.owner_instance)
        {
            return Err(invalid(&point.source_span));
        }
        if !instances.contains(&(
            selection.projection.clone(),
            selection.source_instance.clone(),
        )) || !dependency_instance_matches(tables, row, source, &selection.source_instance)
        {
            return Err(invalid(&point.source_span));
        }
        let source_owner = match &selection.dependency.action {
            DependencyAction::TransferUnique {
                source_owner,
                receiving_owner,
            }
            | DependencyAction::PromoteShared {
                source_owner,
                receiving_owner,
            } if receiving_owner == &consumer.receiving => source_owner,
            DependencyAction::BorrowFixed {
                referent_lifetime: FixedLifetime::Capability(source_owner),
                use_owner,
            } if use_owner == &consumer.receiving => source_owner,
            _ => return Err(invalid(&point.source_span)),
        };
        let capability = tables
            .owner_capabilities
            .iter()
            .find(|owner| &owner.id == source_owner)
            .ok_or_else(|| invalid(&point.source_span))?;
        if current_owner(tables, donor, source_owner).map_err(|_| invalid(&point.source_span))?
            != selection.owner_instance
            || capability.endpoints.is_empty()
        {
            return Err(invalid(&point.source_span));
        }
        if !reaches(points, capability.introduction.point, point.id)
            || !capability.endpoints.iter().all(|endpoint| {
                donor.owns.iter().any(|owns| {
                    owns.capability == *source_owner
                        && owns.owner_instance == selection.owner_instance
                        && owns.payload == selection.source_instance
                        && owns.endpoint == *endpoint
                        && owns.state == SourceObligationState::Outstanding
                })
            })
        {
            return Err(invalid(&point.source_span));
        }
        match &selection.dependency.action {
            DependencyAction::TransferUnique { .. } => {
                let donor_rows = donor
                    .owns
                    .iter()
                    .filter(|owns| {
                        owns.payload == selection.source_instance
                            && owns.capability == *source_owner
                            && owns.owner_instance == selection.owner_instance
                            && owns.state == SourceObligationState::Outstanding
                    })
                    .collect::<Vec<_>>();
                let receiving_rows = row
                    .owns
                    .iter()
                    .filter(|owns| {
                        owns.payload == selection.source_instance
                            && owns.state == SourceObligationState::Outstanding
                    })
                    .collect::<Vec<_>>();
                if donor_rows.len() != capability.endpoints.len()
                    || donor_rows
                        .iter()
                        .any(|owns| owns.kind != RuntimeBackingKind::OwnedAllocation)
                    || receiving_rows.len() != transition.endpoints.len()
                    || receiving_rows
                        .iter()
                        .any(|owns| owns.kind != RuntimeBackingKind::OwnedAllocation)
                {
                    return Err(invalid(&point.source_span));
                }
                if !capability.endpoints.iter().all(|endpoint| {
                    row.owns.iter().any(|owns| {
                        owns.capability == *source_owner
                            && owns.owner_instance == selection.owner_instance
                            && owns.payload == selection.source_instance
                            && owns.endpoint == *endpoint
                            && owns.state == SourceObligationState::Transferred
                    })
                }) || !transition.endpoints.iter().all(|endpoint| {
                    row.owns.iter().any(|owns| {
                        owns.capability == consumer.receiving
                            && owns.owner_instance == transition.receiving_instance
                            && owns.payload == selection.source_instance
                            && owns.endpoint == *endpoint
                            && owns.state == SourceObligationState::Outstanding
                    })
                }) {
                    return Err(invalid(&point.source_span));
                }
                if row.owns.iter().any(|owns| {
                    owns.payload == selection.source_instance
                        && owns.state == SourceObligationState::Outstanding
                        && (owns.capability != consumer.receiving
                            || owns.owner_instance != transition.receiving_instance)
                }) {
                    return Err(invalid(&point.source_span));
                }
            }
            DependencyAction::BorrowFixed { .. } => {
                if !capability.endpoints.iter().all(|endpoint| {
                    row.owns.iter().any(|owns| {
                        owns.capability == *source_owner
                            && owns.owner_instance == selection.owner_instance
                            && owns.payload == selection.source_instance
                            && owns.endpoint == *endpoint
                            && owns.state == SourceObligationState::Outstanding
                    })
                }) {
                    return Err(invalid(&point.source_span));
                }
            }
            _ => return Err(invalid(&point.source_span)),
        }
    }
    let mut rebuilt = row.clone();
    owner_receiving::refresh_views(tables, &mut rebuilt, &point.source_span)?;
    if rebuilt.owner_views != row.owner_views {
        return Err(invalid(&point.source_span));
    }
    Ok(())
}
