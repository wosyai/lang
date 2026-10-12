use super::*;

#[derive(Clone)]
pub(super) struct AssignmentSavedFlowPayload {
    pub(super) selected: Vec<SourceSavedPortFact>,
    pub(super) selection: FrozenSelectionOccurrence,
    source: ReferencePlaceId,
    holder: ReferenceBindingId,
    checked: BTreeSet<ReferenceOriginId>,
    projected_checked: Vec<ReferenceProjectedState>,
    environments: BTreeSet<CallableEnvironmentId>,
    projected_environments: Vec<CallableProjectedEnvironment>,
    relations: Vec<CallableHolderRelation>,
    sources: Vec<CallableHolderPlace>,
    producers: Vec<ConnectedCaptureHolderFact>,
    backings: Vec<RuntimeBackingFact>,
    projected_backings: Vec<RuntimeProjectedBackingFact>,
    frozen_backings: Vec<RuntimeBackingFact>,
    uninitialized: HashSet<ReferenceBindingId>,
    moved: Vec<(ReferencePlaceId, SourceSpan)>,
    released: BTreeSet<ReferenceBindingId>,
    invalidated: BTreeSet<ReferenceBindingId>,
}

impl PartialEq for AssignmentSavedFlowPayload {
    fn eq(&self, other: &Self) -> bool {
        self.selection == other.selection
            && self.source == other.source
            && self.holder == other.holder
            && self.checked == other.checked
            && self.projected_checked == other.projected_checked
            && self.environments == other.environments
            && self.projected_environments == other.projected_environments
            && self.relations == other.relations
            && self.sources == other.sources
            && self.producers == other.producers
            && self.backings == other.backings
            && self.projected_backings == other.projected_backings
            && self.frozen_backings == other.frozen_backings
            && self.uninitialized == other.uninitialized
            && self.moved == other.moved
            && self.released == other.released
            && self.invalidated == other.invalidated
    }
}

pub(super) fn attach_selected(
    state: &mut ReferenceEdgeState,
    point: &ReferenceFlowPoint,
    selected: &selected_source::SelectedSourceState,
    tables: &EvaluatedValueTables,
) -> Result<(), ReferenceAnalysisError> {
    if !matches!(point.kind, CfgPointKind::AssignmentSnapshot { .. }) {
        return Ok(());
    }
    let selected_source::SelectedSourceState::Active { monitor, .. } = selected else {
        return Ok(());
    };
    let rows = state.assignment_saved.get_mut(&point.id).ok_or_else(|| {
        ReferenceAnalysisError::MissingContext {
            source_span: point.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        }
    })?;
    for row in rows {
        row.selected = tables
            .source_saved_ports
            .iter()
            .filter(|saved| {
                saved.snapshot.point == point.id
                    && saved.selection == row.selection
                    && saved.monitor == *monitor
                    && saved.source_place == row.source
            })
            .cloned()
            .collect();
    }
    Ok(())
}

fn rebase(
    source: &ReferencePlaceId,
    holder: &ReferenceBindingId,
    place: &ReferencePlaceId,
) -> ReferencePlaceId {
    ReferencePlaceId {
        source: holder.source.clone(),
        function_span: holder.function_span,
        declaration_span: holder.declaration_span,
        block_span: holder.block_span,
        binding: holder.clone(),
        projections: place.projections[source.projections.len()..].to_vec(),
        place: place.place.clone(),
    }
}

fn selected_holder(source: &ReferencePlaceId, holder: &CallableHolderPlace) -> bool {
    match holder {
        CallableHolderPlace::Binding(binding) => {
            source.projections.is_empty() && source.binding == *binding
        }
        CallableHolderPlace::Projection(place) => storage_contains(source, &available_place(place)),
        _ => false,
    }
}

fn saved_holder(
    source: &ReferencePlaceId,
    holder: &ReferenceBindingId,
    original: &CallableHolderPlace,
) -> CallableHolderPlace {
    match original {
        CallableHolderPlace::Binding(_) => CallableHolderPlace::Binding(holder.clone()),
        CallableHolderPlace::Projection(place) => {
            let place = rebase(source, holder, &available_place(place));
            if place.projections.is_empty() {
                CallableHolderPlace::Binding(holder.clone())
            } else {
                CallableHolderPlace::Projection(place)
            }
        }
        _ => unreachable!("selected storage holder"),
    }
}

