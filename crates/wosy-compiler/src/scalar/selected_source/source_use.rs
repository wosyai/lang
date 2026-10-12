use super::*;
mod schema;
pub use schema::*;
mod edges;
mod inputs;

fn coverage(
    graph: &mut SourceUseGraph,
    point: &ReferenceFlowPoint,
    reason: SourceUseCoverageReason,
) {
    let row = SourceUseCoverage {
        event: SourceEventId {
            owner: graph.id.owner.clone(),
            point: point.id,
        },
        source_span: point.source_span.clone(),
        reason,
    };
    if !graph.coverage.contains(&row) {
        graph.coverage.push(row);
    }
}

fn node_ref(
    tables: &EvaluatedValueTables,
    monitor: &JointPortMonitorRef,
    source: &SourceSpan,
) -> Result<SourceUseNodeRef, ReferenceAnalysisError> {
    let row = tables
        .source_port_monitors
        .get(monitor.index)
        .filter(|row| row.id == *monitor)
        .ok_or_else(|| invalid(source))?;
    Ok(SourceUseNodeRef {
        contributor: row.contributor.clone(),
        monitor: monitor.clone(),
    })
}

fn payload_subjects(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    slot: &SourceSelectedSlotFact,
    point: &ReferenceFlowPoint,
    graph: &mut SourceUseGraph,
) -> Result<Vec<(Vec<ValueProjection>, SourceUseSubject)>, ReferenceAnalysisError> {
    let mut subjects = Vec::new();
    if slot.availability != ValueAvailability::Available {
        coverage(
            graph,
            point,
            SourceUseCoverageReason::Unavailable {
                coordinate: slot.coordinate.clone(),
                state: slot.availability.clone(),
            },
        );
        return Ok(subjects);
    }
    let mut pending = vec![(Vec::new(), slot.view.clone(), Vec::new())];
    while let Some((projection, id, mut ancestors)) = pending.pop() {
        if ancestors.contains(&id) {
            coverage(graph, point, SourceUseCoverageReason::SourceRecurrence(id));
            continue;
        }
        ancestors.push(id.clone());
        let view = tables
            .source_views
            .get(id.index)
            .filter(|view| view.id == id)
            .ok_or_else(|| invalid(&point.source_span))?;
        match &view.payload {
            SourcePayloadView::Aggregate(members) => {
                for member in members.iter().rev() {
                    let mut child = projection.clone();
                    child.extend(member.projection.clone());
                    pending.push((child, member.view.clone(), ancestors.clone()));
                }
            }
            SourcePayloadView::Raw { referent, .. }
            | SourcePayloadView::Backing {
                instance: referent, ..
            } => {
                subjects.push((
                    projection.clone(),
                    SourceUseSubject::Payload {
                        instance: referent.clone(),
                        projection: Vec::new(),
                    },
                ));
            }
            SourcePayloadView::Checked { state, referents } => {
                if *state != CheckedValueState::Live {
                    coverage(
                        graph,
                        point,
                        SourceUseCoverageReason::CheckedState {
                            coordinate: slot.coordinate.clone(),
                            state: state.clone(),
                        },
                    );
                    continue;
                }
                selected_grant::validate_view(tables, row, &id, &point.source_span)?;
                if referents.is_empty() {
                    coverage(
                        graph,
                        point,
                        SourceUseCoverageReason::PendingView(id.clone()),
                    );
                }
                for referent in referents {
                    let SourceCheckedGrantState::Selected(class) = &referent.loan else {
                        coverage(
                            graph,
                            point,
                            SourceUseCoverageReason::PendingView(id.clone()),
                        );
                        continue;
                    };
                    subjects.push((
                        projection.clone(),
                        SourceUseSubject::Grant {
                            class: class.clone(),
                            referent: referent.instance.clone(),
                        },
                    ));
                    subjects.push((
                        projection.clone(),
                        SourceUseSubject::Payload {
                            instance: referent.instance.clone(),
                            projection: referent.projection.clone(),
                        },
                    ));
                }
            }
            SourcePayloadView::Formal(_) | SourcePayloadView::Unwritten => {
                coverage(graph, point, SourceUseCoverageReason::PendingView(id))
            }
            SourcePayloadView::Plain | SourcePayloadView::StaticRaw(_) => {}
        }
    }
    let payloads = subjects.clone();
    for (projection, subject) in payloads {
        if let SourceUseSubject::Payload { instance, .. } = subject {
            if !row.instances.contains(&instance) {
                return Err(invalid(&point.source_span));
            }
            if matches!(instance, SourceInstanceRef::Residual(_)) {
                coverage(
                    graph,
                    point,
                    SourceUseCoverageReason::PendingView(slot.view.clone()),
                );
            }
            for owns in row.owns.iter().filter(|owns| {
                owns.payload == instance && owns.state == SourceObligationState::Outstanding
            }) {
                let subject = (
                    projection.clone(),
                    SourceUseSubject::Owner {
                        capability: owns.capability.clone(),
                        instance: owns.owner_instance.clone(),
                    },
                );
                if !subjects.contains(&subject) {
                    subjects.push(subject);
                }
            }
        }
    }
    Ok(subjects)
}

