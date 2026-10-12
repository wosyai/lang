use super::*;

pub(super) fn refresh_views(
    tables: &EvaluatedValueTables,
    row: &mut SourceConfigurationFact,
    source: &SourceSpan,
) -> Result<(), ReferenceAnalysisError> {
    let mut views = Vec::new();
    for slot in &row.slots {
        for (projection, payload) in
            owner_consumer::selected_instances(tables, row, &slot.view, source)?
        {
            for owns in row.owns.iter().filter(|owns| {
                owns.payload == payload && owns.state == SourceObligationState::Outstanding
            }) {
                let fact = SourceOwnerViewFact {
                    coordinate: slot.coordinate.clone(),
                    projection: projection.clone(),
                    payload: payload.clone(),
                    capability: owns.capability.clone(),
                    owner_instance: owns.owner_instance.clone(),
                };
                if !views.contains(&fact) {
                    views.push(fact);
                }
            }
        }
    }
    row.owner_views = views;
    Ok(())
}

pub(super) fn disposition(
    tables: &EvaluatedValueTables,
    source: &EvaluatedValueId,
    consumer: &ValueConsumerId,
    span: &SourceSpan,
) -> Result<Option<ValueDispositionAction>, ReferenceAnalysisError> {
    let inputs = tables
        .disposition_inputs
        .iter()
        .filter(|input| input.source == *source && input.consumer == *consumer)
        .collect::<Vec<_>>();
    let [input] = inputs.as_slice() else {
        return Err(invalid(span));
    };
    Ok(match input.policy {
        CopyPolicy::Copy => Some(ValueDispositionAction::CopyTo(consumer.clone())),
        CopyPolicy::Move => Some(ValueDispositionAction::MoveTo(consumer.clone())),
        CopyPolicy::Deferred => None,
    })
}

fn pending(
    tables: &mut EvaluatedValueTables,
    event: &SourceEventId,
    input: &SourceConfigurationFact,
    source: &EvaluatedValueId,
    reason: OwnerReceivingPendingReason,
) {
    let fact = OwnerReceivingPendingFact {
        event: event.clone(),
        input: input.id.clone(),
        source: source.clone(),
        reason,
    };
    if !tables.pending_owner_receiving.contains(&fact) {
        tables.pending_owner_receiving.push(fact);
    }
}