pub(super) fn capture(
    state: &mut ReferenceEdgeState,
    point: &ReferenceFlowPoint,
    frozen_context: &[FrozenAssignmentDestination],
    phase: &assignment_frozen_flow::FrozenPhaseContext,
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    let CfgPointKind::AssignmentSnapshot {
        destination_point,
        holder,
        observed,
    } = &point.kind
    else {
        return;
    };
    assert_eq!(phase.destination_point, *destination_point);
    assert!(matches!(&phase.usage.consumer, FrozenConsumerRole::Snapshot(id) if *id == point.id));
    let mut rows = Vec::new();
    for frozen in frozen_context {
        assert_eq!(frozen.point, *destination_point);
        assert_eq!(frozen.selection, phase.usage.selection);
        if unavailable_place(state, &frozen.place) {
            if *observed {
                errors.push(ReferenceAnalysisError::ConflictingPlace {
                    source_span: point.source_span.clone(),
                    origins: unavailable_origins(state, &frozen.place),
                });
            }
            continue;
        }
        let source = available_place(&frozen.place);
        let mut checked = BTreeSet::new();
        let mut environments = BTreeSet::new();
        let mut backings = Vec::new();
        if source.projections.is_empty() {
            checked.extend(state.bindings.get(&source.binding).into_iter().flatten());
            environments.extend(
                state
                    .callable_bindings
                    .get(&source.binding)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            backings.extend(
                state
                    .backing_bindings
                    .get(&source.binding)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        let mut projected_checked = Vec::new();
        for known in state
            .projected
            .iter()
            .filter(|known| storage_contains(&source, &known.place))
        {
            if reference_places_same(&source, &known.place) {
                checked.extend(&known.origins);
            } else {
                let mut saved = known.clone();
                saved.place = rebase(&source, holder, &known.place);
                projected_checked.push(saved);
            }
        }
        let mut projected_environments = Vec::new();
        for known in state
            .callable_projected
            .iter()
            .filter(|known| storage_contains(&source, &known.place))
        {
            if reference_places_same(&source, &known.place) {
                environments.extend(known.environments.iter().cloned());
            } else {
                projected_environments.push(CallableProjectedEnvironment {
                    place: rebase(&source, holder, &known.place),
                    environments: known.environments.clone(),
                });
            }
        }
        let mut projected_backings = Vec::new();
        for known in state
            .backing_projected
            .iter()
            .filter(|known| storage_contains(&source, &known.place))
        {
            if reference_places_same(&source, &known.place) {
                backings.push(known.backing.clone());
            } else {
                projected_backings.push(RuntimeProjectedBackingFact {
                    place: rebase(&source, holder, &known.place),
                    backing: known.backing.clone(),
                });
            }
        }
        let mut relations = Vec::new();
        let mut sources = Vec::new();
        let mut producers = Vec::new();
        for relation in &state.holder_relations {
            let mut saved = relation.clone();
            saved.holders.clear();
            for original in relation
                .holders
                .iter()
                .filter(|original| selected_holder(&source, original))
            {
                let Some(producer) = state
                    .holder_provenance
                    .lookup(&relation.environment, original)
                else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                    });
                    continue;
                };
                let target = saved_holder(&source, holder, original);
                if !sources.contains(&target) {
                    sources.push(target.clone());
                }
                saved.holders.push(target.clone());
                producers.push(ConnectedCaptureHolderFact {
                    holder: target,
                    environment: relation.environment.clone(),
                    producer: producer.clone(),
                });
            }
            if !saved.holders.is_empty() {
                relations.push(saved);
            }
        }
        rows.push(AssignmentSavedFlowPayload {
            selected: Vec::new(),
            selection: frozen.selection.clone(),
            source: frozen.place.clone(),
            holder: holder.clone(),
            checked,
            projected_checked,
            environments,
            projected_environments,
            relations,
            sources,
            producers,
            backings,
            projected_backings,
            frozen_backings: state
                .backing_bindings
                .get(&source.binding)
                .into_iter()
                .flatten()
                .cloned()
                .collect(),
            uninitialized: state
                .uninitialized
                .iter()
                .filter(|binding| **binding == source.binding)
                .cloned()
                .collect(),
            moved: state
                .moved
                .iter()
                .filter(|(place, _)| storage_contains(&source, &available_place(place)))
                .cloned()
                .collect(),
            released: state.released.clone(),
            invalidated: state.invalidated.clone(),
        });
    }
    for row in &rows {
        state
            .bindings
            .entry(row.holder.clone())
            .or_default()
            .extend(&row.checked);
        state.projected.extend(row.projected_checked.clone());
        state
            .callable_bindings
            .entry(row.holder.clone())
            .or_default()
            .extend(row.environments.iter().cloned());
        state
            .callable_projected
            .extend(row.projected_environments.clone());
        state
            .backing_bindings
            .entry(row.holder.clone())
            .or_default()
            .extend(row.backings.clone());
        state
            .backing_projected
            .extend(row.projected_backings.clone());
        state.holder_relations.extend(row.relations.clone());
        for producer in &row.producers {
            state.holder_provenance.insert(
                producer.environment.clone(),
                producer.holder.clone(),
                producer.producer.clone(),
            );
        }
    }
    normalize_holder_relations(&mut state.holder_relations);
    state.assignment_saved.insert(point.id, rows);
}

