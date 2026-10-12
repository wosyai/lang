use super::*;

impl ValueSchemaRemap {
    fn use_node(&self, node: &SourceUseNodeRef) -> Result<SourceUseNodeRef, ValueSchemaRemapError> {
        Ok(SourceUseNodeRef {
            contributor: self.source_tuple(&node.contributor)?,
            monitor: self.source_monitor(&node.monitor)?,
        })
    }

    fn use_subject(
        &self,
        subject: &SourceUseSubject,
    ) -> Result<SourceUseSubject, ValueSchemaRemapError> {
        Ok(match subject {
            SourceUseSubject::Payload {
                instance,
                projection,
            } => SourceUseSubject::Payload {
                instance: self.source_instance(instance)?,
                projection: self.projections(projection)?,
            },
            SourceUseSubject::Grant { class, referent } => SourceUseSubject::Grant {
                class: self.grant_class(class)?,
                referent: self.source_instance(referent)?,
            },
            SourceUseSubject::Owner {
                capability,
                instance,
            } => SourceUseSubject::Owner {
                capability: self.capability(capability)?,
                instance: self.source_instance(instance)?,
            },
        })
    }

    fn use_support(
        &self,
        support: &SourceUseSupport,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<SourceUseSupport, ValueSchemaRemapError> {
        Ok(SourceUseSupport {
            node: self.use_node(&support.node)?,
            coordinate: self.boundary_slot(&support.coordinate, correspondence)?,
            projection: self.projections(&support.projection)?,
            subject: self.use_subject(&support.subject)?,
        })
    }

    pub(super) fn source_use_observation(
        &self,
        observation: &SourceUseObservation,
    ) -> Result<SourceUseObservation, ValueSchemaRemapError> {
        Ok(SourceUseObservation {
            graph: SourceUseGraphRef {
                owner: self.producer(&observation.graph.owner)?,
            },
            node: self.use_node(&observation.node)?,
            reachable: observation.reachable.clone(),
            parent_support: observation.parent_support.clone(),
            coverage: observation.coverage.clone(),
        })
    }

    fn use_equation(
        &self,
        tables: &EvaluatedValueTables,
        equation: &JointPortMonitorEquation,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<JointPortMonitorEquation, ValueSchemaRemapError> {
        let originals = tables
            .source_port_equations
            .iter()
            .enumerate()
            .filter(|(_, original)| **original == *equation)
            .collect::<Vec<_>>();
        let [(index, _)] = originals.as_slice() else {
            return Err(ValueSchemaRemapError::MissingProducer(
                equation.contributor.owner.clone(),
            ));
        };
        Ok(self.source_port_equations(tables, correspondence)?[*index].clone())
    }

    fn use_coverage(
        &self,
        coverage: &SourceUseCoverage,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<SourceUseCoverage, ValueSchemaRemapError> {
        let owner = &coverage.event.owner;
        let reason = match &coverage.reason {
            SourceUseCoverageReason::Operation(kind) => SourceUseCoverageReason::Operation(*kind),
            SourceUseCoverageReason::MissingContributor => {
                SourceUseCoverageReason::MissingContributor
            }
            SourceUseCoverageReason::PendingProducer { configuration, producer } => {
                SourceUseCoverageReason::PendingProducer {
                    configuration: self.source_configuration(configuration)?,
                    producer: SourcePendingProducerFact {
                        event: self.source_event(&producer.event)?,
                        outputs: producer.outputs.iter().map(|output| self.value(output)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    },
                }
            }
            SourceUseCoverageReason::MissingEquation(id) => {
                SourceUseCoverageReason::MissingEquation(self.source_tuple(id)?)
            }
            SourceUseCoverageReason::MissingInput(role) => {
                SourceUseCoverageReason::MissingInput(self.source_role(owner, role)?)
            }
            SourceUseCoverageReason::MissingSelection(slot) => {
                SourceUseCoverageReason::MissingSelection(self.boundary_slot(slot, correspondence)?)
            }
            SourceUseCoverageReason::Unavailable { coordinate, state } => {
                SourceUseCoverageReason::Unavailable {
                    coordinate: self.boundary_slot(coordinate, correspondence)?,
                    state: self.availability(owner, state)?,
                }
            }
            SourceUseCoverageReason::CheckedState { coordinate, state } => {
                SourceUseCoverageReason::CheckedState {
                    coordinate: self.boundary_slot(coordinate, correspondence)?,
                    state: match state {
                        CheckedValueState::Live => CheckedValueState::Live,
                        CheckedValueState::Null { source } => CheckedValueState::Null {
                            source: source.clone(),
                        },
                        CheckedValueState::Expired {
                            origin,
                            effect_point,
                        } => CheckedValueState::Expired {
                            origin: self.origin(owner, *origin)?,
                            effect_point: self.point(owner, *effect_point)?,
                        },
                    },
                }
            }
            SourceUseCoverageReason::PendingView(view) => {
                SourceUseCoverageReason::PendingView(self.source_view(view)?)
            }
            SourceUseCoverageReason::Projection(place) => {
                SourceUseCoverageReason::Projection(self.place(owner, place)?)
            }
            SourceUseCoverageReason::AssignmentInput => SourceUseCoverageReason::AssignmentInput,
            SourceUseCoverageReason::Transport {
                source,
                destination,
            } => SourceUseCoverageReason::Transport {
                source: self.boundary_slot(source, correspondence)?,
                destination: self.boundary_slot(destination, correspondence)?,
            },
            SourceUseCoverageReason::ForeignChild(event) => {
                SourceUseCoverageReason::ForeignChild(self.source_event(event)?)
            }
            SourceUseCoverageReason::SourceRecurrence(view) => {
                SourceUseCoverageReason::SourceRecurrence(self.source_view(view)?)
            }
        };
        Ok(SourceUseCoverage {
            event: self.source_event(&coverage.event)?,
            source_span: coverage.source_span.clone(),
            reason,
        })
    }

    pub(super) fn source_use_graphs(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceUseGraph>, ValueSchemaRemapError> {
        let common = self.common_loans(tables)?;
        let transitions = self.owner_receiving_transitions(tables)?;
        let creations = self.source_grant_creations(tables, correspondence)?;
        tables
            .source_use_graphs
            .iter()
            .map(|original| {
                let epochs = tables
                    .common_loans
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.input == original.epoch)
                    .collect::<Vec<_>>();
                let [(index, _)] = epochs.as_slice() else {
                    return Err(ValueSchemaRemapError::MissingProducer(
                        original.id.owner.clone(),
                    ));
                };
                let mut graph = original.clone();
                graph.id.owner = self.producer(&original.id.owner)?;
                graph.epoch = common[*index].input.clone();
                for node in &mut graph.nodes {
                    node.id = self.use_node(&node.id)?;
                    node.event = self.source_event(&node.event)?;
                    node.configuration = self.source_configuration(&node.configuration)?;
                    node.frame = self.source_frame(&node.frame)?;
                    node.supports = node
                        .supports
                        .iter()
                        .map(|support| self.use_support(support, correspondence))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                for usage in &mut graph.uses {
                    usage.event = self.source_event(&usage.event)?;
                    usage.contributor = self.source_tuple(&usage.contributor)?;
                    usage.equation = self.use_equation(tables, &usage.equation, correspondence)?;
                    usage.configuration = self.source_configuration(&usage.configuration)?;
                    usage.support = self.use_support(&usage.support, correspondence)?;
                    usage.input = match &usage.input {
                        SourceUseInput::Selected(port) => {
                            SourceUseInput::Selected(self.selected_port(port, correspondence)?)
                        }
                        SourceUseInput::Storage(slot) => {
                            SourceUseInput::Storage(SourceSelectedSlotFact {
                                coordinate: self.boundary_slot(&slot.coordinate, correspondence)?,
                                view: self.source_view(&slot.view)?,
                                availability: self
                                    .availability(&original.id.owner, &slot.availability)?,
                            })
                        }
                    };
                }
                for edge in &mut graph.edges {
                    let original_equation = edge.equation.clone();
                    edge.equation = self.use_equation(tables, &edge.equation, correspondence)?;
                    edge.source = self.use_support(&edge.source, correspondence)?;
                    edge.destination = self.use_support(&edge.destination, correspondence)?;
                    edge.witness = match &edge.witness {
                        SourceUseEdgeWitness::Transport(transport) => {
                            SourceUseEdgeWitness::Transport(JointValueTransportFact {
                                source: self.boundary_slot(&transport.source, correspondence)?,
                                destination: self.boundary_slot(&transport.destination, correspondence)?,
                                kind: match &transport.kind {
                                    JointValueTransportKind::Project(member) => {
                                        JointValueTransportKind::Project(SourceShapeMemberPort {
                                            shape: self.shape(&member.shape)?,
                                            member: member.member,
                                        })
                                    }
                                    kind => kind.clone(),
                                },
                                child: match &transport.child {
                                    JointValueChildPort::LocalInput => {
                                        JointValueChildPort::LocalInput
                                    }
                                    JointValueChildPort::SelectedChild { event, port } => {
                                        JointValueChildPort::SelectedChild {
                                            event: self.source_event(event)?,
                                            port: *port,
                                        }
                                    }
                                },
                            })
                        }
                        SourceUseEdgeWitness::Production(production) => {
                            let ordinal = original_equation
                                .productions
                                .iter()
                                .position(|row| row == production)
                                .ok_or_else(|| {
                                    ValueSchemaRemapError::MissingProducer(
                                        original.id.owner.clone(),
                                    )
                                })?;
                            SourceUseEdgeWitness::Production(
                                edge.equation.productions[ordinal].clone(),
                            )
                        }
                        SourceUseEdgeWitness::Parent(witness) => {
                            let index = tables
                                .source_grant_creations
                                .iter()
                                .position(|row| row == witness)
                                .ok_or_else(|| {
                                    ValueSchemaRemapError::MissingProducer(
                                        original.id.owner.clone(),
                                    )
                                })?;
                            SourceUseEdgeWitness::Parent(creations[index].clone())
                        }
                        SourceUseEdgeWitness::Receiving(transition) => {
                            let index = tables
                                .owner_receiving_transitions
                                .iter()
                                .position(|row| row == transition)
                                .ok_or_else(|| {
                                    ValueSchemaRemapError::MissingProducer(
                                        original.id.owner.clone(),
                                    )
                                })?;
                            SourceUseEdgeWitness::Receiving(transitions[index].clone())
                        }
                    };
                }
                for reachable in &mut graph.reachable {
                    reachable.support = self.use_support(&reachable.support, correspondence)?;
                }
                graph.coverage = graph
                    .coverage
                    .iter()
                    .map(|row| self.use_coverage(row, correspondence))
                    .collect::<Result<Vec<_>, _>>()?;
                for row in &mut graph.returns {
                    row.consumer = self.consumer(&row.consumer)?;
                    row.input = self.source_use_observation(&row.input)?;
                    row.output = self.use_node(&row.output)?;
                    let index = tables
                        .owner_receiving_transitions
                        .iter()
                        .position(|original| original == &row.transition)
                        .ok_or_else(|| {
                            ValueSchemaRemapError::MissingProducer(original.id.owner.clone())
                        })?;
                    row.transition = transitions[index].clone();
                    row.subjects = row
                        .subjects
                        .iter()
                        .map(|subject| self.use_subject(subject))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                Ok(graph)
            })
            .collect()
    }
}
