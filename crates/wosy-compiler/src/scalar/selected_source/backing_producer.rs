use super::*;

pub(super) fn register(cfg: &mut ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    for point in cfg.initial_points.clone() {
        let CfgPointKind::Backing {
            target,
            transfer: RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed,
            ..
        } = &point.kind
        else {
            continue;
        };
        let shapes = cfg
            .evaluated_values
            .source_binding_shapes
            .iter()
            .filter(|shape| shape.binding == *target)
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err(invalid(&point.source_span));
        };
        let shape = shape.shape.clone();
        let event = SourceEventId {
            owner: cfg.producer_owner.clone(),
            point: point.id,
        };
        if cfg
            .evaluated_values
            .source_backing_producers
            .iter()
            .any(|producer| producer.event == event)
            || cfg
                .evaluated_values
                .values
                .iter()
                .any(|value| value.id.point == point.id)
        {
            return Err(invalid(&point.source_span));
        }
        let value = bind_evaluated_value_id(
            cfg,
            ResolvedValueOutputId {
                producer: cfg.producer_owner.clone(),
                occurrence: point.source_span.clone(),
                output_projection: Vec::new(),
                output: 0,
            },
            point.id,
        )
        .map_err(|_| invalid(&point.source_span))?;
        cfg.evaluated_values.values.push(EvaluatedValueFact {
            id: value.clone(),
            shape: shape.clone(),
            cases: Vec::new(),
        });
        cfg.evaluated_values
            .source_backing_producers
            .push(SourceBackingProducerFact {
                event,
                target: target.clone(),
                value,
                shape,
            });
    }
    Ok(())
}

pub(super) fn resolve(
    tables: &EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    point: &ReferenceFlowPoint,
) -> Result<SourceBackingProducerFact, ReferenceAnalysisError> {
    let CfgPointKind::Backing {
        target,
        transfer: RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed,
        ..
    } = &point.kind
    else {
        return Err(invalid(&point.source_span));
    };
    let event = SourceEventId {
        owner: owner.clone(),
        point: point.id,
    };
    let producers = tables
        .source_backing_producers
        .iter()
        .filter(|producer| producer.event == event)
        .collect::<Vec<_>>();
    let [producer] = producers.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    let shapes = tables
        .source_binding_shapes
        .iter()
        .filter(|shape| shape.binding == *target)
        .collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    let values = tables
        .values
        .iter()
        .filter(|value| value.id.point == point.id)
        .collect::<Vec<_>>();
    let [value] = values.as_slice() else {
        return Err(invalid(&point.source_span));
    };
    if producer.target != *target
        || producer.shape != shape.shape
        || value.id != producer.value
        || value.shape != producer.shape
        || producer.value.resolved.producer != *owner
        || producer.value.point != point.id
        || producer.value.resolved.occurrence != point.source_span
        || producer.value.resolved.output != 0
        || !producer.value.resolved.output_projection.is_empty()
    {
        return Err(invalid(&point.source_span));
    }
    Ok((*producer).clone())
}

