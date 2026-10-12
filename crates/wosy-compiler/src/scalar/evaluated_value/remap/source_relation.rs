use super::*;

impl ValueSchemaRemap {
    pub(super) fn grant_class(
        &self,
        class: &SourceGrantClassRef,
    ) -> Result<SourceGrantClassRef, ValueSchemaRemapError> {
        Ok(SourceGrantClassRef {
            owner: self.producer(&class.owner)?,
            variable: class.variable.clone(),
        })
    }
    pub(super) fn source_grant_creations(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceGrantCreationWitness>, ValueSchemaRemapError> {
        tables
            .source_grant_creations
            .iter()
            .map(|witness| {
                Ok(SourceGrantCreationWitness {
                    event: self.source_event(&witness.event)?,
                    input: self.source_configuration(&witness.input)?,
                    output: self.source_configuration(&witness.output)?,
                    contributor: self.source_tuple(&witness.contributor)?,
                    child: self.grant_class(&witness.child)?,
                    parent: match &witness.parent {
                        SourceGrantParent::Root => SourceGrantParent::Root,
                        SourceGrantParent::Selected {
                            class,
                            source,
                            selection,
                        } => SourceGrantParent::Selected {
                            class: self.grant_class(class)?,
                            source: self.value(source)?,
                            selection: self.selected_port(selection, correspondence)?,
                        },
                    },
                })
            })
            .collect()
    }
    fn move_ref(
        &self,
        id: &SourceMoveWitnessRef,
    ) -> Result<SourceMoveWitnessRef, ValueSchemaRemapError> {
        Ok(SourceMoveWitnessRef {
            owner: self.producer(&id.owner)?,
            index: id.index,
        })
    }
    fn move_input(
        &self,
        input: &SourceMoveInput,
    ) -> Result<SourceMoveInput, ValueSchemaRemapError> {
        Ok(SourceMoveInput {
            event: self.source_event(&input.event)?,
            source: self.value(&input.source)?,
            place: self.place(&input.event.owner, &input.place)?,
        })
    }
    pub(super) fn source_move_inputs(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<SourceMoveInput>, ValueSchemaRemapError> {
        tables
            .source_move_inputs
            .iter()
            .map(|input| self.move_input(input))
            .collect()
    }
    pub(super) fn source_move_witnesses(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceMoveWitnessFact>, ValueSchemaRemapError> {
        tables
            .source_move_witnesses
            .iter()
            .map(|fact| {
                Ok(SourceMoveWitnessFact {
                    id: self.move_ref(&fact.id)?,
                    event: self.source_event(&fact.event)?,
                    input: self.source_configuration(&fact.input)?,
                    output: self.source_configuration(&fact.output)?,
                    contributor: self.source_tuple(&fact.contributor)?,
                    source: self.value(&fact.source)?,
                    selection: self.selected_port(&fact.selection, correspondence)?,
                    place: self.place(&fact.event.owner, &fact.place)?,
                })
            })
            .collect()
    }
    pub(super) fn pending_source_moves(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<SourceMovePendingFact>, ValueSchemaRemapError> {
        tables
            .pending_source_moves
            .iter()
            .map(|fact| {
                Ok(SourceMovePendingFact {
                    event: self.source_event(&fact.event)?,
                    reason: match &fact.reason {
                        SourceMovePendingReason::Deferred => SourceMovePendingReason::Deferred,
                        SourceMovePendingReason::Through => SourceMovePendingReason::Through,
                        SourceMovePendingReason::UnsupportedPlace(place) => {
                            SourceMovePendingReason::UnsupportedPlace(
                                self.place(&fact.event.owner, place)?,
                            )
                        }
                        SourceMovePendingReason::SelectedSource(input) => {
                            SourceMovePendingReason::SelectedSource(self.move_input(input)?)
                        }
                    },
                })
            })
            .collect()
    }
    pub(super) fn pending_owner_receiving(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<OwnerReceivingPendingFact>, ValueSchemaRemapError> {
        tables
            .pending_owner_receiving
            .iter()
            .map(|pending| {
                Ok(OwnerReceivingPendingFact {
                    event: self.source_event(&pending.event)?,
                    input: self.source_configuration(&pending.input)?,
                    source: self.value(&pending.source)?,
                    reason: match &pending.reason {
                        OwnerReceivingPendingReason::DeferredPolicy => {
                            OwnerReceivingPendingReason::DeferredPolicy
                        }
                        OwnerReceivingPendingReason::SelectedSource => {
                            OwnerReceivingPendingReason::SelectedSource
                        }
                        OwnerReceivingPendingReason::DependencyAction { case, dependency } => {
                            OwnerReceivingPendingReason::DependencyAction {
                                case: self.case(case)?,
                                dependency: self.dependency(&pending.event.owner, dependency)?,
                            }
                        }
                    },
                })
            })
            .collect()
    }
    pub(super) fn owner_receiving_transitions(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<OwnerReceivingTransitionFact>, ValueSchemaRemapError> {
        tables
            .owner_receiving_transitions
            .iter()
            .map(|transition| {
                let receiver = tables
                    .owner_capabilities
                    .iter()
                    .find(|owner| owner.id == transition.receiving)
                    .ok_or_else(|| {
                        ValueSchemaRemapError::MissingOwnerCapability(transition.receiving.clone())
                    })?;
                let mapped = self.owner_fact(receiver)?;
                let endpoints = transition
                    .endpoints
                    .iter()
                    .map(|endpoint| {
                        let position = receiver
                            .endpoints
                            .iter()
                            .position(|original| original == endpoint)
                            .ok_or_else(|| {
                                ValueSchemaRemapError::MissingOwnerEndpoint(endpoint.clone())
                            })?;
                        Ok(mapped.endpoints[position].clone())
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                Ok(OwnerReceivingTransitionFact {
                    event: self.source_event(&transition.event)?,
                    input: self.source_configuration(&transition.input)?,
                    output: self.source_configuration(&transition.output)?,
                    receiving: self.capability(&transition.receiving)?,
                    receiving_instance: self.source_instance(&transition.receiving_instance)?,
                    endpoints,
                    dependencies: transition
                        .dependencies
                        .iter()
                        .map(|selection| {
                            Ok(OwnerDependencySelection {
                                case: self.case(&selection.case)?,
                                projection: self.projections(&selection.projection)?,
                                path: self.path(&selection.path)?,
                                source_instance: self
                                    .source_instance(&selection.source_instance)?,
                                owner_instance: self.source_instance(&selection.owner_instance)?,
                                dependency: self
                                    .dependency(&transition.event.owner, &selection.dependency)?,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                })
            })
            .collect()
    }
    pub(super) fn source_monitor(
        &self,
        id: &JointPortMonitorRef,
    ) -> Result<JointPortMonitorRef, ValueSchemaRemapError> {
        Ok(JointPortMonitorRef {
            owner: self.producer(&id.owner)?,
            index: id.index,
        })
    }
    fn saved_selection(
        &self,
        selection: &FrozenSelectionOccurrence,
    ) -> Result<FrozenSelectionOccurrence, ValueSchemaRemapError> {
        Ok(FrozenSelectionOccurrence {
            application: AssignmentApplicationRef {
                frame: self.source_frame(&selection.application.frame)?,
                operation: self.operation(&selection.application.operation)?,
            },
            destination: AssignmentDestinationId {
                operation: self.operation(&selection.destination.operation)?,
                target: selection.destination.target,
            },
        })
    }
    fn port_case(
        &self,
        row: &SourceCaseViewFact,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<SourceCaseViewFact, ValueSchemaRemapError> {
        Ok(SourceCaseViewFact {
            case: self.case(&row.case)?,
            view: self.source_view(&row.view)?,
            port: self.boundary_slot(&row.port, correspondence)?,
            path: self.path(&row.path)?,
            configuration: self.source_configuration(&row.configuration)?,
            contributor: self.source_tuple(&row.contributor)?,
        })
    }
    pub(super) fn selected_port(
        &self,
        port: &JointSelectedPortFact,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<JointSelectedPortFact, ValueSchemaRemapError> {
        Ok(JointSelectedPortFact {
            coordinate: self.boundary_slot(&port.coordinate, correspondence)?,
            observation: self.port_case(&port.observation, correspondence)?,
            sources: port
                .sources
                .iter()
                .map(|source| self.port_case(source, correspondence))
                .collect::<Result<Vec<_>, _>>()?,
            moves: port
                .moves
                .iter()
                .map(|id| self.move_ref(id))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
    pub(super) fn source_port_monitors(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<JointPortMonitorFact>, ValueSchemaRemapError> {
        tables
            .source_port_monitors
            .iter()
            .map(|monitor| {
                Ok(JointPortMonitorFact {
                    id: self.source_monitor(&monitor.id)?,
                    contributor: self.source_tuple(&monitor.contributor)?,
                    ports: monitor
                        .ports
                        .iter()
                        .map(|port| self.selected_port(port, correspondence))
                        .collect::<Result<Vec<_>, _>>()?,
                    pending: monitor
                        .pending
                        .iter()
                        .map(|port| self.boundary_slot(port, correspondence))
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect()
    }
    pub(super) fn source_port_equations(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<JointPortMonitorEquation>, ValueSchemaRemapError> {
        tables
            .source_port_equations
            .iter()
            .map(|equation| {
                Ok(JointPortMonitorEquation {
                    contributor: self.source_tuple(&equation.contributor)?,
                    input: self.source_monitor(&equation.input)?,
                    output: self.source_monitor(&equation.output)?,
                    moves: equation
                        .moves
                        .iter()
                        .map(|id| self.move_ref(id))
                        .collect::<Result<Vec<_>, _>>()?,
                    children: equation
                        .children
                        .iter()
                        .map(|child| {
                            Ok(JointSelectedChildFact {
                                event: self.source_event(&child.event)?,
                                monitor: self.source_monitor(&child.monitor)?,
                                ports: child
                                    .ports
                                    .iter()
                                    .map(|port| self.boundary_slot(port, correspondence))
                                    .collect::<Result<Vec<_>, _>>()?,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    productions: equation
                        .productions
                        .iter()
                        .map(|production| {
                            Ok(JointPortProductionFact {
                                observation: self.port_case(&production.observation, correspondence)?,
                                inputs: production
                                    .inputs
                                    .iter()
                                    .map(|port| self.boundary_slot(port, correspondence))
                                    .collect::<Result<Vec<_>, _>>()?,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                })
            })
            .collect()
    }
    pub(super) fn source_saved_ports(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceSavedPortFact>, ValueSchemaRemapError> {
        tables
            .source_saved_ports
            .iter()
            .map(|saved| {
                Ok(SourceSavedPortFact {
                    source_place: self.place(&saved.snapshot.owner, &saved.source_place)?,
                    snapshot: self.source_event(&saved.snapshot)?,
                    selection: self.saved_selection(&saved.selection)?,
                    monitor: self.source_monitor(&saved.monitor)?,
                    selected: self.selected_port(&saved.selected, correspondence)?,
                    duties: saved
                        .duties
                        .iter()
                        .map(|owns| {
                            let capability = tables
                                .owner_capabilities
                                .iter()
                                .find(|capability| capability.id == owns.capability)
                                .ok_or_else(|| {
                                    ValueSchemaRemapError::MissingOwnerCapability(
                                        owns.capability.clone(),
                                    )
                                })?;
                            let position = capability
                                .endpoints
                                .iter()
                                .position(|endpoint| endpoint == &owns.endpoint)
                                .ok_or_else(|| {
                                    ValueSchemaRemapError::MissingOwnerEndpoint(
                                        owns.endpoint.clone(),
                                    )
                                })?;
                            Ok(SourceOwnsFact {
                                capability: self.capability(&owns.capability)?,
                                owner_instance: self.source_instance(&owns.owner_instance)?,
                                payload: self.source_instance(&owns.payload)?,
                                endpoint: self.owner_fact(capability)?.endpoints[position].clone(),
                                kind: owns.kind.clone(),
                                state: owns.state,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                })
            })
            .collect()
    }
    pub(super) fn source_case_views(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceCaseViewFact>, ValueSchemaRemapError> {
        tables
            .source_case_views
            .iter()
            .map(|row| {
                Ok(SourceCaseViewFact {
                    case: self.case(&row.case)?,
                    view: self.source_view(&row.view)?,
                    port: self.boundary_slot(&row.port, correspondence)?,
                    path: self.path(&row.path)?,
                    configuration: self.source_configuration(&row.configuration)?,
                    contributor: self.source_tuple(&row.contributor)?,
                })
            })
            .collect()
    }
    pub(super) fn source_owner_consumers(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<EvaluatedOwnerConsumer>, ValueSchemaRemapError> {
        tables
            .owner_consumers
            .iter()
            .map(|row| self.source_owner_consumer(row))
            .collect()
    }
    fn source_owner_consumer(
        &self,
        row: &EvaluatedOwnerConsumer,
    ) -> Result<EvaluatedOwnerConsumer, ValueSchemaRemapError> {
        Ok(EvaluatedOwnerConsumer {
            consumer: self.consumer(&row.consumer)?,
            input: self.case(&row.input)?,
            path: self.path(&row.path)?,
            receiving: self.capability(&row.receiving)?,
            contributor: self.source_tuple(&row.contributor)?,
            disposition: self.disposition(&row.consumer.producer, &row.disposition)?,
            dependencies: row
                .dependencies
                .iter()
                .map(|selection| {
                    Ok(OwnerDependencySelection {
                        case: self.case(&selection.case)?,
                        projection: self.projections(&selection.projection)?,
                        path: self.path(&selection.path)?,
                        source_instance: self.source_instance(&selection.source_instance)?,
                        owner_instance: self.source_instance(&selection.owner_instance)?,
                        dependency: self
                            .dependency(&row.consumer.producer, &selection.dependency)?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }
    pub(super) fn pending_owner_consumers(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<PendingOwnerConsumer>, ValueSchemaRemapError> {
        tables
            .pending_owner_consumers
            .iter()
            .map(|pending| {
                Ok(PendingOwnerConsumer {
                    monitor: self.source_monitor(&pending.monitor)?,
                    candidate: self.source_owner_consumer(&pending.candidate)?,
                    requirements: pending
                        .requirements
                        .iter()
                        .map(|requirement| {
                            Ok(match requirement {
                                OwnerConsumerRequirement::SelectedRemainingUses(observation) => {
                                    OwnerConsumerRequirement::SelectedRemainingUses(
                                        self.source_use_observation(observation)?,
                                    )
                                }
                                OwnerConsumerRequirement::StagedLoanValidation(event) => {
                                    OwnerConsumerRequirement::StagedLoanValidation(
                                        self.source_event(event)?,
                                    )
                                }
                                OwnerConsumerRequirement::CandidateLocalQueries(id) => {
                                    OwnerConsumerRequirement::CandidateLocalQueries(
                                        StagedLoanQueryRef {
                                            owner: self.producer(&id.owner)?,
                                        },
                                    )
                                }
                                OwnerConsumerRequirement::RemainingDependencyUses(
                                    configuration,
                                ) => OwnerConsumerRequirement::RemainingDependencyUses(
                                    self.source_configuration(configuration)?,
                                ),
                                OwnerConsumerRequirement::SourceMoveWitness(value) => {
                                    OwnerConsumerRequirement::SourceMoveWitness(self.value(value)?)
                                }
                                OwnerConsumerRequirement::CompleteSourcePredicateProduct(
                                    configuration,
                                ) => OwnerConsumerRequirement::CompleteSourcePredicateProduct(
                                    self.source_configuration(configuration)?,
                                ),
                                OwnerConsumerRequirement::FiniteSupportNormalization(
                                    configuration,
                                ) => OwnerConsumerRequirement::FiniteSupportNormalization(
                                    self.source_configuration(configuration)?,
                                ),
                                OwnerConsumerRequirement::FixedReturnMaterialization(instance) => {
                                    OwnerConsumerRequirement::FixedReturnMaterialization(
                                        self.source_instance(instance)?,
                                    )
                                }
                                OwnerConsumerRequirement::BoundarySubstitution(formal) => {
                                    OwnerConsumerRequirement::BoundarySubstitution(
                                        self.source_formal(formal, correspondence)?,
                                    )
                                }
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                })
            })
            .collect()
    }
    pub(super) fn source_role(
        &self,
        owner: &ProducerCfgOwnerId,
        role: &SourceCoordinateRole,
    ) -> Result<SourceCoordinateRole, ValueSchemaRemapError> {
        Ok(match role {
            SourceCoordinateRole::Boundary(role) => {
                SourceCoordinateRole::Boundary(self.boundary_role(owner, role)?)
            }
            SourceCoordinateRole::Binding(binding) => {
                SourceCoordinateRole::Binding(self.binding(owner, binding)?)
            }
            SourceCoordinateRole::Evaluated(value) => {
                SourceCoordinateRole::Evaluated(self.value(value)?)
            }
            SourceCoordinateRole::Owner(capability) => {
                SourceCoordinateRole::Owner(self.capability(capability)?)
            }
            SourceCoordinateRole::Saved(selection) => {
                SourceCoordinateRole::Saved(self.saved_selection(selection)?)
            }
        })
    }
    pub(super) fn source_instance(
        &self,
        instance: &SourceInstanceRef,
    ) -> Result<SourceInstanceRef, ValueSchemaRemapError> {
        Ok(match instance {
            SourceInstanceRef::Supported(id) => {
                SourceInstanceRef::Supported(SourceSupportedClassRef {
                    owner: self.producer(&id.owner)?,
                    index: id.index,
                })
            }
            SourceInstanceRef::Residual(id) => SourceInstanceRef::Residual(RefinedPopulationRef {
                owner: self.producer(&id.owner)?,
                index: id.index,
            }),
        })
    }
    pub(super) fn source_view(
        &self,
        id: &SourceValueViewRef,
    ) -> Result<SourceValueViewRef, ValueSchemaRemapError> {
        Ok(SourceValueViewRef {
            owner: self.producer(&id.owner)?,
            index: id.index,
        })
    }
    pub(super) fn source_configuration(
        &self,
        id: &SourceConfigurationRef,
    ) -> Result<SourceConfigurationRef, ValueSchemaRemapError> {
        Ok(SourceConfigurationRef {
            owner: self.producer(&id.owner)?,
            index: id.index,
        })
    }
    pub(super) fn source_tuple(
        &self,
        id: &JointProducerTupleRef,
    ) -> Result<JointProducerTupleRef, ValueSchemaRemapError> {
        Ok(JointProducerTupleRef {
            owner: self.producer(&id.owner)?,
            index: id.index,
        })
    }
    fn source_formal(
        &self,
        id: &BoundaryFormalPortRef,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<BoundaryFormalPortRef, ValueSchemaRemapError> {
        correspondence.formal(id)
    }
    pub(super) fn source_frame(
        &self,
        frame: &FrameVariableRef,
    ) -> Result<FrameVariableRef, ValueSchemaRemapError> {
        Ok(FrameVariableRef {
            owner: self.producer(&frame.owner)?,
            variable: frame.variable,
        })
    }
    fn source_object_role(
        &self,
        role: &SourceObjectRole,
    ) -> Result<SourceObjectRole, ValueSchemaRemapError> {
        Ok(match role {
            SourceObjectRole::Owner(owner) => SourceObjectRole::Owner(self.capability(owner)?),
            _ => role.clone(),
        })
    }
    pub(super) fn source_configurations(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceConfigurationFact>, ValueSchemaRemapError> {
        tables
            .source_configurations
            .iter()
            .map(|original| {
                let mut row = original.clone();
                row.id = self.source_configuration(&original.id)?;
                row.frame = self.source_frame(&original.frame)?;
                for slot in &mut row.slots {
                    slot.coordinate = self.boundary_slot(&slot.coordinate, correspondence)?;
                    slot.view = self.source_view(&slot.view)?;
                    slot.availability =
                        self.availability(&original.id.owner, &slot.availability)?;
                }
                row.instances = row
                    .instances
                    .iter()
                    .map(|instance| self.source_instance(instance))
                    .collect::<Result<Vec<_>, _>>()?;
                for created in &mut row.created {
                    created.instance = self.source_instance(&created.instance)?;
                    created.event = self.source_event(&created.event)?;
                    created.shape = self.shape(&created.shape)?;
                    created.role = self.source_object_role(&created.role)?;
                }
                for (owns, old) in row.owns.iter_mut().zip(&original.owns) {
                    owns.capability = self.capability(&owns.capability)?;
                    owns.owner_instance = self.source_instance(&owns.owner_instance)?;
                    owns.payload = self.source_instance(&owns.payload)?;
                    let capability = tables
                        .owner_capabilities
                        .iter()
                        .find(|fact| fact.id == old.capability)
                        .ok_or_else(|| {
                            ValueSchemaRemapError::MissingOwnerCapability(old.capability.clone())
                        })?;
                    let position = capability
                        .endpoints
                        .iter()
                        .position(|endpoint| endpoint == &owns.endpoint)
                        .ok_or_else(|| {
                            ValueSchemaRemapError::MissingOwnerEndpoint(owns.endpoint.clone())
                        })?;
                    owns.endpoint = self.owner_fact(capability)?.endpoints[position].clone();
                }
                for depends in &mut row.depends {
                    depends.source = self.source_instance(&depends.source)?;
                    depends.target = self.source_instance(&depends.target)?;
                }
                for target in &mut row.has_target {
                    target.source = self.source_instance(&target.source)?;
                    target.target.owner = self.producer(&target.target.owner)?;
                }
                for functional in &mut row.functional {
                    functional.source = self.source_instance(&functional.source)?;
                }
                for relation in row
                    .same_target
                    .iter_mut()
                    .chain(row.different_target.iter_mut())
                {
                    relation.left = self.source_instance(&relation.left)?;
                    relation.right = self.source_instance(&relation.right)?;
                }
                for input in &mut row.formal_inputs {
                    input.port = self.source_formal(&input.port, correspondence)?;
                    input.coordinate = self.boundary_slot(&input.coordinate, correspondence)?;
                    input.shape = self.shape(&input.shape)?;
                }
                for pending in &mut row.pending {
                    pending.event = self.source_event(&pending.event)?;
                    pending.outputs = pending
                        .outputs
                        .iter()
                        .map(|value| self.value(value))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                for saved in &mut row.saved {
                    saved.coordinate = self.boundary_slot(&saved.coordinate, correspondence)?;
                    saved.snapshot = self.source_event(&saved.snapshot)?;
                    saved.selection.application.frame =
                        self.source_frame(&saved.selection.application.frame)?;
                    saved.selection.application.operation =
                        self.operation(&saved.selection.application.operation)?;
                    saved.selection.destination.operation =
                        self.operation(&saved.selection.destination.operation)?;
                    saved.holder = self.binding(&original.id.owner, &saved.holder)?;
                    saved.view = self.source_view(&saved.view)?;
                    saved.availability =
                        self.availability(&original.id.owner, &saved.availability)?;
                }
                for grant in &mut row.grants {
                    grant.class = self.grant_class(&grant.class)?;
                    grant.site = self.source_event(&grant.site)?;
                    grant.referent = self.source_instance(&grant.referent)?;
                    grant.coordinate = self.boundary_slot(&grant.coordinate, correspondence)?;
                    grant.projection = self.projections(&grant.projection)?;
                }
                for pair in &mut row.grant_ancestry {
                    pair.ancestor = self.grant_class(&pair.ancestor)?;
                    pair.descendant = self.grant_class(&pair.descendant)?;
                }
                for view in &mut row.owner_views {
                    view.coordinate = self.boundary_slot(&view.coordinate, correspondence)?;
                    view.projection = self.projections(&view.projection)?;
                    view.payload = self.source_instance(&view.payload)?;
                    view.capability = self.capability(&view.capability)?;
                    view.owner_instance = self.source_instance(&view.owner_instance)?;
                }
                Ok(row)
            })
            .collect()
    }
    pub(super) fn source_contributors(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<JointProducerTupleFact>, ValueSchemaRemapError> {
        tables
            .source_contributors
            .iter()
            .map(|original| {
                let mut row = original.clone();
                row.id = self.source_tuple(&row.id)?;
                row.event = self.source_event(&row.event)?;
                row.input = self.source_configuration(&row.input)?;
                row.output = self.source_configuration(&row.output)?;
                row.kind = match &row.kind {
                    JointProducerRuleKind::Branch { predecessor } => {
                        JointProducerRuleKind::Branch {
                            predecessor: self.source_event(predecessor)?,
                        }
                    }
                    JointProducerRuleKind::Backedge { predecessor } => {
                        JointProducerRuleKind::Backedge {
                            predecessor: self.source_event(predecessor)?,
                        }
                    }
                    JointProducerRuleKind::Call { child } => JointProducerRuleKind::Call {
                        child: self.producer(child)?,
                    },
                    JointProducerRuleKind::Capture { child } => JointProducerRuleKind::Capture {
                        child: self.producer(child)?,
                    },
                    JointProducerRuleKind::Root
                    | JointProducerRuleKind::Step
                    | JointProducerRuleKind::Return => row.kind,
                };
                for child in &mut row.children {
                    child.event = self.source_event(&child.event)?;
                    child.configuration = self.source_configuration(&child.configuration)?;
                    child.ports = child
                        .ports
                        .iter()
                        .map(|port| self.boundary_slot(port, correspondence))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                row.preserved = row
                    .preserved
                    .iter()
                    .map(|instance| self.source_instance(instance))
                    .collect::<Result<Vec<_>, _>>()?;
                row.fresh = row
                    .fresh
                    .iter()
                    .map(|instance| self.source_instance(instance))
                    .collect::<Result<Vec<_>, _>>()?;
                row.outputs = row
                    .outputs
                    .iter()
                    .map(|value| self.value(value))
                    .collect::<Result<Vec<_>, _>>()?;
                row.transports = row
                    .transports
                    .iter()
                    .map(|transport| {
                        Ok(JointValueTransportFact {
                            source: self.boundary_slot(&transport.source, correspondence)?,
                            destination: self.boundary_slot(&transport.destination, correspondence)?,
                            kind: match &transport.kind {
                                JointValueTransportKind::Project(port) => {
                                    JointValueTransportKind::Project(SourceShapeMemberPort {
                                        shape: self.shape(&port.shape)?,
                                        member: port.member,
                                    })
                                }
                                other => other.clone(),
                            },
                            child: match &transport.child {
                                JointValueChildPort::LocalInput => JointValueChildPort::LocalInput,
                                JointValueChildPort::SelectedChild { event, port } => {
                                    JointValueChildPort::SelectedChild {
                                        event: self.source_event(event)?,
                                        port: *port,
                                    }
                                }
                            },
                        })
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                row.productions = row
                    .productions
                    .iter()
                    .map(|production| {
                        Ok(JointValueProductionPort {
                            destination: self.boundary_slot(&production.destination, correspondence)?,
                            inputs: production
                                .inputs
                                .iter()
                                .map(|port| self.boundary_slot(port, correspondence))
                                .collect::<Result<Vec<_>, _>>()?,
                        })
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                Ok(row)
            })
            .collect()
    }
    pub(super) fn source_instances(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceSupportedClassFact>, ValueSchemaRemapError> {
        tables
            .source_instances
            .iter()
            .map(|original| {
                let mut row = original.clone();
                row.id.owner = self.producer(&row.id.owner)?;
                row.frame = self.source_frame(&row.frame)?;
                row.support = self.boundary_slot(&row.support, correspondence)?;
                row.shape = self.shape(&row.shape)?;
                row.role = self.source_object_role(&row.role)?;
                row.origin = match &row.origin {
                    SourceInstanceOrigin::Formal(formal) => {
                        SourceInstanceOrigin::Formal(self.source_formal(formal, correspondence)?)
                    }
                    SourceInstanceOrigin::Creation(event) => {
                        SourceInstanceOrigin::Creation(self.source_event(event)?)
                    }
                };
                Ok(row)
            })
            .collect()
    }
    pub(super) fn source_populations(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<SourcePopulationFact>, ValueSchemaRemapError> {
        tables
            .source_populations
            .iter()
            .map(|original| {
                let mut row = original.clone();
                row.id.owner = self.producer(&row.id.owner)?;
                row.site = self.source_event(&row.site)?;
                row.shape = self.shape(&row.shape)?;
                row.role = self.source_object_role(&row.role)?;
                row.owner = self.capability(&row.owner)?;
                Ok(row)
            })
            .collect()
    }
    pub(super) fn source_views(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<SourceValueViewFact>, ValueSchemaRemapError> {
        tables
            .source_views
            .iter()
            .map(|original| {
                let mut row = original.clone();
                row.id = self.source_view(&row.id)?;
                row.shape = self.shape(&row.shape)?;
                row.payload = match &row.payload {
                    SourcePayloadView::Plain => SourcePayloadView::Plain,
                    SourcePayloadView::Unwritten => SourcePayloadView::Unwritten,
                    SourcePayloadView::StaticRaw(source) => {
                        SourcePayloadView::StaticRaw(source.clone())
                    }
                    SourcePayloadView::Formal(formal) => {
                        SourcePayloadView::Formal(self.source_formal_value(formal)?)
                    }
                    SourcePayloadView::Raw {
                        provenance,
                        referent,
                    } => {
                        let EvaluatedPayloadFact::Raw { provenance } = self.payload(
                            &original.id.owner,
                            &EvaluatedPayloadFact::Raw {
                                provenance: provenance.clone(),
                            },
                        )?
                        else {
                            unreachable!()
                        };
                        SourcePayloadView::Raw {
                            provenance,
                            referent: self.source_instance(referent)?,
                        }
                    }
                    SourcePayloadView::Aggregate(members) => SourcePayloadView::Aggregate(
                        members
                            .iter()
                            .map(|member| {
                                Ok(SourceMemberView {
                                    projection: self.projections(&member.projection)?,
                                    source: self.value(&member.source)?,
                                    view: self.source_view(&member.view)?,
                                })
                            })
                            .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    ),
                    SourcePayloadView::Backing { instance, element } => {
                        SourcePayloadView::Backing {
                            instance: self.source_instance(instance)?,
                            element: self.shape(element)?,
                        }
                    }
                    SourcePayloadView::Checked { state, referents } => {
                        let EvaluatedPayloadFact::Checked { state, .. } = self.payload(
                            &original.id.owner,
                            &EvaluatedPayloadFact::Checked {
                                state: state.clone(),
                                origins: BTreeSet::new(),
                            },
                        )?
                        else {
                            unreachable!()
                        };
                        SourcePayloadView::Checked {
                            state,
                            referents: referents
                                .iter()
                                .map(|referent| {
                                    Ok(SourceCheckedReferentView {
                                        instance: self.source_instance(&referent.instance)?,
                                        coordinate: self.boundary_slot(&referent.coordinate, correspondence)?,
                                        projection: self.projections(&referent.projection)?,
                                        grant: referent.grant.clone(),
                                        mode: referent.mode,
                                        loan: match &referent.loan {
                                            SourceCheckedGrantState::Selected(class) => {
                                                SourceCheckedGrantState::Selected(
                                                    self.grant_class(class)?,
                                                )
                                            }
                                            SourceCheckedGrantState::Pending { event } => {
                                                SourceCheckedGrantState::Pending {
                                                    event: self.source_event(event)?,
                                                }
                                            }
                                        },
                                    })
                                })
                                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                        }
                    }
                };
                row.dependencies = row
                    .dependencies
                    .iter()
                    .map(|dependency| self.dependency(&original.id.owner, dependency))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(row)
            })
            .collect()
    }
    pub(super) fn source_paths(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<EvaluatedPathFact>, ValueSchemaRemapError> {
        tables
            .source_paths
            .iter()
            .map(|original| {
                let mut row = original.clone();
                row.id.producer = self.producer(&row.id.producer)?;
                row.event = self.source_event(&row.event)?;
                row.frame = self.source_frame(&row.frame)?;
                row.configuration = self.source_configuration(&row.configuration)?;
                row.contributor = self.source_tuple(&row.contributor)?;
                Ok(row)
            })
            .collect()
    }
    pub(super) fn source_backings(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<SourceBackingObservation>, ValueSchemaRemapError> {
        tables
            .source_backing_observations
            .iter()
            .map(|row| {
                let backing = self
                    .import(&row.view.owner)?
                    .backings
                    .iter()
                    .find(|(old, _)| old == &row.backing)
                    .map(|(_, new)| new.clone())
                    .ok_or_else(|| ValueSchemaRemapError::MissingBacking(row.backing.clone()))?;
                Ok(SourceBackingObservation {
                    view: self.source_view(&row.view)?,
                    backing,
                })
            })
            .collect()
    }
}
