use super::reference::{holder_place_same, producer_cfg_source, HolderProvenance};
use super::*;

#[path = "capture_input_tuple.rs"]
mod capture_input_tuple;

pub(super) struct PendingCapturePath {
    pub(super) capture: CfgPointId,
    pub(super) span: SourceSpan,
    pub(super) captures: Vec<closure::ClosureCapture>,
    pub(super) path: Vec<CallableCaptureInput>,
    pub(super) provenance: HolderProvenance,
}

pub(super) struct PendingReturnCase {
    pub(super) return_point: CfgPointId,
    pub(super) return_span: SourceSpan,
    pub(super) output: usize,
    pub(super) holder: CallableHolderPlace,
    pub(super) environment: CallableEnvironmentId,
    pub(super) transfer: CallableHolderAction,
    pub(super) producer: HolderProducerRef,
}

pub(super) struct PendingReturnGap {
    pub(super) return_point: CfgPointId,
    pub(super) return_span: SourceSpan,
    pub(super) environment: CallableEnvironmentId,
}

pub(super) fn return_environment_from_inputs(
    cfg: &ScalarReferenceCfg,
    environment: &CallableEnvironmentId,
) -> bool {
    cfg.inputs.iter().any(|input| {
        input.environments.contains(environment)
            || input
                .projected
                .iter()
                .any(|projection| projection.environments.contains(environment))
    })
}

pub(super) fn validated_entry_producer(
    facts: &[CaptureInputPathFact],
    binding: &ReferenceBindingId,
    holder: &CallableHolderPlace,
    environment: &CallableEnvironmentId,
) -> Option<HolderProducerRef> {
    facts
        .iter()
        .flat_map(|fact| &fact.inputs)
        .filter(|input| input.binding == *binding)
        .filter_map(|input| match &input.origin {
            CaptureInputOriginFact::Callable { holders } => Some(holders),
            CaptureInputOriginFact::Value => None,
        })
        .flatten()
        .find(|known| known.environment == *environment && holder_place_same(&known.holder, holder))
        .map(|known| known.producer.clone())
}

