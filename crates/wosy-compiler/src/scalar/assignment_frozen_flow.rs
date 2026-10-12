use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrozenAssignmentDestination {
    pub(super) selection: FrozenSelectionOccurrence,
    pub(super) destination: AssignmentDestinationId,
    pub(super) point: CfgPointId,
    pub(super) place: ReferencePlaceId,
    pub(super) access: FrozenAssignmentAccess,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FrozenAssignmentAccess {
    Direct,
    Through {
        pointer: EvaluatedValueId,
        origin: ReferenceOriginId,
        loan: ReferenceLoanId,
    },
    Indexed {
        holder: CallableArraySelectedHolderFact,
        key: CallableIndexKeyFact,
        guard: Vec<CallableIndexEqualityFact>,
    },
    ThroughIndexed {
        pointer: EvaluatedValueId,
        origin: ReferenceOriginId,
        loan: ReferenceLoanId,
        holder: CallableArraySelectedHolderFact,
        key: CallableIndexKeyFact,
        guard: Vec<CallableIndexEqualityFact>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrozenPhaseContext {
    pub(super) usage: FrozenGuardUse,
    pub(super) destination_point: CfgPointId,
}

fn validate_use(
    state: &ReferenceEdgeState,
    usage: &FrozenGuardUse,
) -> Result<(), ReferenceAnalysisErrorReason> {
    if usage.selection.application.frame != state.self_frame
        || usage.selection.application.operation != usage.selection.destination.operation
        || usage.selection.application.operation.producer != state.self_frame.owner
    {
        return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
    }
    let applications = state
        .assignment_applications
        .iter()
        .filter(|application| application.application == usage.selection.application)
        .collect::<Vec<_>>();
    let [application] = applications.as_slice() else {
        return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
    };
    if application
        .remaining
        .iter()
        .filter(|pending| *pending == usage)
        .count()
        != 1
    {
        return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
    }
    Ok(())
}

fn activate(
    state: &mut ReferenceEdgeState,
    protocol: &assignment_lifecycle::FrozenAssignmentProtocol,
    point: CfgPointId,
) -> Result<FrozenSelectionOccurrence, ReferenceAnalysisErrorReason> {
    let selection = protocol.selection(&state.self_frame, point)?;
    let count = state
        .assignment_applications
        .iter()
        .filter(|active| active.application == selection.application)
        .count();
    if selection.destination.target == 0 {
        if count != 0 {
            return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
        }
        let mut remaining = Vec::new();
        for destination in protocol.destinations.iter().filter(|destination| {
            destination.destination.operation == selection.application.operation
        }) {
            let owning_selection = protocol.selection(&state.self_frame, destination.point)?;
            for usage in protocol.uses(&owning_selection)? {
                if remaining.contains(&usage) {
                    return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
                }
                remaining.push(usage);
            }
        }
        state.assignment_applications.push(FrozenApplicationState {
            application: selection.application.clone(),
            remaining,
        });
    } else if count != 1 {
        return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
    }
    if state.assignment_destinations.contains_key(&point) {
        return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
    }
    Ok(selection)
}

fn consumer_point(consumer: &FrozenConsumerRole) -> CfgPointId {
    match consumer {
        FrozenConsumerRole::Snapshot(point)
        | FrozenConsumerRole::Commit(point)
        | FrozenConsumerRole::Dispose(point) => *point,
        FrozenConsumerRole::Result(value) => value.point,
    }
}

pub(super) fn phase_context(
    state: &ReferenceEdgeState,
    protocol: &assignment_lifecycle::FrozenAssignmentProtocol,
    point: CfgPointId,
) -> Result<Option<FrozenPhaseContext>, ReferenceAnalysisErrorReason> {
    let mut contexts = Vec::new();
    for destination in &protocol.destinations {
        for consumer in &destination.consumers {
            if consumer_point(consumer) != point {
                continue;
            }
            let selection = protocol.selection(&state.self_frame, destination.point)?;
            let usage = FrozenGuardUse {
                selection,
                consumer: consumer.clone(),
            };
            validate_use(state, &usage)?;
            let rows = state
                .assignment_destinations
                .get(&destination.point)
                .ok_or(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)?;
            if rows.iter().any(|row| {
                row.selection != usage.selection
                    || row.destination != destination.destination
                    || row.point != destination.point
            }) {
                return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
            }
            contexts.push(FrozenPhaseContext {
                usage,
                destination_point: destination.point,
            });
        }
    }
    match contexts.as_slice() {
        [] => Ok(None),
        [context] => Ok(Some(context.clone())),
        _ => Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence),
    }
}

pub(super) fn consume(
    state: &mut ReferenceEdgeState,
    context: &FrozenPhaseContext,
) -> Result<(), ReferenceAnalysisErrorReason> {
    validate_use(state, &context.usage)?;
    let rows = state
        .assignment_destinations
        .get(&context.destination_point)
        .ok_or(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)?;
    if rows.iter().any(|row| {
        row.selection != context.usage.selection || row.point != context.destination_point
    }) {
        return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
    }
    let application = state
        .assignment_applications
        .iter_mut()
        .find(|application| application.application == context.usage.selection.application)
        .expect("validated owning application");
    let position = application
        .remaining
        .iter()
        .position(|usage| usage == &context.usage)
        .expect("validated pending phase");
    application.remaining.remove(position);
    if !application
        .remaining
        .iter()
        .any(|usage| usage.selection == context.usage.selection)
    {
        let rows = state
            .assignment_destinations
            .remove(&context.destination_point)
            .expect("consumed selection rows");
        assert!(rows
            .iter()
            .all(|row| row.selection == context.usage.selection));
    }
    state.assignment_applications.retain(|application| {
        application.application != context.usage.selection.application
            || !application.remaining.is_empty()
    });
    Ok(())
}

pub(super) fn freeze(
    state: &mut ReferenceEdgeState,
    protocol: &assignment_lifecycle::FrozenAssignmentProtocol,
    destinations: &[EvaluatedAssignmentDestination],
    facts: &[ReferenceOrigin],
    point: &ReferenceFlowPoint,
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    let CfgPointKind::AssignmentDestination { storage, place } = &point.kind else {
        return;
    };
    let destination = destinations
        .iter()
        .find(|destination| destination.point == point.id)
        .expect("registered assignment destination");
    let selection = match activate(state, protocol, point.id) {
        Ok(selection) => selection,
        Err(reason) => {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: point.source_span.clone(),
                reason,
            });
            return;
        }
    };
    let mut rows = Vec::new();
    match storage {
        EvaluatedDestinationStorage::Inferred(_) => {}
        EvaluatedDestinationStorage::Direct(place) => match &state.selected_element {
            Some((index_point, selected))
                if selected.binding == place.binding
                    && state
                        .backing_bindings
                        .get(&selected.binding)
                        .is_some_and(|backings| {
                            backings.iter().any(|backing| {
                                backing.kind == RuntimeBackingKind::OwnedAllocation
                                    && !backing.callable_family.is_empty()
                            })
                        }) =>
            {
                let input = destination
                    .selector_inputs
                    .iter()
                    .find(|input| input.point == *index_point)
                    .expect("actual destination selector input");
                assert_eq!(input.resolved.producer, destination.id.operation.producer);
                let occurrence = input.resolved.occurrence.clone();
                let key = CallableIndexKeyFact::Evaluated {
                    point: *index_point,
                    occurrence: occurrence.clone(),
                    function_span: selected.function_span,
                };
                for backing in state
                    .backing_bindings
                    .get(&selected.binding)
                    .into_iter()
                    .flatten()
                {
                    for family in &backing.callable_family {
                        for partition in &family.partitions {
                            if partition
                                .representatives
                                .iter()
                                .any(|representative| representative.key == key)
                            {
                                let row = FrozenAssignmentDestination {
                                    selection: selection.clone(),
                                    destination: destination.id.clone(),
                                    point: point.id,
                                    place: selected.clone(),
                                    access: FrozenAssignmentAccess::Indexed {
                                        holder: CallableArraySelectedHolderFact {
                                            value: backing.value.clone(),
                                            owner: backing.owner.clone(),
                                            length: backing.length.clone(),
                                            index_point: *index_point,
                                            occurrence: occurrence.clone(),
                                            place: selected.clone(),
                                        },
                                        key: key.clone(),
                                        guard: partition.relations.clone(),
                                    },
                                };
                                if !rows.contains(&row) {
                                    rows.push(row);
                                }
                            }
                        }
                    }
                }
                if rows.is_empty() {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    });
                }
            }
            Some((_, selected)) if selected.binding == place.binding => {
                rows.push(FrozenAssignmentDestination {
                    selection: selection.clone(),
                    destination: destination.id.clone(),
                    point: point.id,
                    place: selected.clone(),
                    access: FrozenAssignmentAccess::Direct,
                })
            }
            _ => rows.push(FrozenAssignmentDestination {
                selection: selection.clone(),
                destination: destination.id.clone(),
                point: point.id,
                place: place.clone(),
                access: FrozenAssignmentAccess::Direct,
            }),
        },
        EvaluatedDestinationStorage::Through { pointer, .. } => {
            for origin in state.point_values[pointer.point.0].iter().flatten() {
                let loan = match &facts[origin.0] {
                    ReferenceOrigin::Fresh { loan, .. }
                    | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                    ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        });
                        continue;
                    }
                };
                let mut selected = loan.origin_place.clone();
                selected.place = place.clone();
                collect_reference_projections(place, Some(loan), &mut selected.projections);
                let cells = indexed_loan_cells(state, loan);
                if cells.is_empty() {
                    rows.push(FrozenAssignmentDestination {
                        selection: selection.clone(),
                        destination: destination.id.clone(),
                        point: point.id,
                        place: selected,
                        access: FrozenAssignmentAccess::Through {
                            pointer: pointer.clone(),
                            origin: *origin,
                            loan: loan.clone(),
                        },
                    });
                } else {
                    for (holder, guard, _, key) in cells {
                        let row = FrozenAssignmentDestination {
                            selection: selection.clone(),
                            destination: destination.id.clone(),
                            point: point.id,
                            place: selected.clone(),
                            access: FrozenAssignmentAccess::ThroughIndexed {
                                pointer: pointer.clone(),
                                origin: *origin,
                                loan: loan.clone(),
                                holder,
                                key,
                                guard,
                            },
                        };
                        if !rows.contains(&row) {
                            rows.push(row);
                        }
                    }
                }
            }
        }
    }
    for row in &rows {
        let indexed = match &row.access {
            FrozenAssignmentAccess::Indexed { holder, key, guard }
            | FrozenAssignmentAccess::ThroughIndexed {
                holder, key, guard, ..
            } => Some((holder, key, guard)),
            _ => None,
        };
        if let Some((holder, key, guard)) = indexed {
            for backing in state
                .backing_bindings
                .values_mut()
                .flatten()
                .filter(|backing| {
                    backing.value == holder.value
                        && backing.owner == holder.owner
                        && backing.length == holder.length
                })
            {
                for family in &mut backing.callable_family {
                    for partition in &mut family.partitions {
                        if compatible(partition, guard) {
                            for representative in &mut partition.representatives {
                                if representative.key == *key {
                                    let anchor =
                                        CallableIndexAnchorFact::InFlightSelection(point.id);
                                    if !representative.anchors.contains(&anchor) {
                                        representative.anchors.push(anchor);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    state.assignment_destinations.insert(point.id, rows);
    state.selected_element = None;
}

pub(super) fn compatible(
    partition: &CallableIndexedPartitionFact,
    guard: &[CallableIndexEqualityFact],
) -> bool {
    let mut refined = partition.clone();
    for relation in guard {
        if !refined.relations.contains(relation) {
            refined.relations.push(relation.clone());
        }
    }
    normalize_indexed_partition(&mut refined)
}

pub(super) fn backing_matches(
    rows: &[FrozenAssignmentDestination],
    backing: &RuntimeBackingFact,
) -> bool {
    rows.iter().any(|row| match &row.access {
        FrozenAssignmentAccess::Indexed { holder, .. }
        | FrozenAssignmentAccess::ThroughIndexed { holder, .. } => {
            backing.value == holder.value
                && backing.owner == holder.owner
                && backing.length == holder.length
        }
        _ => false,
    })
}

pub(super) fn partition_matches(
    rows: &[FrozenAssignmentDestination],
    backing: &RuntimeBackingFact,
    partition: &CallableIndexedPartitionFact,
) -> bool {
    rows.iter().any(|row| match &row.access {
        FrozenAssignmentAccess::Indexed { holder, guard, .. }
        | FrozenAssignmentAccess::ThroughIndexed { holder, guard, .. } => {
            backing.value == holder.value
                && backing.owner == holder.owner
                && backing.length == holder.length
                && compatible(partition, guard)
        }
        _ => false,
    })
}

pub(super) fn committed(
    state: &ReferenceEdgeState,
    destinations: &[EvaluatedAssignmentDestination],
    commits: &[AssignmentCommit],
    point: CfgPointId,
) -> Option<Vec<FrozenAssignmentDestination>> {
    commits
        .iter()
        .find(|commit| commit.point == point)
        .map(|commit| {
            let destination = destinations
                .iter()
                .find(|destination| destination.id == commit.destination)
                .expect("registered committed destination");
            let rows = state.assignment_destinations[&destination.point].clone();
            let selection = FrozenSelectionOccurrence {
                application: AssignmentApplicationRef {
                    frame: state.self_frame.clone(),
                    operation: commit.destination.operation.clone(),
                },
                destination: commit.destination.clone(),
            };
            validate_use(
                state,
                &FrozenGuardUse {
                    selection: selection.clone(),
                    consumer: FrozenConsumerRole::Commit(point),
                },
            )
            .expect("actual pending committed phase");
            assert!(rows.iter().all(|row| row.selection == selection
                && row.destination == commit.destination
                && row.point == destination.point));
            rows
        })
}

pub(super) fn direct_place(rows: &[FrozenAssignmentDestination]) -> &ReferencePlaceId {
    rows.iter()
        .map(|row| {
            assert!(matches!(
                row.access,
                FrozenAssignmentAccess::Direct | FrozenAssignmentAccess::Indexed { .. }
            ));
            &row.place
        })
        .reduce(|left, right| {
            assert_eq!(
                left, right,
                "same incoming direct destination across index guards"
            );
            left
        })
        .expect("actual direct destination")
}

pub(super) fn selection(
    rows: &[FrozenAssignmentDestination],
) -> Option<(CfgPointId, ReferencePlaceId)> {
    rows.iter()
        .filter_map(|row| match &row.access {
            FrozenAssignmentAccess::Indexed { holder, .. } => {
                Some((holder.index_point, row.place.clone()))
            }
            FrozenAssignmentAccess::Direct => None,
            _ => unreachable!("direct committed selection"),
        })
        .reduce(|left, right| {
            assert_eq!(left, right, "same evaluated index across frozen guards");
            left
        })
}

#[cfg(test)]
#[path = "assignment_frozen_flow_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "assignment_occurrence_phase_tests.rs"]
mod occurrence_phase_tests;
