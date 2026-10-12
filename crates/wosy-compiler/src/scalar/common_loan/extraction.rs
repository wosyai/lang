use super::*;
#[cfg(test)]
#[path = "qualified_assignment_tests.rs"]
mod qualified_assignment_tests;

fn role(
    input: &mut CommonLoanInput,
    source: CommonLoanSourceRole,
    projection: Vec<ValueProjection>,
) -> CommonLoanRoleId {
    let role = CommonLoanRole {
        owner: input.owner.clone(),
        source,
        projection,
    };
    if let Some(index) = input.roles.iter().position(|old| *old == role) {
        return CommonLoanRoleId(index);
    }
    let id = CommonLoanRoleId(input.roles.len());
    input.roles.push(role);
    id
}
fn fact(event: &mut CommonLoanEvent, kind: CommonLoanFactKind) {
    event.facts.push(CommonLoanFact {
        event: event.event.clone(),
        source_span: event.source_span.clone(),
        kind,
    });
}
fn pending(input: &mut CommonLoanInput, event: &CommonLoanEvent, reason: CommonLoanPendingReason) {
    let row = CommonLoanPending {
        event: event.event.clone(),
        source_span: event.source_span.clone(),
        reason,
    };
    if !input.pending.contains(&row) {
        input.pending.push(row);
    }
}
fn output(
    input: &mut CommonLoanInput,
    event: &CommonLoanEvent,
    point: CfgPointId,
) -> Option<EvaluatedValueId> {
    let values = input
        .outputs
        .iter()
        .filter(|output| output.value.point == point)
        .map(|output| output.value.clone())
        .collect::<Vec<_>>();
    match values.as_slice() {
        [value] => Some(value.clone()),
        [] => {
            pending(input, event, CommonLoanPendingReason::MissingOutput);
            None
        }
        _ => {
            pending(
                input,
                event,
                CommonLoanPendingReason::AmbiguousOutputs(values),
            );
            None
        }
    }
}
fn projection(place: &ReferencePlaceId) -> Option<Vec<ValueProjection>> {
    let mut result = Vec::new();
    for component in &place.projections {
        let ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) = component
        else {
            return None;
        };
        result.push(ValueProjection::Field(field.clone()));
    }
    Some(result)
}
fn storage(
    input: &mut CommonLoanInput,
    event: &CommonLoanEvent,
    place: &ReferencePlaceId,
) -> Option<(CommonLoanRoleId, CommonLoanTarget)> {
    if place.binding.source != place.source {
        pending(input, event, CommonLoanPendingReason::Projection);
        return None;
    }
    let Some(projection) = projection(place) else {
        pending(input, event, CommonLoanPendingReason::Projection);
        return None;
    };
    let target = CommonLoanTarget::Storage {
        binding: place.binding.clone(),
        projection: projection.clone(),
    };
    let role = role(
        input,
        CommonLoanSourceRole::Binding(place.binding.clone()),
        projection,
    );
    Some((role, target))
}
fn policy(
    input: &mut CommonLoanInput,
    event: &CommonLoanEvent,
    value: &EvaluatedValueId,
) -> Option<CopyPolicy> {
    let values = input
        .outputs
        .iter()
        .filter(|output| output.value == *value)
        .collect::<Vec<_>>();
    let [output] = values.as_slice() else {
        pending(input, event, CommonLoanPendingReason::MissingOutput);
        return None;
    };
    match output.shape.payload {
        ValuePayloadShape::Plain(_) => Some(CopyPolicy::Copy),
        ValuePayloadShape::Checked {
            mutability: ScalarReferenceMutability::Shared,
            ..
        } => Some(CopyPolicy::Copy),
        ValuePayloadShape::Checked {
            mutability: ScalarReferenceMutability::Mutable,
            ..
        } => Some(CopyPolicy::Move),
        _ => {
            let shape = output.shape.id.clone();
            pending(input, event, CommonLoanPendingReason::Shape(shape));
            None
        }
    }
}
fn flow(
    input: &mut CommonLoanInput,
    event: &mut CommonLoanEvent,
    value: &EvaluatedValueId,
    target: CommonLoanRoleId,
    policy: CopyPolicy,
) {
    let source = role(
        input,
        CommonLoanSourceRole::Evaluated(value.clone()),
        Vec::new(),
    );
    fact(
        event,
        CommonLoanFactKind::Flow {
            source,
            target,
            policy,
        },
    );
}
fn through(
    input: &mut CommonLoanInput,
    event: &CommonLoanEvent,
    pointer_point: CfgPointId,
    place: &ScalarPlace,
) -> Option<(CommonLoanRoleId, CommonLoanTarget)> {
    let value = output(input, event, pointer_point)?;
    let pointer = role(input, CommonLoanSourceRole::Evaluated(value), Vec::new());
    if !matches!(place, ScalarPlace::Dereference { .. }) {
        pending(input, event, CommonLoanPendingReason::Projection);
    }
    Some((
        pointer,
        CommonLoanTarget::Through {
            pointer,
            place: place.clone(),
        },
    ))
}