fn make_node(
    cfg: &ScalarReferenceCfg,
    monitor: &JointPortMonitorFact,
    graph: &mut SourceUseGraph,
) -> Result<SourceUseNode, ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    let source = &cfg.initial_points[0].source_span;
    let rule = tables
        .source_contributors
        .get(monitor.contributor.index)
        .filter(|rule| rule.id == monitor.contributor)
        .ok_or_else(|| invalid(source))?;
    let point = cfg
        .initial_points
        .get(rule.event.point.0)
        .ok_or_else(|| invalid(source))?;
    let row = tables
        .source_configurations
        .get(rule.output.index)
        .filter(|row| row.id == rule.output)
        .ok_or_else(|| invalid(&point.source_span))?;
    if monitor.id.owner != rule.id.owner
        || rule.event.owner != cfg.producer_owner
        || row.frame.owner != rule.id.owner
    {
        return Err(invalid(&point.source_span));
    }
    port_monitor::query(tables, &monitor.id, &[], &point.source_span)?;
    let id = SourceUseNodeRef {
        contributor: rule.id.clone(),
        monitor: monitor.id.clone(),
    };
    let mut supports = Vec::new();
    for slot in &row.slots {
        let coordinate = tables
            .source_slots
            .get(slot.coordinate.index)
            .filter(|coordinate| coordinate.id == slot.coordinate)
            .ok_or_else(|| invalid(&point.source_span))?;
        let mut subjects = payload_subjects(tables, row, slot, point, graph)?;
        if matches!(coordinate.role, SourceCoordinateRole::Binding(_))
            && slot.availability == ValueAvailability::Available
        {
            match current_cell(tables, row, &slot.coordinate) {
                Ok(instance) => {
                    let mut pending = vec![(Vec::new(), coordinate.shape.clone(), Vec::new())];
                    while let Some((projection, shape, mut ancestors)) = pending.pop() {
                        if ancestors.contains(&shape) {
                            coverage(
                                graph,
                                point,
                                SourceUseCoverageReason::PendingView(slot.view.clone()),
                            );
                            continue;
                        }
                        ancestors.push(shape.clone());
                        subjects.push((
                            projection.clone(),
                            SourceUseSubject::Payload {
                                instance: instance.clone(),
                                projection: projection.clone(),
                            },
                        ));
                        for owns in row.owns.iter().filter(|owns| {
                            owns.payload == instance
                                && owns.state == SourceObligationState::Outstanding
                        }) {
                            let owner = (
                                projection.clone(),
                                SourceUseSubject::Owner {
                                    capability: owns.capability.clone(),
                                    instance: owns.owner_instance.clone(),
                                },
                            );
                            if !subjects.contains(&owner) {
                                subjects.push(owner);
                            }
                        }
                        let shape = tables
                            .shapes
                            .iter()
                            .find(|row| row.id == shape)
                            .ok_or_else(|| invalid(&point.source_span))?;
                        match &shape.payload {
                            ValuePayloadShape::Struct { fields, .. } => {
                                for field in fields.iter().rev() {
                                    let mut child = projection.clone();
                                    child.push(ValueProjection::Field(field.field.clone()));
                                    pending.push((child, field.shape.clone(), ancestors.clone()));
                                }
                            }
                            ValuePayloadShape::FixedArray { element, length } => {
                                for index in (0..*length).rev() {
                                    let mut child = projection.clone();
                                    child.push(ValueProjection::FixedIndex(index));
                                    pending.push((child, element.clone(), ancestors.clone()));
                                }
                            }
                            ValuePayloadShape::Callable { .. }
                            | ValuePayloadShape::Enum { .. }
                            | ValuePayloadShape::RuntimeArray { .. } => coverage(
                                graph,
                                point,
                                SourceUseCoverageReason::PendingView(slot.view.clone()),
                            ),
                            _ => {}
                        }
                    }
                }
                Err(()) => coverage(
                    graph,
                    point,
                    SourceUseCoverageReason::MissingInput(coordinate.role.clone()),
                ),
            }
        }
        for (projection, subject) in subjects {
            let support = SourceUseSupport {
                node: id.clone(),
                coordinate: slot.coordinate.clone(),
                projection,
                subject,
            };
            if !supports.contains(&support) {
                supports.push(support);
            }
        }
    }
    Ok(SourceUseNode {
        id,
        event: rule.event.clone(),
        configuration: row.id.clone(),
        frame: row.frame.clone(),
        supports,
    })
}

