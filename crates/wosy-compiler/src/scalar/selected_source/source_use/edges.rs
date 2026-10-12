use super::*;

fn append(
    graph: &mut SourceUseGraph,
    equation: &JointPortMonitorEquation,
    source: &SourceUseSupport,
    destination: &SourceUseSupport,
    witness: SourceUseEdgeWitness,
) {
    let edge = SourceUseEdge {
        equation: equation.clone(),
        source: source.clone(),
        destination: destination.clone(),
        witness,
    };
    if !graph.edges.contains(&edge) {
        graph.edges.push(edge);
    }
}

pub(super) fn extract(
    cfg: &ScalarReferenceCfg,
    rule: &JointProducerTupleFact,
    equation: &JointPortMonitorEquation,
    graph: &mut SourceUseGraph,
) -> Result<(), ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    let point = &cfg.initial_points[rule.event.point.0];
    let input_id = node_ref(tables, &equation.input, &point.source_span)?;
    let output_id = node_ref(tables, &equation.output, &point.source_span)?;
    let input = graph
        .nodes
        .iter()
        .find(|node| node.id == input_id)
        .ok_or_else(|| invalid(&point.source_span))?
        .clone();
    let output = graph
        .nodes
        .iter()
        .find(|node| node.id == output_id)
        .ok_or_else(|| invalid(&point.source_span))?
        .clone();
    if input.configuration != rule.input
        || output.configuration != rule.output
        || output.id.contributor != rule.id
    {
        return Err(invalid(&point.source_span));
    }
    for transport in &rule.transports {
        let source_node = match &transport.child {
            JointValueChildPort::LocalInput => input.clone(),
            JointValueChildPort::SelectedChild { event, port } => {
                let children = equation
                    .children
                    .iter()
                    .filter(|child| {
                        child.event == *event && child.ports.get(*port) == Some(&transport.source)
                    })
                    .collect::<Vec<_>>();
                let [child] = children.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                if child.monitor.owner != graph.id.owner {
                    coverage(
                        graph,
                        point,
                        SourceUseCoverageReason::ForeignChild(event.clone()),
                    );
                    continue;
                }
                let id = node_ref(tables, &child.monitor, &point.source_span)?;
                graph
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
                    .ok_or_else(|| invalid(&point.source_span))?
                    .clone()
            }
        };
        let row = &tables.source_configurations[source_node.configuration.index];
        let after = &tables.source_configurations[output.configuration.index];
        let sources = row
            .slots
            .iter()
            .filter(|slot| slot.coordinate == transport.source)
            .collect::<Vec<_>>();
        let destinations = after
            .slots
            .iter()
            .filter(|slot| slot.coordinate == transport.destination)
            .collect::<Vec<_>>();
        let ([source_slot], [destination_slot]) = (sources.as_slice(), destinations.as_slice())
        else {
            coverage(
                graph,
                point,
                SourceUseCoverageReason::Transport {
                    source: transport.source.clone(),
                    destination: transport.destination.clone(),
                },
            );
            continue;
        };
        if source_slot.availability != ValueAvailability::Available
            || destination_slot.availability != ValueAvailability::Available
        {
            coverage(
                graph,
                point,
                SourceUseCoverageReason::Transport {
                    source: transport.source.clone(),
                    destination: transport.destination.clone(),
                },
            );
            continue;
        }
        let source_selection = port_monitor::selection(
            tables,
            &source_node.id.monitor,
            &transport.source,
            &point.source_span,
        )?;
        let destination_selection = port_monitor::selection(
            tables,
            &output.id.monitor,
            &transport.destination,
            &point.source_span,
        )?;
        let physical_preserve = transport.kind == JointValueTransportKind::Preserve
            && source_slot == destination_slot
            && transport.source == transport.destination;
        if !physical_preserve && (source_selection.is_none() || destination_selection.is_none()) {
            coverage(
                graph,
                point,
                SourceUseCoverageReason::Transport {
                    source: transport.source.clone(),
                    destination: transport.destination.clone(),
                },
            );
            continue;
        }
        let prefix = if let JointValueTransportKind::Project(member) = &transport.kind {
            let view = tables
                .source_views
                .get(source_slot.view.index)
                .filter(|view| view.id == source_slot.view && view.shape == member.shape)
                .ok_or_else(|| invalid(&point.source_span))?;
            let SourcePayloadView::Aggregate(members) = &view.payload else {
                return Err(invalid(&point.source_span));
            };
            members
                .get(member.member)
                .ok_or_else(|| invalid(&point.source_span))?
                .projection
                .clone()
        } else {
            Vec::new()
        };
        for source in source_node.supports.iter().filter(|source| {
            source.coordinate == transport.source && source.projection.starts_with(&prefix)
        }) {
            if source_selection.is_none() {
                let cell = current_cell(tables, row, &transport.source).ok();
                let physical_storage = match (&source.subject, cell) {
                    (SourceUseSubject::Payload { instance, .. }, Some(cell)) => *instance == cell,
                    (
                        SourceUseSubject::Owner {
                            capability,
                            instance,
                        },
                        Some(cell),
                    ) => row.owns.iter().any(|owns| {
                        owns.payload == cell
                            && owns.capability == *capability
                            && owns.owner_instance == *instance
                            && owns.state == SourceObligationState::Outstanding
                    }),
                    _ => false,
                };
                if !physical_storage {
                    coverage(
                        graph,
                        point,
                        SourceUseCoverageReason::MissingSelection(transport.source.clone()),
                    );
                    continue;
                }
            }
            for destination in output.supports.iter().filter(|destination| {
                destination.coordinate == transport.destination
                    && destination.subject == source.subject
                    && destination.projection == source.projection[prefix.len()..]
            }) {
                if let SourceUseSubject::Grant { class, referent } = &source.subject {
                    if !row.grants.iter().any(|grant| {
                        grant.class == *class
                            && grant.referent == *referent
                            && after.grants.contains(grant)
                    }) {
                        return Err(invalid(&point.source_span));
                    }
                }
                if !physical_preserve {
                    let (Some(before), Some(after)) = (&source_selection, &destination_selection)
                    else {
                        return Err(invalid(&point.source_span));
                    };
                    if before.observation != after.observation
                        && !after.sources.contains(&before.observation)
                        && !before.sources.contains(&after.observation)
                    {
                        return Err(invalid(&point.source_span));
                    }
                }
                append(
                    graph,
                    equation,
                    source,
                    destination,
                    SourceUseEdgeWitness::Transport(transport.clone()),
                );
            }
        }
    }
    for production in &equation.productions {
        for coordinate in &production.inputs {
            let source_selection =
                port_monitor::selection(tables, &equation.input, coordinate, &point.source_span)?;
            let output_selection = port_monitor::selection(
                tables,
                &equation.output,
                &production.observation.port,
                &point.source_span,
            )?;
            let storage_source = tables.source_slots.iter().any(|slot| {
                slot.id == *coordinate && matches!(slot.role, SourceCoordinateRole::Binding(_))
            });
            if output_selection.is_none() || (!storage_source && source_selection.is_none()) {
                coverage(
                    graph,
                    point,
                    SourceUseCoverageReason::Transport {
                        source: coordinate.clone(),
                        destination: production.observation.port.clone(),
                    },
                );
                continue;
            }
            for source in input
                .supports
                .iter()
                .filter(|source| source.coordinate == *coordinate)
            {
                for destination in output.supports.iter().filter(|destination| {
                    destination.coordinate == production.observation.port
                        && destination.subject == source.subject
                }) {
                    let output_view = tables
                        .source_views
                        .get(production.observation.view.index)
                        .filter(|view| view.id == production.observation.view)
                        .ok_or_else(|| invalid(&point.source_span))?;
                    let projection_matches = match &output_view.payload {
                        SourcePayloadView::Aggregate(members) => members.iter().any(|member| {
                            destination.projection.starts_with(&member.projection)
                                && destination.projection[member.projection.len()..]
                                    == source.projection
                                && source_selection.as_ref().is_some_and(|selection| {
                                    selection.observation.case.value == member.source
                                        && selection.observation.view == member.view
                                })
                        }),
                        _ => source.projection == destination.projection,
                    };
                    if !projection_matches {
                        continue;
                    }
                    let conserved = match (&source_selection, &output_selection) {
                        (Some(source), Some(destination)) => {
                            destination.sources.contains(&source.observation)
                                || source
                                    .sources
                                    .iter()
                                    .any(|leaf| destination.sources.contains(leaf))
                        }
                        (None, Some(_)) => {
                            storage_source
                                && rule.preserved.iter().any(|instance| match &source.subject {
                                    SourceUseSubject::Payload {
                                        instance: payload, ..
                                    } => instance == payload,
                                    SourceUseSubject::Owner {
                                        instance: owner, ..
                                    } => instance == owner,
                                    SourceUseSubject::Grant { .. } => false,
                                })
                        }
                        _ => false,
                    };
                    if conserved {
                        append(
                            graph,
                            equation,
                            source,
                            destination,
                            SourceUseEdgeWitness::Production(production.clone()),
                        );
                    }
                }
            }
        }
    }
    let before = &tables.source_configurations[rule.input.index];
    let after = &tables.source_configurations[rule.output.index];
    for transition in tables
        .owner_receiving_transitions
        .iter()
        .filter(|transition| {
            transition.event == rule.event
                && transition.input == rule.input
                && transition.output == rule.output
        })
    {
        for dependency in &transition.dependencies {
            let capability = match &dependency.dependency.action {
                DependencyAction::TransferUnique {
                    source_owner,
                    receiving_owner,
                } if *receiving_owner == transition.receiving => source_owner,
                DependencyAction::BorrowFixed { .. } => continue,
                _ => return Err(invalid(&point.source_span)),
            };
            if !before.owns.iter().any(|owns| {
                owns.payload == dependency.source_instance
                    && owns.capability == *capability
                    && owns.owner_instance == dependency.owner_instance
                    && owns.state == SourceObligationState::Outstanding
            }) || !after.owns.iter().any(|owns| {
                owns.payload == dependency.source_instance
                    && owns.capability == transition.receiving
                    && owns.owner_instance == transition.receiving_instance
                    && owns.state == SourceObligationState::Outstanding
            }) {
                return Err(invalid(&point.source_span));
            }
            for source in input.supports.iter().filter(|support| {
                support.projection == dependency.projection
                    && support.subject
                        == (SourceUseSubject::Owner {
                            capability: capability.clone(),
                            instance: dependency.owner_instance.clone(),
                        })
            }) {
                let membership = port_monitor::selection(
                    tables,
                    &equation.input,
                    &source.coordinate,
                    &point.source_span,
                )?;
                let Some(membership) = membership else {
                    continue;
                };
                if membership.observation.case != dependency.case
                    || membership.observation.path != dependency.path
                {
                    continue;
                }
                for destination in output.supports.iter().filter(|support| {
                    support.coordinate == source.coordinate
                        && support.projection == source.projection
                        && support.subject
                            == (SourceUseSubject::Owner {
                                capability: transition.receiving.clone(),
                                instance: transition.receiving_instance.clone(),
                            })
                }) {
                    append(
                        graph,
                        equation,
                        source,
                        destination,
                        SourceUseEdgeWitness::Receiving(transition.clone()),
                    );
                }
            }
        }
    }
    for witness in tables.source_grant_creations.iter().filter(|witness| {
        witness.contributor == rule.id
            && witness.input == rule.input
            && witness.output == rule.output
            && witness.event == rule.event
    }) {
        let SourceGrantParent::Selected {
            class, selection, ..
        } = &witness.parent
        else {
            continue;
        };
        let membership = port_monitor::query(
            tables,
            &equation.input,
            std::slice::from_ref(&selection.coordinate),
            &point.source_span,
        )?;
        if membership.as_slice() != std::slice::from_ref(selection) {
            return Err(invalid(&point.source_span));
        }
        let parents = before
            .grants
            .iter()
            .filter(|grant| grant.class == *class)
            .collect::<Vec<_>>();
        let children = after
            .grants
            .iter()
            .filter(|grant| grant.class == witness.child)
            .collect::<Vec<_>>();
        let ([parent], [child]) = (parents.as_slice(), children.as_slice()) else {
            return Err(invalid(&point.source_span));
        };
        if parent.referent != child.referent
            || parent.coordinate != child.coordinate
            || parent.projection != child.projection
            || !after.grant_ancestry.contains(&SourceGrantAncestryFact {
                ancestor: class.clone(),
                descendant: witness.child.clone(),
            })
        {
            return Err(invalid(&point.source_span));
        }
        for source in input.supports.iter().filter(|source| {
            source.coordinate == selection.coordinate
                && source.subject
                    == (SourceUseSubject::Grant {
                        class: class.clone(),
                        referent: parent.referent.clone(),
                    })
        }) {
            for destination in output.supports.iter().filter(|destination| {
                destination.subject
                    == (SourceUseSubject::Grant {
                        class: witness.child.clone(),
                        referent: child.referent.clone(),
                    })
            }) {
                append(
                    graph,
                    equation,
                    source,
                    destination,
                    SourceUseEdgeWitness::Parent(witness.clone()),
                );
            }
        }
    }
    Ok(())
}