pub(super) fn extract(
    tables: &EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    points: &[ReferenceFlowPoint],
) -> Result<CommonLoanInput, ReferenceAnalysisError> {
    let mut input = CommonLoanInput {
        owner: owner.clone(),
        outputs: Vec::new(),
        binding_shapes: tables.source_binding_shapes.clone(),
        final_inputs: tables.final_inputs.clone(),
        roles: Vec::new(),
        sites: Vec::new(),
        events: Vec::new(),
        pending: Vec::new(),
    };
    for value in &tables.values {
        if value.id.resolved.producer != *owner {
            continue;
        }
        if input.outputs.iter().any(|output| output.value == value.id) {
            return Err(invalid(&points[0].source_span));
        }
        let point = points
            .get(value.id.point.0)
            .ok_or_else(|| invalid(&points[0].source_span))?;
        if value.id.resolved.occurrence.source != point.source_span.source {
            return Err(invalid(&point.source_span));
        }
        let shapes = tables
            .shapes
            .iter()
            .filter(|shape| shape.id == value.shape)
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err(invalid(&point.source_span));
        };
        input.outputs.push(CommonLoanOutput {
            value: value.id.clone(),
            shape: (*shape).clone(),
        });
    }
    for (index, point) in points.iter().enumerate() {
        if point.id != CfgPointId(index)
            || point
                .successors
                .iter()
                .any(|successor| successor.0 >= points.len())
        {
            return Err(invalid(&point.source_span));
        }
        let mut event = CommonLoanEvent {
            event: SourceEventId {
                owner: owner.clone(),
                point: point.id,
            },
            source_span: point.source_span.clone(),
            kind: (&point.kind).into(),
            successors: point
                .successors
                .iter()
                .map(|point| SourceEventId {
                    owner: owner.clone(),
                    point: *point,
                })
                .collect(),
            facts: Vec::new(),
        };
        match &point.kind {
            CfgPointKind::Entry => {
                for signature in tables
                    .boundary_signatures
                    .iter()
                    .filter(|signature| signature.owner == *owner)
                {
                    for port in &signature.ports {
                        role(
                            &mut input,
                            CommonLoanSourceRole::Boundary(port.role.clone()),
                            Vec::new(),
                        );
                        if !matches!(port.role, BoundaryPortRole::Output(_)) {
                            pending(&mut input, &event, CommonLoanPendingReason::Boundary);
                        }
                    }
                }
            }
            CfgPointKind::Borrow { request } => {
                let destination = output(&mut input, &event, point.id);
                let access = match request {
                    ReferenceBorrowRequest::DirectStorage { place, loan, .. } => {
                        storage(&mut input, &event, place)
                            .map(|(role, target)| (role, target, loan.mode, None))
                    }
                    ReferenceBorrowRequest::Through {
                        pointer_point,
                        place,
                        mode,
                        ..
                    } => through(&mut input, &event, *pointer_point, place)
                        .map(|(role, target)| (role, target, *mode, Some(role))),
                };
                if let (Some(value), Some((access_role, target, mode, parent))) =
                    (destination, access)
                {
                    let outputs = input
                        .outputs
                        .iter()
                        .filter(|output| output.value == value)
                        .collect::<Vec<_>>();
                    let [actual] = outputs.as_slice() else {
                        return Err(invalid(&point.source_span));
                    };
                    if !matches!(actual.shape.payload, ValuePayloadShape::Checked { mutability, .. } if mutability == mode)
                    {
                        return Err(invalid(&point.source_span));
                    }
                    fact(
                        &mut event,
                        CommonLoanFactKind::Use {
                            role: access_role,
                            mode,
                            target: target.clone(),
                            kind: CommonLoanUseKind::Access,
                        },
                    );
                    let site = CommonLoanSiteId(input.sites.len());
                    input.sites.push(CommonLoanSite {
                        event: event.event.clone(),
                        source_span: point.source_span.clone(),
                        mode,
                        target,
                    });
                    let output = role(
                        &mut input,
                        CommonLoanSourceRole::Evaluated(value),
                        Vec::new(),
                    );
                    fact(&mut event, CommonLoanFactKind::Issue { output, site });
                    if let Some(parent) = parent {
                        fact(
                            &mut event,
                            CommonLoanFactKind::ParentSite {
                                child: site,
                                parent,
                            },
                        );
                    }
                }
            }
            CfgPointKind::Read { binding } => {
                let source = role(
                    &mut input,
                    CommonLoanSourceRole::Binding(binding.clone()),
                    Vec::new(),
                );
                let target = CommonLoanTarget::Storage {
                    binding: binding.clone(),
                    projection: Vec::new(),
                };
                fact(
                    &mut event,
                    CommonLoanFactKind::Use {
                        role: source,
                        mode: ScalarReferenceMutability::Shared,
                        target,
                        kind: CommonLoanUseKind::Access,
                    },
                );
                if let Some(value) = output(&mut input, &event, point.id) {
                    let target = role(
                        &mut input,
                        CommonLoanSourceRole::Evaluated(value.clone()),
                        Vec::new(),
                    );
                    fact(
                        &mut event,
                        CommonLoanFactKind::Flow {
                            source,
                            target,
                            policy: CopyPolicy::Copy,
                        },
                    );
                    policy(&mut input, &event, &value);
                }
            }
            CfgPointKind::ReadPlace {
                pointer_point,
                place,
                candidates,
                ..
            } => match pointer_point {
                Some(pointer) => {
                    if let Some((pointer, target)) = through(&mut input, &event, *pointer, place) {
                        fact(
                            &mut event,
                            CommonLoanFactKind::Use {
                                role: pointer,
                                mode: ScalarReferenceMutability::Shared,
                                target,
                                kind: CommonLoanUseKind::Access,
                            },
                        );
                        if let Some(value) = output(&mut input, &event, point.id) {
                            let outputs = input
                                .outputs
                                .iter()
                                .filter(|output| output.value == value)
                                .collect::<Vec<_>>();
                            let [actual] = outputs.as_slice() else {
                                return Err(invalid(&point.source_span));
                            };
                            if matches!(actual.shape.payload, ValuePayloadShape::Plain(_)) {
                                fact(
                                    &mut event,
                                    CommonLoanFactKind::NoLoan {
                                        outputs: vec![value],
                                    },
                                );
                            } else {
                                pending(
                                    &mut input,
                                    &event,
                                    CommonLoanPendingReason::SelectedTarget,
                                );
                            }
                        }
                    }
                }
                None => {
                    if let [place] = candidates.as_slice() {
                        if let Some((source, target)) = storage(&mut input, &event, place) {
                            fact(
                                &mut event,
                                CommonLoanFactKind::Use {
                                    role: source,
                                    mode: ScalarReferenceMutability::Shared,
                                    target,
                                    kind: CommonLoanUseKind::Access,
                                },
                            );
                            if let Some(value) = output(&mut input, &event, point.id) {
                                let target = role(
                                    &mut input,
                                    CommonLoanSourceRole::Evaluated(value.clone()),
                                    Vec::new(),
                                );
                                fact(
                                    &mut event,
                                    CommonLoanFactKind::Flow {
                                        source,
                                        target,
                                        policy: CopyPolicy::Copy,
                                    },
                                );
                                policy(&mut input, &event, &value);
                            }
                        }
                    } else {
                        pending(&mut input, &event, CommonLoanPendingReason::SelectedTarget);
                    }
                }
            },
            CfgPointKind::Bind { binding } => {
                let target = role(
                    &mut input,
                    CommonLoanSourceRole::Binding(binding.clone()),
                    Vec::new(),
                );
                fact(&mut event, CommonLoanFactKind::KillSlot(target));
                let sources = tables
                    .binding_inputs
                    .iter()
                    .filter(|source| source.producer == *owner && source.consumer == point.id)
                    .collect::<Vec<_>>();
                if let [source] = sources.as_slice() {
                    let disposition = reference::binding_disposition(tables, source, binding)?;
                    flow(&mut input, &mut event, &source.source, target, disposition.policy);
                } else {
                    pending(
                        &mut input,
                        &event,
                        CommonLoanPendingReason::MissingBindingInput(binding.clone()),
                    );
                }
            }
            CfgPointKind::Move { place, .. } => {
                let sources = tables
                    .source_move_inputs
                    .iter()
                    .filter(|source| source.event == event.event)
                    .collect::<Vec<_>>();
                if let [source] = sources.as_slice() {
                    if source.place != *place {
                        return Err(invalid(&point.source_span));
                    }
                    if let Some((storage, target)) = storage(&mut input, &event, place) {
                        fact(
                            &mut event,
                            CommonLoanFactKind::Use {
                                role: storage,
                                mode: ScalarReferenceMutability::Mutable,
                                target,
                                kind: CommonLoanUseKind::Access,
                            },
                        );
                        let evaluated = role(
                            &mut input,
                            CommonLoanSourceRole::Evaluated(source.source.clone()),
                            Vec::new(),
                        );
                        fact(
                            &mut event,
                            CommonLoanFactKind::Flow {
                                source: storage,
                                target: evaluated,
                                policy: CopyPolicy::Move,
                            },
                        );
                        fact(&mut event, CommonLoanFactKind::KillSlot(storage));
                    }
                } else {
                    pending(&mut input, &event, CommonLoanPendingReason::MissingOutput);
                }
            }
            CfgPointKind::AssignmentDestination {
                storage: destination,
                ..
            } => {
                let destinations = tables
                    .destinations
                    .iter()
                    .filter(|destination| {
                        destination.id.operation.producer == *owner && destination.point == point.id
                    })
                    .collect::<Vec<_>>();
                let [actual] = destinations.as_slice() else {
                    pending(
                        &mut input,
                        &event,
                        CommonLoanPendingReason::MissingAssignmentInput,
                    );
                    input.events.push(event);
                    continue;
                };
                if actual.storage != *destination || actual.source_span != point.source_span {
                    return Err(invalid(&point.source_span));
                }
                let frozen = role(
                    &mut input,
                    CommonLoanSourceRole::Frozen(event.event.clone()),
                    Vec::new(),
                );
                if let EvaluatedDestinationStorage::Through { pointer, .. } = destination {
                    flow(&mut input, &mut event, pointer, frozen, CopyPolicy::Copy);
                }
            }
            CfgPointKind::AssignmentSnapshot {
                destination_point, ..
            } => {
                let destinations = tables
                    .destinations
                    .iter()
                    .filter(|destination| {
                        destination.id.operation.producer == *owner
                            && destination.point == *destination_point
                    })
                    .collect::<Vec<_>>();
                let [destination] = destinations.as_slice() else {
                    pending(
                        &mut input,
                        &event,
                        CommonLoanPendingReason::MissingAssignmentInput,
                    );
                    input.events.push(event);
                    continue;
                };
                let snapshots = tables
                    .snapshots
                    .iter()
                    .filter(|snapshot| {
                        snapshot.destination.operation.producer == *owner
                            && snapshot.point == point.id
                            && snapshot.destination == destination.id
                    })
                    .collect::<Vec<_>>();
                let [snapshot] = snapshots.as_slice() else {
                    pending(
                        &mut input,
                        &event,
                        CommonLoanPendingReason::MissingAssignmentInput,
                    );
                    input.events.push(event);
                    continue;
                };
                let saved = role(
                    &mut input,
                    CommonLoanSourceRole::Saved(event.event.clone()),
                    Vec::new(),
                );
                if let EvaluatedDestinationStorage::Direct(place) = &destination.storage {
                    if let Some((source, target)) = storage(&mut input, &event, place) {
                        fact(
                            &mut event,
                            CommonLoanFactKind::Use {
                                role: source,
                                mode: ScalarReferenceMutability::Shared,
                                target,
                                kind: CommonLoanUseKind::Access,
                            },
                        );
                        fact(
                            &mut event,
                            CommonLoanFactKind::Flow {
                                source,
                                target: saved,
                                policy: CopyPolicy::Copy,
                            },
                        );
                        if let SnapshotState::Saved(value) = &snapshot.state {
                            if value.point != point.id || value.resolved.producer != *owner {
                                return Err(invalid(&point.source_span));
                            }
                            let evaluated = role(
                                &mut input,
                                CommonLoanSourceRole::Evaluated(value.clone()),
                                Vec::new(),
                            );
                            fact(
                                &mut event,
                                CommonLoanFactKind::Flow {
                                    source: saved,
                                    target: evaluated,
                                    policy: CopyPolicy::Copy,
                                },
                            );
                        } else {
                            pending(
                                &mut input,
                                &event,
                                CommonLoanPendingReason::MissingAssignmentInput,
                            );
                        }
                    }
                } else {
                    pending(&mut input, &event, CommonLoanPendingReason::SelectedTarget);
                }
            }
            CfgPointKind::AssignmentResult {
                snapshot_point,
                output: ordinal,
            } => {
                let results = tables
                    .results
                    .iter()
                    .filter(|result| {
                        result.operation.producer == *owner
                            && result.value.resolved.producer == *owner
                            && result.snapshot == *snapshot_point
                            && result.output == *ordinal
                            && result.value.point == point.id
                    })
                    .collect::<Vec<_>>();
                let [result] = results.as_slice() else {
                    pending(
                        &mut input,
                        &event,
                        CommonLoanPendingReason::MissingAssignmentInput,
                    );
                    input.events.push(event);
                    continue;
                };
                let saved = role(
                    &mut input,
                    CommonLoanSourceRole::Saved(SourceEventId {
                        owner: owner.clone(),
                        point: *snapshot_point,
                    }),
                    Vec::new(),
                );
                let result_role = role(
                    &mut input,
                    CommonLoanSourceRole::Result(event.event.clone()),
                    Vec::new(),
                );
                let evaluated = role(
                    &mut input,
                    CommonLoanSourceRole::Evaluated(result.value.clone()),
                    Vec::new(),
                );
                fact(
                    &mut event,
                    CommonLoanFactKind::Flow {
                        source: saved,
                        target: result_role,
                        policy: CopyPolicy::Copy,
                    },
                );
                fact(
                    &mut event,
                    CommonLoanFactKind::Flow {
                        source: result_role,
                        target: evaluated,
                        policy: CopyPolicy::Copy,
                    },
                );
            }
            CfgPointKind::AssignmentDispose { snapshot_point } => {
                let saved = role(
                    &mut input,
                    CommonLoanSourceRole::Saved(SourceEventId {
                        owner: owner.clone(),
                        point: *snapshot_point,
                    }),
                    Vec::new(),
                );
                fact(&mut event, CommonLoanFactKind::KillSlot(saved));
            }
            CfgPointKind::Assign {
                target: place,
                rhs_point,
                ..
            } => {
                let commits = tables
                    .commits
                    .iter()
                    .filter(|commit| {
                        commit.destination.operation.producer == *owner
                            && commit.replacement.resolved.producer == *owner
                            && commit.point == point.id
                    })
                    .collect::<Vec<_>>();
                let [commit] = commits.as_slice() else {
                    pending(
                        &mut input,
                        &event,
                        CommonLoanPendingReason::MissingAssignmentInput,
                    );
                    input.events.push(event);
                    continue;
                };
                let destinations = tables
                    .destinations
                    .iter()
                    .filter(|destination| destination.id == commit.destination)
                    .collect::<Vec<_>>();
                let [destination] = destinations.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                let corresponds = match &destination.storage {
                    EvaluatedDestinationStorage::Direct(actual) => *actual == *place,
                    EvaluatedDestinationStorage::Inferred(binding) => {
                        *binding == place.binding && place.projections.is_empty()
                    }
                    EvaluatedDestinationStorage::Through { .. } => false,
                };
                if !corresponds || commit.replacement.point != *rhs_point {
                    return Err(invalid(&point.source_span));
                }
                let value = &commit.replacement;
                if let Some((target, place)) = storage(&mut input, &event, place) {
                    fact(
                        &mut event,
                        CommonLoanFactKind::Use {
                            role: target,
                            mode: ScalarReferenceMutability::Mutable,
                            target: place,
                            kind: CommonLoanUseKind::Access,
                        },
                    );
                    fact(&mut event, CommonLoanFactKind::KillSlot(target));
                    if let Some(policy) = policy(&mut input, &event, value) {
                        flow(&mut input, &mut event, value, target, policy);
                    }
                }
            }
            CfgPointKind::AssignThrough {
                pointer_point,
                place,
                ..
            } => {
                if let Some((pointer, target)) = through(&mut input, &event, *pointer_point, place)
                {
                    fact(
                        &mut event,
                        CommonLoanFactKind::Use {
                            role: pointer,
                            mode: ScalarReferenceMutability::Mutable,
                            target,
                            kind: CommonLoanUseKind::Access,
                        },
                    );
                }
                pending(&mut input, &event, CommonLoanPendingReason::SelectedTarget);
            }
            CfgPointKind::ReturnOutput {
                output: ordinal, ..
            } => {
                let sources = tables.final_inputs.iter().filter(|source| source.consumer.producer == *owner && source.consumer.point == point.id && matches!(source.consumer.kind, ValueConsumerKind::FinalOutput { output, .. } if output == *ordinal)).collect::<Vec<_>>();
                let [source] = sources.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                let target = role(
                    &mut input,
                    CommonLoanSourceRole::Result(event.event.clone()),
                    Vec::new(),
                );
                let policies = tables
                    .disposition_inputs
                    .iter()
                    .filter(|policy| {
                        policy.source == source.source && policy.consumer == source.consumer
                    })
                    .collect::<Vec<_>>();
                let [policy] = policies.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                if policy.policy == CopyPolicy::Deferred {
                    pending(&mut input, &event, CommonLoanPendingReason::DeferredPolicy);
                }
                flow(
                    &mut input,
                    &mut event,
                    &source.source,
                    target,
                    policy.policy,
                );
                let source_role = role(
                    &mut input,
                    CommonLoanSourceRole::Evaluated(source.source.clone()),
                    Vec::new(),
                );
                let producer = points
                    .get(source.source.point.0)
                    .ok_or_else(|| invalid(&point.source_span))?;
                let target = match &producer.kind {
                    CfgPointKind::Read { binding } => Some(CommonLoanTarget::Storage {
                        binding: binding.clone(),
                        projection: Vec::new(),
                    }),
                    CfgPointKind::Borrow {
                        request: ReferenceBorrowRequest::DirectStorage { place, .. },
                    } => storage(&mut input, &event, place).map(|(_, target)| target),
                    CfgPointKind::Borrow {
                        request:
                            ReferenceBorrowRequest::Through {
                                pointer_point,
                                place,
                                ..
                            },
                    }
                    | CfgPointKind::ReadPlace {
                        pointer_point: Some(pointer_point),
                        place,
                        ..
                    } => {
                        through(&mut input, &event, *pointer_point, place).map(|(_, target)| target)
                    }
                    CfgPointKind::ReadPlace {
                        pointer_point: None,
                        candidates,
                        ..
                    } => {
                        if let [place] = candidates.as_slice() {
                            storage(&mut input, &event, place).map(|(_, target)| target)
                        } else {
                            None
                        }
                    }
                    CfgPointKind::AssignmentResult { snapshot_point, .. } => {
                        let snapshots = tables
                            .snapshots
                            .iter()
                            .filter(|snapshot| {
                                snapshot.destination.operation.producer == *owner
                                    && snapshot.point == *snapshot_point
                            })
                            .collect::<Vec<_>>();
                        let [snapshot] = snapshots.as_slice() else {
                            return Err(invalid(&point.source_span));
                        };
                        if source.source.resolved.producer != *owner {
                            return Err(invalid(&point.source_span));
                        }
                        if let SnapshotState::Saved(value) = &snapshot.state {
                            if value.resolved.producer != *owner || value.point != *snapshot_point {
                                return Err(invalid(&point.source_span));
                            }
                        }
                        let destinations = tables
                            .destinations
                            .iter()
                            .filter(|destination| destination.id == snapshot.destination)
                            .collect::<Vec<_>>();
                        let [destination] = destinations.as_slice() else {
                            return Err(invalid(&point.source_span));
                        };
                        match &destination.storage {
                            EvaluatedDestinationStorage::Direct(place) => {
                                storage(&mut input, &event, place).map(|(_, target)| target)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    fact(
                        &mut event,
                        CommonLoanFactKind::Use {
                            role: source_role,
                            mode: ScalarReferenceMutability::Shared,
                            target,
                            kind: CommonLoanUseKind::Escape,
                        },
                    );
                } else {
                    let outputs = input
                        .outputs
                        .iter()
                        .filter(|output| output.value == source.source)
                        .collect::<Vec<_>>();
                    let [output] = outputs.as_slice() else {
                        return Err(invalid(&point.source_span));
                    };
                    if matches!(output.shape.payload, ValuePayloadShape::Plain(_)) {
                        fact(
                            &mut event,
                            CommonLoanFactKind::NoLoan {
                                outputs: vec![source.source.clone()],
                            },
                        );
                    } else {
                        pending(&mut input, &event, CommonLoanPendingReason::SelectedTarget);
                    }
                }
            }
            CfgPointKind::ScopeExit => {
                let bindings = input
                    .binding_shapes
                    .iter()
                    .filter(|shape| {
                        shape.binding.block_span == point.source_span.range
                            && shape.binding.source == point.source_span.source
                    })
                    .map(|shape| shape.binding.clone())
                    .collect::<Vec<_>>();
                for binding in bindings {
                    let roles = input.roles.iter().enumerate().filter(|(_, role)| matches!(&role.source, CommonLoanSourceRole::Binding(actual) if *actual == binding)).map(|(index, _)| CommonLoanRoleId(index)).collect::<Vec<_>>();
                    for role in roles {
                        fact(&mut event, CommonLoanFactKind::KillSlot(role));
                    }
                }
            }
            CfgPointKind::Capture { .. } => {
                pending(&mut input, &event, CommonLoanPendingReason::Capture)
            }
            CfgPointKind::Call { .. } => pending(&mut input, &event, CommonLoanPendingReason::Call),
            CfgPointKind::AllocationIdentityAddress { .. } => {
                pending(&mut input, &event, CommonLoanPendingReason::RawAuthority)
            }
            CfgPointKind::Release { .. } => {
                pending(&mut input, &event, CommonLoanPendingReason::Release)
            }
            CfgPointKind::Invalidate { .. } => {
                pending(&mut input, &event, CommonLoanPendingReason::Invalidate)
            }
            CfgPointKind::DeferredMove { .. }
            | CfgPointKind::DeferredMoveThrough { .. }
            | CfgPointKind::DeferredArrayElement { .. } => {
                pending(&mut input, &event, CommonLoanPendingReason::DeferredPolicy)
            }
            CfgPointKind::MoveThrough { .. }
            | CfgPointKind::BindArrayElement { .. }
            | CfgPointKind::BackingPlace { .. }
            | CfgPointKind::CopyCallableFields { .. }
            | CfgPointKind::CopyProjectedBackings { .. }
            | CfgPointKind::SelectArrayElement { .. }
            | CfgPointKind::Backing { .. } => {
                pending(&mut input, &event, CommonLoanPendingReason::SelectedTarget)
            }
            CfgPointKind::ValueOutputProjection { .. } => {}
            CfgPointKind::Evaluate(_)
            | CfgPointKind::Branch { .. }
            | CfgPointKind::LoopTest { .. }
            | CfgPointKind::ScopeEnter
            | CfgPointKind::Join
            | CfgPointKind::Return => {
                let outputs = input
                    .outputs
                    .iter()
                    .filter(|output| output.value.point == point.id)
                    .cloned()
                    .collect::<Vec<_>>();
                if outputs
                    .iter()
                    .all(|output| matches!(output.shape.payload, ValuePayloadShape::Plain(_)))
                {
                    fact(
                        &mut event,
                        CommonLoanFactKind::NoLoan {
                            outputs: outputs.iter().map(|output| output.value.clone()).collect(),
                        },
                    );
                }
                for output in outputs {
                    if !matches!(output.shape.payload, ValuePayloadShape::Plain(_)) {
                        pending(
                            &mut input,
                            &event,
                            CommonLoanPendingReason::Shape(output.shape.id),
                        );
                    }
                }
            }
        }
        for successor in &point.successors {
            for (source, destination) in reference::conditional_output_production::edge_outputs(tables, owner, point.id, *successor)? {
                let source = role(&mut input, CommonLoanSourceRole::Evaluated(source), Vec::new());
                let target = role(&mut input, CommonLoanSourceRole::Evaluated(destination), Vec::new());
                fact(&mut event, CommonLoanFactKind::Flow { source, target, policy: CopyPolicy::Copy });
            }
        }
        input.events.push(event);
    }
    Ok(input)
}