fn close_reachable(graph: &mut SourceUseGraph) {
    let mut worklist = std::collections::VecDeque::new();
    for (index, usage) in graph.uses.iter().enumerate() {
        graph.reachable.push(SourceReachableUse {
            support: usage.support.clone(),
            direct_use: index,
            predecessors: Vec::new(),
        });
        worklist.push_back(graph.reachable.len() - 1);
    }
    while let Some(index) = worklist.pop_front() {
        let next = graph.reachable[index].clone();
        for (edge_index, edge) in graph
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| edge.destination == next.support)
        {
            let predecessor = SourceUsePredecessor {
                edge: edge_index,
                reachable: index,
            };
            if let Some(previous) = graph.reachable.iter_mut().find(|previous| {
                previous.support == edge.source && previous.direct_use == next.direct_use
            }) {
                if !previous.predecessors.contains(&predecessor) {
                    previous.predecessors.push(predecessor);
                }
            } else {
                graph.reachable.push(SourceReachableUse {
                    support: edge.source.clone(),
                    direct_use: next.direct_use,
                    predecessors: vec![predecessor],
                });
                worklist.push_back(graph.reachable.len() - 1);
            }
        }
    }
}

pub(super) fn observe(
    graph: &SourceUseGraph,
    node: &SourceUseNodeRef,
) -> Result<SourceUseObservation, ReferenceAnalysisError> {
    if !graph.nodes.iter().any(|row| row.id == *node) {
        return Err(invalid(&graph.epoch.events[0].source_span));
    }
    Ok(SourceUseObservation {
        graph: graph.id.clone(),
        node: node.clone(),
        reachable: graph
            .reachable
            .iter()
            .enumerate()
            .filter(|(_, row)| row.support.node == *node)
            .map(|(index, _)| index)
            .collect(),
        parent_support: graph
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| {
                edge.source.node == *node && matches!(edge.witness, SourceUseEdgeWitness::Parent(_))
            })
            .map(|(index, _)| index)
            .collect(),
        coverage: (0..graph.coverage.len()).collect(),
    })
}