fn snapshot(points: &[ReferenceFlowPoint], point: CfgPointId) -> CfgPointId {
    match points[point.0].kind {
        CfgPointKind::AssignmentSnapshot { .. } => point,
        CfgPointKind::AssignmentResult { snapshot_point, .. } => snapshot_point,
        _ => unreachable!("actual saved value producer"),
    }
}

pub(super) fn root_holder(state: &ReferenceEdgeState, point: CfgPointId) -> CallableHolderPlace {
    let sources = state.holder_point_sources[point.0]
        .iter()
        .filter(|holder| matches!(holder, CallableHolderPlace::Binding(_)))
        .collect::<Vec<_>>();
    let [source] = sources.as_slice() else {
        panic!("unique actual saved callable root")
    };
    (*source).clone()
}

pub(super) fn checked(
    state: &ReferenceEdgeState,
    snapshot: CfgPointId,
) -> BTreeSet<ReferenceOriginId> {
    state.assignment_saved[&snapshot]
        .iter()
        .flat_map(|row| row.checked.iter().copied())
        .collect()
}

pub(super) fn projected_backings(
    state: &ReferenceEdgeState,
    points: &[ReferenceFlowPoint],
    point: CfgPointId,
) -> Vec<RuntimeProjectedBackingFact> {
    state.assignment_saved[&snapshot(points, point)]
        .iter()
        .flat_map(|row| row.projected_backings.clone())
        .collect()
}

pub(super) fn publish(
    state: &mut ReferenceEdgeState,
    points: &[ReferenceFlowPoint],
    point: &mut ReferenceFlowPoint,
    phase: &assignment_frozen_flow::FrozenPhaseContext,
) {
    assert!(match &phase.usage.consumer {
        FrozenConsumerRole::Snapshot(id) => *id == point.id,
        FrozenConsumerRole::Result(value) => value.point == point.id,
        _ => false,
    });
    let rows = &state.assignment_saved[&snapshot(points, point.id)];
    assert!(rows
        .iter()
        .all(|row| row.selection == phase.usage.selection));
    point.possible_environments = rows
        .iter()
        .flat_map(|row| row.environments.iter().cloned())
        .collect();
    point.possible_projected_environments = rows
        .iter()
        .flat_map(|row| row.projected_environments.clone())
        .collect();
    point.possible_backings = rows.iter().flat_map(|row| row.backings.clone()).collect();
    point.possible_projected_backings = rows
        .iter()
        .flat_map(|row| row.projected_backings.clone())
        .collect();
    state.callable_point_projected[point.id.0] = point.possible_projected_environments.clone();
    state.callable_value_projected = point.possible_projected_environments.clone();
    state.holder_sources = rows.iter().flat_map(|row| row.sources.clone()).collect();
    state.holder_value = state
        .holder_relations
        .iter()
        .filter(|relation| {
            relation
                .holders
                .iter()
                .any(|holder| state.holder_sources.contains(holder))
        })
        .cloned()
        .collect();
}

pub(super) fn dispose(
    state: &mut ReferenceEdgeState,
    snapshot: CfgPointId,
    phase: &assignment_frozen_flow::FrozenPhaseContext,
) {
    assert!(matches!(
        phase.usage.consumer,
        FrozenConsumerRole::Dispose(_)
    ));
    let rows = state
        .assignment_saved
        .remove(&snapshot)
        .expect("actual saved disposition");
    assert!(rows
        .iter()
        .all(|row| row.selection == phase.usage.selection));
    for row in rows {
        state.bindings.remove(&row.holder);
        state.callable_bindings.remove(&row.holder);
        state.backing_bindings.remove(&row.holder);
        state
            .projected
            .retain(|known| known.place.binding != row.holder);
        state
            .callable_projected
            .retain(|known| known.place.binding != row.holder);
        state
            .backing_projected
            .retain(|known| known.place.binding != row.holder);
        for relation in &mut state.holder_relations {
            relation
                .holders
                .retain(|holder| !row.sources.contains(holder));
        }
        for producer in row.producers {
            state
                .holder_provenance
                .remove(&producer.environment, &producer.holder);
        }
    }
    state
        .holder_relations
        .retain(|relation| !relation.holders.is_empty());
}

#[cfg(test)]
#[path = "assignment_saved_flow_tests.rs"]
mod tests;