pub(super) fn record_holder_creation_roots(
    point: &ReferenceFlowPoint,
    id: CfgPointId,
    relations: &[CallableHolderRelation],
    provenance: &mut HolderProvenance,
) {
    let created = relations
        .iter()
        .flat_map(|relation| {
            relation.holders.iter().filter_map(|holder| match holder {
                CallableHolderPlace::Temporary(created) if *created == id => {
                    Some((relation.environment.clone(), holder.clone()))
                }
                CallableHolderPlace::Projection(place)
                    if matches!(place.binding.kind, ReferenceBindingKind::CallOutput { point: created, .. } if created == id) =>
                {
                    Some((relation.environment.clone(), holder.clone()))
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    if created.is_empty() {
        return;
    }
    match &point.kind {
        CfgPointKind::Capture { .. } => {
            for (environment, holder) in created {
                if !matches!(holder, CallableHolderPlace::Temporary(_)) {
                    continue;
                }
                provenance.insert(
                    environment.clone(),
                    holder,
                    HolderProducerRef::CaptureCreation {
                        point: id,
                        environment,
                    },
                );
            }
        }
        CfgPointKind::Call { .. } => {
            for (environment, holder) in created {
                let projection = match &holder {
                    CallableHolderPlace::Temporary(_) => Vec::new(),
                    CallableHolderPlace::Projection(place) => place.projections.clone(),
                    _ => continue,
                };
                let Some(selection) = point.call_import_paths.iter().flatten().find(|selected| {
                    selected.call == id
                        && selected.destination == environment
                        && selected.projection == projection
                }) else {
                    continue;
                };
                provenance.insert(
                    environment.clone(),
                    holder,
                    HolderProducerRef::CallCreation {
                        point: id,
                        environment,
                        selection: selection.clone(),
                    },
                );
            }
        }
        _ => {}
    }
}

pub(super) fn build_capture_input_path_fact(
    owner: &ProducerCfgOwnerId,
    capture: CfgPointId,
    capture_span: &SourceSpan,
    captures: &[closure::ClosureCapture],
    path: &[CallableCaptureInput],
    provenance: &HolderProvenance,
) -> Result<CaptureInputPathFact, ReferenceAnalysisError> {
    let tuple =
        capture_input_tuple::CaptureInputTuple::new(owner, capture, capture_span, captures, path)
            .map_err(|_| ReferenceAnalysisError::MissingContext {
            source_span: capture_span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        })?;
    let mut inputs = Vec::new();
    for pair in tuple.pairs() {
        let capture = pair.declaration;
        let snapshot = pair.snapshot;
        let mut holders = Vec::new();
        for environment in &snapshot.environments {
            let holder = CallableHolderPlace::Binding(capture.binding.clone());
            let Some(producer) = provenance.lookup(environment, &holder) else {
                return Err(ReferenceAnalysisError::MissingContext {
                    source_span: capture.use_span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                });
            };
            holders.push(ConnectedCaptureHolderFact {
                holder,
                environment: environment.clone(),
                producer: producer.clone(),
            });
        }
        for projected in &snapshot.projected {
            for environment in &projected.environments {
                let holder = CallableHolderPlace::Projection(projected.place.clone());
                let Some(producer) = provenance.lookup(environment, &holder) else {
                    return Err(ReferenceAnalysisError::MissingContext {
                        source_span: capture.use_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                    });
                };
                holders.push(ConnectedCaptureHolderFact {
                    holder,
                    environment: environment.clone(),
                    producer: producer.clone(),
                });
            }
        }
        inputs.push(CaptureInputFact {
            binding: capture.binding.clone(),
            origin: if holders.is_empty() {
                CaptureInputOriginFact::Value
            } else {
                CaptureInputOriginFact::Callable { holders }
            },
        });
    }
    Ok(CaptureInputPathFact {
        capture,
        capture_span: capture_span.clone(),
        owner: owner.clone(),
        inputs,
    })
}

fn edge_holds(
    edge: &CallableHolderEdgeFact,
    environment: &CallableEnvironmentId,
    holder: &CallableHolderPlace,
) -> bool {
    edge.after.iter().any(|relation| {
        relation.environment == *environment
            && relation
                .holders
                .iter()
                .any(|known| holder_place_same(known, holder))
    })
}

fn edge_held_before(
    edge: &CallableHolderEdgeFact,
    environment: &CallableEnvironmentId,
    holder: &CallableHolderPlace,
) -> bool {
    edge.before.iter().any(|relation| {
        relation.environment == *environment
            && relation
                .holders
                .iter()
                .any(|known| holder_place_same(known, holder))
    })
}

fn match_producer_link<'a>(
    cfg: &'a ScalarReferenceCfg,
    producer: &'a HolderProducerRef,
) -> Result<Option<&'a HolderProducerRef>, ()> {
    match producer {
        HolderProducerRef::CaptureCreation { point, environment } => {
            let Some(known) = cfg.points.get(point.0) else {
                return Err(());
            };
            if !matches!(&known.kind, CfgPointKind::Capture { .. }) {
                return Err(());
            }
            let created = CallableHolderPlace::Temporary(*point);
            let found = cfg
                .holder_edges
                .iter()
                .any(|edge| edge.from == *point && edge_holds(edge, environment, &created));
            if found {
                Ok(None)
            } else {
                Err(())
            }
        }
        HolderProducerRef::CallCreation {
            point,
            environment,
            selection,
        } => {
            let Some(known) = cfg.points.get(point.0) else {
                return Err(());
            };
            let CfgPointKind::Call { target, output, .. } = &known.kind else {
                return Err(());
            };
            if selection.call != *point
                || selection.target != *target
                || selection.output != *output
                || selection.destination != *environment
            {
                return Err(());
            }
            if !known
                .call_import_paths
                .iter()
                .flatten()
                .any(|selected| selected == selection)
            {
                return Err(());
            }
            if !cfg.environment_imports.iter().any(|entry| {
                entry.table == selection.source.table
                    && entry.source_index == selection.source.alternative.source_index
                    && entry.source_instance == selection.source.alternative.source_instance
                    && entry.destination == *environment
            }) {
                return Err(());
            }
            let direct = CallableHolderPlace::Temporary(*point);
            let found = cfg.holder_edges.iter().any(|edge| {
                edge.from == *point
                    && edge.after.iter().any(|relation| {
                        relation.environment == *environment
                            && (relation.holders.iter().any(|holder| holder_place_same(holder, &direct))
                                || relation.holders.iter().any(|holder| match holder {
                                    CallableHolderPlace::Projection(place)
                                        if matches!(place.binding.kind, ReferenceBindingKind::CallOutput { point: created, .. } if created == *point) =>
                                    {
                                        selection.projection == place.projections
                                    }
                                    _ => false,
                                }))
                    })
            });
            if found {
                Ok(None)
            } else {
                Err(())
            }
        }
        HolderProducerRef::InstalledRead {
            point,
            environment,
            action,
        } => {
            let CallableHolderAction::RetainCheckedSelected { to, .. } = action else {
                return Err(());
            };
            let found = cfg.holder_edges.iter().any(|edge| {
                edge.from == *point
                    && edge.actions.contains(action)
                    && edge_holds(edge, environment, to)
            });
            if found {
                Ok(None)
            } else {
                Err(())
            }
        }
        HolderProducerRef::Forward {
            point,
            environment,
            action,
            previous,
        } => {
            let (from, to) = match action {
                CallableHolderAction::Retain { from, to, .. }
                | CallableHolderAction::Move { from, to, .. } => (Some(from), Some(to)),
                CallableHolderAction::RetainSelected { from, .. }
                | CallableHolderAction::MoveSelected { from, .. } => (Some(from), None),
                _ => (None, None),
            };
            let (Some(from), _) = (from, to) else {
                return Err(());
            };
            let held_after = match to {
                Some(to) => cfg.holder_edges.iter().any(|edge| {
                    edge.from == *point
                        && edge.actions.contains(action)
                        && edge_held_before(edge, environment, from)
                        && edge_holds(edge, environment, to)
                }),
                None => cfg.holder_edges.iter().any(|edge| {
                    edge.from == *point
                        && edge.actions.contains(action)
                        && edge_held_before(edge, environment, from)
                }),
            };
            if held_after {
                Ok(Some(previous))
            } else {
                Err(())
            }
        }
        HolderProducerRef::GlobalCallsiteInput {
            call,
            holder,
            environment,
        } => {
            let found = cfg.global_callable_input_paths.iter().any(|path| {
                path.call == *call
                    && path.holders.iter().any(|fact| {
                        holder_place_same(&fact.holder, holder)
                            && fact.relations.iter().any(|relation| {
                                relation.environment == *environment
                                    && relation
                                        .holders
                                        .iter()
                                        .any(|known| holder_place_same(known, holder))
                            })
                    })
            });
            if found
                && cfg
                    .environments
                    .get(environment.index)
                    .is_some_and(|fact| fact.literal == environment.literal)
            {
                Ok(None)
            } else {
                Err(())
            }
        }
    }
}

fn validate_holder_producer_chain(
    cfg: &ScalarReferenceCfg,
    producer: &HolderProducerRef,
) -> Result<(), ()> {
    let mut current = producer;
    loop {
        match match_producer_link(cfg, current) {
            Ok(next) => match next {
                Some(next) => current = next,
                None => return Ok(()),
            },
            Err(()) => {
                let trusted = cfg
                    .capture_input_path_facts
                    .iter()
                    .filter(|fact| fact.owner != cfg.producer_owner)
                    .flat_map(|fact| &fact.inputs)
                    .filter_map(|input| match &input.origin {
                        CaptureInputOriginFact::Callable { holders } => Some(holders),
                        CaptureInputOriginFact::Value => None,
                    })
                    .flatten()
                    .any(|known| known.producer == *current);
                return if trusted { Ok(()) } else { Err(()) };
            }
        }
    }
}

pub(super) fn validate_capture_input_path_fact(
    cfg: &ScalarReferenceCfg,
    fact: &CaptureInputPathFact,
) -> Result<(), ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: fact.capture_span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    if fact.owner != cfg.producer_owner {
        return Err(invalid());
    }
    let Some(point) = cfg.points.get(fact.capture.0) else {
        return Err(invalid());
    };
    if !matches!(&point.kind, CfgPointKind::Capture { .. }) {
        return Err(invalid());
    }
    for input in &fact.inputs {
        let CaptureInputOriginFact::Callable { holders } = &input.origin else {
            continue;
        };
        if holders.is_empty() {
            return Err(invalid());
        }
        for holder in holders {
            if validate_holder_producer_chain(cfg, &holder.producer).is_err() {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn return_transfer_connected(cfg: &ScalarReferenceCfg, fact: &ProducerReturnCaseFact) -> bool {
    let CallableHolderAction::ReturnTransfer {
        environment, to, ..
    } = &fact.transfer
    else {
        return false;
    };
    if *environment != fact.environment {
        return false;
    }
    let holds_output = |relations: &[CallableHolderRelation]| {
        relations.iter().any(|relation| {
            relation.environment == fact.environment
                && relation.holders.iter().any(|holder| holder == to)
        })
    };
    cfg.holder_edges
        .iter()
        .filter(|edge| edge.from == fact.return_point && edge.actions.contains(&fact.transfer))
        .any(|edge| holds_output(&edge.after))
        || cfg
            .holder_exits
            .iter()
            .filter(|exit| exit.from == fact.return_point && exit.actions.contains(&fact.transfer))
            .any(|exit| holds_output(&exit.after))
}

fn installed_case_indexes_valid(cfg: &ScalarReferenceCfg, producer: &HolderProducerRef) -> bool {
    let mut current = Some(producer);
    while let Some(link) = current {
        match link {
            HolderProducerRef::CaptureCreation { point, environment } => {
                let Some(known) = cfg.points.get(point.0) else {
                    return false;
                };
                if !matches!(&known.kind, CfgPointKind::Capture { .. }) {
                    return false;
                }
                if cfg
                    .environments
                    .get(environment.index)
                    .is_none_or(|fact| fact.literal != environment.literal)
                {
                    return false;
                }
                return true;
            }
            HolderProducerRef::CallCreation {
                point,
                environment,
                selection,
            } => {
                let Some(known) = cfg.points.get(point.0) else {
                    return false;
                };
                if !matches!(&known.kind, CfgPointKind::Call { .. }) {
                    return false;
                }
                if selection.call != *point || selection.destination != *environment {
                    return false;
                }
                if cfg
                    .environments
                    .get(environment.index)
                    .is_none_or(|fact| fact.literal != environment.literal)
                {
                    return false;
                }
                return true;
            }
            HolderProducerRef::InstalledRead {
                point,
                environment,
                action,
            } => {
                let Some(known) = cfg.points.get(point.0) else {
                    return false;
                };
                if !matches!(&known.kind, CfgPointKind::ReadPlace { .. }) {
                    return false;
                }
                if cfg
                    .environments
                    .get(environment.index)
                    .is_none_or(|fact| fact.literal != environment.literal)
                {
                    return false;
                }
                let CallableHolderAction::RetainCheckedSelected {
                    old,
                    guard: _,
                    to: _,
                    ..
                } = action
                else {
                    return false;
                };
                let CallableCellCaseFact::Initialized {
                    environment_origin,
                    backing: _,
                    owner: _,
                    capture_paths: _,
                    loans,
                    ..
                } = old
                else {
                    return true;
                };
                if cfg
                    .environments
                    .get(environment_origin.index)
                    .is_none_or(|fact| fact.literal != environment_origin.literal)
                {
                    return false;
                }
                if loans
                    .iter()
                    .any(|loan| cfg.facts.get(loan.origin.0).is_none())
                {
                    return false;
                }
                return true;
            }
            HolderProducerRef::GlobalCallsiteInput {
                call,
                holder,
                environment,
            } => {
                if cfg
                    .environments
                    .get(environment.index)
                    .is_none_or(|fact| fact.literal != environment.literal)
                {
                    return false;
                }
                return cfg.global_callable_input_paths.iter().any(|path| {
                    path.call == *call
                        && path.holders.iter().any(|fact| {
                            holder_place_same(&fact.holder, holder)
                                && fact
                                    .relations
                                    .iter()
                                    .any(|relation| relation.environment == *environment)
                        })
                });
            }
            HolderProducerRef::Forward { previous, .. } => {
                current = Some(previous);
            }
        }
    }
    false
}

fn installed_selection_refuted(action: &CallableHolderAction) -> bool {
    let CallableHolderAction::RetainCheckedSelected {
        holder, old, guard, ..
    } = action
    else {
        return false;
    };
    let CallableCellCaseFact::Initialized { holder_aliases, .. } = old else {
        return false;
    };
    if holder_aliases.is_empty() {
        return false;
    }
    let Some(ReferencePlaceProjection::Index(selected)) = holder.place.projections.last() else {
        return false;
    };
    let ScalarExpression::Integer {
        value: selected, ..
    } = selected
    else {
        return false;
    };
    holder_aliases.iter().all(|alias| {
        let CallableHolderPlace::Projection(place) = alias else {
            return false;
        };
        let Some(ReferencePlaceProjection::Index(candidate)) = place.projections.last() else {
            return false;
        };
        let ScalarExpression::Integer { value: candidate, .. } = candidate else {
            return false;
        };
        if candidate == selected {
            return false;
        }
        guard.iter().any(|equality| {
            equality.relation == CallableArrayIndexRelation::Distinct
                && ((matches!(&equality.left, CallableIndexKeyFact::Constant(left) if left == selected)
                    && matches!(&equality.right, CallableIndexKeyFact::Constant(right) if right == candidate))
                    || (matches!(&equality.right, CallableIndexKeyFact::Constant(right) if right == selected)
                        && matches!(&equality.left, CallableIndexKeyFact::Constant(left) if left == candidate)))
        })
    })
}

pub(super) fn return_case_selection_refuted(producer: &HolderProducerRef) -> bool {
    let mut current = Some(producer);
    while let Some(link) = current {
        match link {
            HolderProducerRef::InstalledRead { action, .. } => {
                return installed_selection_refuted(action);
            }
            HolderProducerRef::Forward { previous, .. } => {
                current = Some(previous);
            }
            HolderProducerRef::CaptureCreation { .. }
            | HolderProducerRef::CallCreation { .. }
            | HolderProducerRef::GlobalCallsiteInput { .. } => {
                return false;
            }
        }
    }
    false
}

pub(super) fn validate_producer_return_case_fact(
    cfg: &ScalarReferenceCfg,
    fact: &ProducerReturnCaseFact,
) -> Result<(), ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: cfg
            .points
            .get(fact.return_point.0)
            .map(|point| point.source_span.clone())
            .unwrap_or_else(|| {
                SourceSpan::new(
                    producer_cfg_source(&cfg.producer_owner).clone(),
                    ByteSpan::new(0, 0),
                )
            }),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    if fact.table.owner != cfg.producer_owner {
        return Err(invalid());
    }
    let Some(point) = cfg.points.get(fact.return_point.0) else {
        return Err(invalid());
    };
    let CfgPointKind::ReturnOutput { output, .. } = &point.kind else {
        return Err(invalid());
    };
    if *output != fact.output {
        return Err(invalid());
    }
    let Some(environment) = cfg.environments.get(fact.environment.index) else {
        return Err(invalid());
    };
    if environment.literal != fact.environment.literal {
        return Err(invalid());
    }
    if fact.alternative.source_index != fact.environment.index {
        return Err(invalid());
    }
    if environment.instance != fact.alternative.source_instance {
        return Err(invalid());
    }
    if validate_holder_producer_chain(cfg, &fact.producer).is_err() {
        return Err(invalid());
    }
    if return_case_selection_refuted(&fact.producer) {
        return Err(invalid());
    }
    if !installed_case_indexes_valid(cfg, &fact.producer) {
        return Err(invalid());
    }
    if !return_transfer_connected(cfg, fact) {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn build_producer_return_case_fact(
    owner: &ProducerCfgOwnerId,
    pending: &PendingReturnCase,
    instance: &CallableEnvironmentInstance,
) -> ProducerReturnCaseFact {
    ProducerReturnCaseFact {
        table: ProducerTableId {
            owner: owner.clone(),
        },
        return_point: pending.return_point,
        output: pending.output,
        alternative: ProducerOutputAlternativeId {
            source_index: pending.environment.index,
            source_instance: instance.clone(),
        },
        holder: pending.holder.clone(),
        environment: pending.environment.clone(),
        transfer: pending.transfer.clone(),
        producer: pending.producer.clone(),
    }
}