fn attach_returns(
    cfg: &ScalarReferenceCfg,
    graph: &mut SourceUseGraph,
) -> Result<(), ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    for draft in tables
        .pending_owner_consumers
        .iter()
        .filter(|draft| draft.candidate.consumer.producer == cfg.producer_owner)
    {
        let point = &cfg.initial_points[draft.candidate.consumer.point.0];
        let rule = tables
            .source_contributors
            .get(draft.candidate.contributor.index)
            .filter(|rule| rule.id == draft.candidate.contributor)
            .ok_or_else(|| invalid(&point.source_span))?;
        let output = tables
            .source_configurations
            .get(rule.output.index)
            .filter(|row| row.id == rule.output)
            .ok_or_else(|| invalid(&point.source_span))?;
        owner_consumer::validate_monitored_correspondence(
            tables,
            &cfg.initial_points,
            output,
            &draft.candidate,
            &draft.monitor,
        )?;
        for equation in tables
            .source_port_equations
            .iter()
            .filter(|equation| equation.contributor == rule.id && equation.output == draft.monitor)
        {
            let transitions = tables
                .owner_receiving_transitions
                .iter()
                .filter(|transition| {
                    transition.event == rule.event
                        && transition.input == rule.input
                        && transition.output == rule.output
                        && transition.receiving == draft.candidate.receiving
                        && transition.dependencies == draft.candidate.dependencies
                })
                .collect::<Vec<_>>();
            let [transition] = transitions.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            let input_node = node_ref(tables, &equation.input, &point.source_span)?;
            let input = observe(graph, &input_node)?;
            let uses = graph
                .uses
                .iter()
                .filter(|usage| {
                    usage.equation == *equation && usage.purpose == SourceUsePurpose::Return
                })
                .collect::<Vec<_>>();
            let mut subjects = Vec::new();
            for usage in uses {
                if !subjects.contains(&usage.support.subject) {
                    subjects.push(usage.support.subject.clone());
                }
            }
            let remaining = input
                .reachable
                .iter()
                .copied()
                .filter(|index| subjects.contains(&graph.reachable[*index].support.subject))
                .collect();
            graph.returns.push(SourceReturnUseObservation {
                consumer: draft.candidate.consumer.clone(),
                input,
                output: node_ref(tables, &equation.output, &point.source_span)?,
                transition: (*transition).clone(),
                subjects,
                remaining,
            });
        }
    }
    Ok(())
}