pub(super) fn prepare(
    tables: &mut EvaluatedValueTables,
    points: &[ReferenceFlowPoint],
    event: &SourceEventId,
    input: &SourceConfigurationFact,
    output: &mut SourceConfigurationFact,
    monitor: &JointPortMonitorRef,
) -> Result<Option<OwnerReceivingTransitionFact>, ReferenceAnalysisError> {
    let span = &points[event.point.0].source_span;
    let inputs = tables
        .final_inputs
        .iter()
        .filter(|port| port.consumer.producer == event.owner && port.consumer.point == event.point)
        .collect::<Vec<_>>();
    let [port] = inputs.as_slice() else {
        return Err(invalid(span));
    };
    let port = (*port).clone();
    if disposition(tables, &port.source, &port.consumer, span)?.is_none() {
        pending(
            tables,
            event,
            input,
            &port.source,
            OwnerReceivingPendingReason::DeferredPolicy,
        );
        return Ok(None);
    }
    let selected = match selected_slot(
        tables,
        input,
        &SourceCoordinateRole::Evaluated(port.source.clone()),
    ) {
        Ok(selected) => selected,
        Err(()) => {
            pending(
                tables,
                event,
                input,
                &port.source,
                OwnerReceivingPendingReason::SelectedSource,
            );
            return Ok(None);
        }
    };
    let Some(membership) = port_monitor::selection(tables, monitor, &selected.coordinate, span)?
    else {
        pending(
            tables,
            event,
            input,
            &port.source,
            OwnerReceivingPendingReason::SelectedSource,
        );
        return Ok(None);
    };
    if membership.observation.case.value != port.source
        || membership.observation.view != selected.view
    {
        return Err(invalid(span));
    }
    let case = tables
        .values
        .iter()
        .find(|value| value.id == port.source)
        .and_then(|value| value.cases.get(membership.observation.case.alternative))
        .filter(|case| case.id == membership.observation.case)
        .ok_or_else(|| invalid(span))?
        .clone();
    if case.availability != ValueAvailability::Available
        || selected.availability != case.availability
    {
        return Err(invalid(span));
    }
    let receivers = tables
        .owner_capabilities
        .iter()
        .filter(|owner| {
            owner.role == OwnerRole::Result(port.consumer.clone()) && owner.introduction == *event
        })
        .cloned()
        .collect::<Vec<_>>();
    let [receiver] = receivers.as_slice() else {
        return Err(invalid(span));
    };
    let receiving_instance =
        current_owner(tables, output, &receiver.id).map_err(|_| invalid(span))?;
    if receiver.endpoints.is_empty()
        || receiver.endpoints.iter().any(|endpoint| {
            endpoint.event != *event
                || endpoint.frame != output.frame
                || endpoint.kind
                    != (OwnerEndpointKind::ResultHandoff {
                        consumer: port.consumer.clone(),
                    })
        })
    {
        return Err(invalid(span));
    }
    let instances = owner_consumer::selected_instances(tables, input, &selected.view, span)?;
    let mut dependencies = Vec::new();
    for dependency in &case.dependencies {
        if !matches!(
            dependency.action,
            DependencyAction::TransferUnique { .. } | DependencyAction::BorrowFixed { .. }
        ) {
            pending(
                tables,
                event,
                input,
                &port.source,
                OwnerReceivingPendingReason::DependencyAction {
                    case: case.id.clone(),
                    dependency: dependency.clone(),
                },
            );
            return Ok(None);
        }
        let mut payloads = Vec::new();
        for (projection, instance) in &instances {
            if projection == &dependency.projection
                && owner_consumer::dependency_instance_matches(tables, input, dependency, instance)
                && !payloads.contains(instance)
            {
                payloads.push(instance.clone());
            }
        }
        let [payload] = payloads.as_slice() else {
            return Err(invalid(span));
        };
        let obligations = input
            .owns
            .iter()
            .filter(|owns| {
                owns.payload == *payload && owns.state == SourceObligationState::Outstanding
            })
            .collect::<Vec<_>>();
        let mut owners = Vec::new();
        for owns in &obligations {
            let owner = (
                owns.capability.clone(),
                owns.owner_instance.clone(),
                owns.kind.clone(),
            );
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
        let [(source_owner, source_instance, kind)] = owners.as_slice() else {
            return Err(invalid(span));
        };
        let capability = tables
            .owner_capabilities
            .iter()
            .find(|owner| owner.id == *source_owner)
            .ok_or_else(|| invalid(span))?;
        if current_owner(tables, input, source_owner).map_err(|_| invalid(span))?
            != *source_instance
            || capability.endpoints.is_empty()
            || capability.endpoints.len() != obligations.len()
            || capability.endpoints.iter().any(|endpoint| {
                !obligations.iter().any(|owns| owns.endpoint == *endpoint)
                    || endpoint.event == *event
            })
        {
            return Err(invalid(span));
        }
        let mut dependency = dependency.clone();
        dependency.action = match &dependency.action {
            DependencyAction::TransferUnique { .. }
                if *kind == RuntimeBackingKind::OwnedAllocation =>
            {
                DependencyAction::TransferUnique {
                    source_owner: source_owner.clone(),
                    receiving_owner: receiver.id.clone(),
                }
            }
            DependencyAction::BorrowFixed { .. } if *kind == RuntimeBackingKind::FixedValue => {
                DependencyAction::BorrowFixed {
                    referent_lifetime: FixedLifetime::Capability(source_owner.clone()),
                    use_owner: receiver.id.clone(),
                }
            }
            _ => {
                pending(
                    tables,
                    event,
                    input,
                    &port.source,
                    OwnerReceivingPendingReason::DependencyAction {
                        case: case.id.clone(),
                        dependency,
                    },
                );
                return Ok(None);
            }
        };
        dependencies.push(OwnerDependencySelection {
            case: case.id.clone(),
            projection: dependency.projection.clone(),
            path: case.path.clone(),
            source_instance: payload.clone(),
            owner_instance: source_instance.clone(),
            dependency,
        });
    }
    let transition = OwnerReceivingTransitionFact {
        event: event.clone(),
        input: input.id.clone(),
        output: output.id.clone(),
        receiving: receiver.id.clone(),
        receiving_instance: receiving_instance.clone(),
        dependencies,
        endpoints: receiver.endpoints.clone(),
    };
    let mut transferred = Vec::new();
    for dependency in &transition.dependencies {
        let DependencyAction::TransferUnique { source_owner, .. } = &dependency.dependency.action
        else {
            continue;
        };
        if transferred.contains(&dependency.source_instance) {
            continue;
        }
        for owns in output.owns.iter_mut().filter(|owns| {
            owns.payload == dependency.source_instance
                && owns.capability == *source_owner
                && owns.owner_instance == dependency.owner_instance
                && owns.state == SourceObligationState::Outstanding
        }) {
            owns.state = SourceObligationState::Transferred;
        }
        for endpoint in &transition.endpoints {
            output.owns.push(SourceOwnsFact {
                capability: receiver.id.clone(),
                owner_instance: receiving_instance.clone(),
                payload: dependency.source_instance.clone(),
                endpoint: endpoint.clone(),
                kind: RuntimeBackingKind::OwnedAllocation,
                state: SourceObligationState::Outstanding,
            });
        }
        transferred.push(dependency.source_instance.clone());
    }
    Ok(Some(transition))
}