pub(super) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    for point in &cfg.initial_points {
        if matches!(
            point.kind,
            CfgPointKind::Backing {
                transfer: RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed,
                ..
            }
        ) {
            resolve(&cfg.evaluated_values, &cfg.producer_owner, point)?;
        }
    }
    for producer in &cfg.evaluated_values.source_backing_producers {
        let point = cfg
            .initial_points
            .get(producer.event.point.0)
            .ok_or_else(|| invalid(&cfg.initial_points[0].source_span))?;
        if resolve(&cfg.evaluated_values, &cfg.producer_owner, point)? != *producer {
            return Err(invalid(&point.source_span));
        }
        let tables = &cfg.evaluated_values;
        for rule in tables
            .source_contributors
            .iter()
            .filter(|rule| rule.event == producer.event)
        {
            let row = tables
                .source_configurations
                .get(rule.output.index)
                .filter(|row| row.id == rule.output)
                .ok_or_else(|| invalid(&point.source_span))?;
            let value = selected_slot(
                tables,
                row,
                &SourceCoordinateRole::Evaluated(producer.value.clone()),
            )
            .map_err(|_| invalid(&point.source_span))?;
            let binding = selected_slot(
                tables,
                row,
                &SourceCoordinateRole::Binding(producer.target.clone()),
            )
            .map_err(|_| invalid(&point.source_span))?;
            let observations = tables
                .source_case_views
                .iter()
                .filter(|observation| {
                    observation.contributor == rule.id && observation.case.value == producer.value
                })
                .collect::<Vec<_>>();
            if observations.len() != 2
                || value.view != binding.view
                || value.availability != ValueAvailability::Available
                || binding.availability != ValueAvailability::Available
                || rule.outputs != vec![producer.value.clone()]
                || rule.productions.len() != 2
                || !rule.transports.iter().all(|transport| {
                    transport.destination != value.coordinate
                        && transport.destination != binding.coordinate
                })
            {
                return Err(invalid(&point.source_span));
            }
            for slot in [value, binding] {
                let selections = observations
                    .iter()
                    .filter(|observation| observation.port == slot.coordinate)
                    .collect::<Vec<_>>();
                let [observation] = selections.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                if observation.view != slot.view
                    || observation.configuration != row.id
                    || observations.iter().any(|other| {
                        other.case != observation.case
                            || other.path != observation.path
                            || other.view != observation.view
                    })
                    || !rule.productions.iter().any(|production| {
                        production.destination == slot.coordinate && production.inputs.is_empty()
                    })
                {
                    return Err(invalid(&point.source_span));
                }
            }
            let view = &tables.source_views[value.view.index];
            if view.shape != producer.shape {
                return Err(invalid(&point.source_span));
            }
            let SourcePayloadView::Backing { instance, .. } = &view.payload else {
                return Err(invalid(&point.source_span));
            };
            if !row.created.iter().any(|created| {
                created.instance == *instance
                    && created.event == producer.event
                    && created.role == SourceObjectRole::Payload
            }) {
                return Err(invalid(&point.source_span));
            }
            let capabilities = tables
                .owner_capabilities
                .iter()
                .filter(|capability| capability.role == OwnerRole::Binding(producer.target.clone()))
                .collect::<Vec<_>>();
            let [capability] = capabilities.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            let owner_instance = current_owner(tables, row, &capability.id)
                .map_err(|_| invalid(&point.source_span))?;
            let duties = row
                .owns
                .iter()
                .filter(|owns| {
                    owns.payload == *instance
                        && owns.capability == capability.id
                        && owns.owner_instance == owner_instance
                        && owns.state == SourceObligationState::Outstanding
                })
                .collect::<Vec<_>>();
            if duties.is_empty() {
                return Err(invalid(&point.source_span));
            }
            if duties
                .iter()
                .any(|duty| duty.kind == RuntimeBackingKind::OwnedAllocation)
            {
                let OwnerParent::Nested(storage_owner) = &capability.parent else {
                    return Err(invalid(&point.source_span));
                };
                let dependency = EvaluatedDependencyFact {
                    projection: Vec::new(),
                    source: DependencySource::Allocation(OwnerRecordFamilyId {
                        producer: producer.event.owner.clone(),
                        creation: producer.event.point,
                        projection: Vec::new(),
                    }),
                    action: DependencyAction::TransferUnique {
                        source_owner: capability.id.clone(),
                        receiving_owner: storage_owner.clone(),
                    },
                    witness: point.source_span.clone(),
                };
                if view.dependencies != vec![dependency] {
                    return Err(invalid(&point.source_span));
                }
            }
            let facts = tables
                .values
                .iter()
                .filter(|fact| fact.id == producer.value)
                .collect::<Vec<_>>();
            let [fact] = facts.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            for observation in observations {
                let case = fact
                    .cases
                    .get(observation.case.alternative)
                    .filter(|case| case.id == observation.case)
                    .ok_or_else(|| invalid(&point.source_span))?;
                if case.dependencies != view.dependencies {
                    return Err(invalid(&point.source_span));
                }
            }
        }
    }
    Ok(())
}