fn build(cfg: &ScalarReferenceCfg) -> Result<SourceUseGraph, ReferenceAnalysisError> {
    let source = &cfg.initial_points[0].source_span;
    let common = cfg
        .evaluated_values
        .common_loans
        .iter()
        .filter(|common| common.input.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [common] = common.as_slice() else {
        return Err(invalid(source));
    };
    common_loan::validate_epoch(cfg, common)?;
    let mut graph = SourceUseGraph {
        id: SourceUseGraphRef {
            owner: cfg.producer_owner.clone(),
        },
        epoch: common.input.clone(),
        nodes: Vec::new(),
        uses: Vec::new(),
        edges: Vec::new(),
        reachable: Vec::new(),
        coverage: Vec::new(),
        returns: Vec::new(),
    };
    for configuration in cfg.evaluated_values.source_configurations.iter()
        .filter(|configuration| configuration.id.owner == cfg.producer_owner) {
        for producer in &configuration.pending {
            let point = cfg.initial_points.get(producer.event.point.0).ok_or_else(|| invalid(source))?;
            if producer.event.owner != cfg.producer_owner || point.id != producer.event.point {
                return Err(invalid(&point.source_span));
            }
            coverage(&mut graph, point, SourceUseCoverageReason::PendingProducer {
                configuration: configuration.id.clone(),
                producer: producer.clone(),
            });
        }
    }
    for monitor in cfg
        .evaluated_values
        .source_port_monitors
        .iter()
        .filter(|monitor| monitor.id.owner == cfg.producer_owner)
    {
        let node = make_node(cfg, monitor, &mut graph)?;
        graph.nodes.push(node);
    }
    for point in &cfg.initial_points {
        let rules = cfg
            .evaluated_values
            .source_contributors
            .iter()
            .filter(|rule| rule.event.owner == cfg.producer_owner && rule.event.point == point.id)
            .collect::<Vec<_>>();
        if rules.is_empty() {
            coverage(
                &mut graph,
                point,
                SourceUseCoverageReason::MissingContributor,
            );
        }
        for rule in rules {
            let equations = cfg
                .evaluated_values
                .source_port_equations
                .iter()
                .filter(|equation| equation.contributor == rule.id)
                .collect::<Vec<_>>();
            if equations.is_empty() && rule.kind != JointProducerRuleKind::Root {
                coverage(
                    &mut graph,
                    point,
                    SourceUseCoverageReason::MissingEquation(rule.id.clone()),
                );
            }
            for equation in equations {
                inputs::extract(cfg, rule, equation, &mut graph)?;
                edges::extract(cfg, rule, equation, &mut graph)?;
            }
        }
    }
    close_reachable(&mut graph);
    attach_returns(cfg, &mut graph)?;
    Ok(graph)
}

pub(in crate::scalar) fn register(
    cfg: &mut ScalarReferenceCfg,
) -> Result<(), ReferenceAnalysisError> {
    let graph = build(cfg)?;
    let requirements = graph
        .returns
        .iter()
        .map(|row| (row.consumer.clone(), row.output.clone(), row.input.clone()))
        .collect::<Vec<_>>();
    cfg.evaluated_values
        .source_use_graphs
        .retain(|old| old.id.owner != cfg.producer_owner);
    cfg.evaluated_values.source_use_graphs.push(graph);
    for draft in cfg
        .evaluated_values
        .pending_owner_consumers
        .iter_mut()
        .filter(|draft| draft.candidate.consumer.producer == cfg.producer_owner)
    {
        draft.requirements.retain(|requirement| {
            !matches!(
                requirement,
                OwnerConsumerRequirement::SelectedRemainingUses(_)
            )
        });
        for (_, _, observation) in requirements.iter().filter(|(consumer, output, _)| {
            *consumer == draft.candidate.consumer
                && output.monitor == draft.monitor
                && output.contributor == draft.candidate.contributor
        }) {
            draft
                .requirements
                .push(OwnerConsumerRequirement::SelectedRemainingUses(
                    observation.clone(),
                ));
        }
    }
    Ok(())
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    let expected = build(cfg)?;
    let rows = cfg
        .evaluated_values
        .source_use_graphs
        .iter()
        .filter(|row| row.id == expected.id)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        return Err(invalid(&cfg.initial_points[0].source_span));
    };
    if **row != expected {
        return Err(invalid(&cfg.initial_points[0].source_span));
    }
    for draft in cfg
        .evaluated_values
        .pending_owner_consumers
        .iter()
        .filter(|draft| draft.candidate.consumer.producer == cfg.producer_owner)
    {
        let observations = expected
            .returns
            .iter()
            .filter(|row| {
                row.consumer == draft.candidate.consumer
                    && row.output.monitor == draft.monitor
                    && row.output.contributor == draft.candidate.contributor
            })
            .collect::<Vec<_>>();
        if observations.is_empty()
            || observations.iter().any(|row| {
                !draft
                    .requirements
                    .contains(&OwnerConsumerRequirement::SelectedRemainingUses(
                        row.input.clone(),
                    ))
            })
        {
            return Err(invalid(
                &cfg.initial_points[draft.candidate.consumer.point.0].source_span,
            ));
        }
    }
    Ok(())
}
