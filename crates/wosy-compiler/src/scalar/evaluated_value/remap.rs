use super::*;
mod common_loan;
mod local_loan;
mod source_relation;
mod formal_graph;
mod formal_completeness;
mod name_resolution;
mod null_constructor;
mod checked_parameter;
mod direct_storage;
mod allocation_identity;
mod source_use;
mod boundary_arena;
mod capture_graph;
mod graph_transport;
mod graph_dependencies;
mod qualified_shapes;
mod mapped_dependencies;
#[cfg(test)]
mod correction_tests;
#[cfg(test)]
mod raw_domain_tests;
mod construction_forest;
mod destination_evidence;
mod resources;
mod header_counterpart;
mod input_counterpart;
pub use resources::{CheckedBoundaryResources, RemappedSourceUseGraphs};
#[cfg(test)]
mod mechanical_schema;
#[cfg(test)]
pub(in crate::scalar::evaluated_value) use mechanical_schema::MechanicalSchemaTableSource;
pub use destination_evidence::BoundaryGraphDestination;
use destination_evidence::BoundaryTableHeaders;
use boundary_arena::BoundaryArenaCorrespondence;
#[cfg(test)]
mod source_use_tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueImportCorrespondence {
    pub producer: ProducerCfgOwnerId,
    pub bindings: Vec<(ReferenceBindingId, ReferenceBindingId)>,
    pub places: Vec<(ReferencePlaceId, ReferencePlaceId)>,
    pub origins: Vec<ReferenceOriginId>,
    pub loans: Vec<(ReferenceLoanId, ReferenceLoanId)>,
    pub runtime_values: Vec<(RuntimeValueId, RuntimeValueId)>,
    pub environments: Vec<(CallableEnvironmentId, CallableEnvironmentId)>,
    pub backings: Vec<(RuntimeBackingFact, RuntimeBackingFact)>,
    pub holders: Vec<(ConnectedCaptureHolderFact, ConnectedCaptureHolderFact)>,
}

impl ValueSchemaRemap {
    pub fn callable_field_copy(
        &self,
        producer: &ProducerCfgOwnerId,
        source: &CallableFieldSource,
        target: &ReferenceBindingId,
    ) -> Result<CfgPointKind, ValueSchemaRemapError> {
        Ok(CfgPointKind::CopyCallableFields {
            source: self.callable_field_source(producer, source)?,
            target: self.binding(producer, target)?,
        })
    }

    pub fn callable_field_source(
        &self,
        producer: &ProducerCfgOwnerId,
        source: &CallableFieldSource,
    ) -> Result<CallableFieldSource, ValueSchemaRemapError> {
        Ok(match source {
            CallableFieldSource::Evaluated(value) => CallableFieldSource::Evaluated(self.value(value)?),
            CallableFieldSource::Assignment(binding) => CallableFieldSource::Assignment(self.binding(producer, binding)?),
        })
    }

    fn source_event(&self, event: &SourceEventId) -> Result<SourceEventId, ValueSchemaRemapError> {
        Ok(SourceEventId {
            owner: self.producer(&event.owner)?,
            point: self.point(&event.owner, event.point)?,
        })
    }

    fn owner_fact(
        &self,
        owner: &OwnerCapabilityFact,
    ) -> Result<OwnerCapabilityFact, ValueSchemaRemapError> {
        let producer = &owner.id.producer;
        Ok(OwnerCapabilityFact {
            id: self.capability(&owner.id)?,
            frame: FrameVariableRef {
                owner: self.producer(&owner.frame.owner)?,
                variable: owner.frame.variable,
            },
            role: match &owner.role {
                OwnerRole::Module(binding) => OwnerRole::Module(self.binding(producer, binding)?),
                OwnerRole::Scope(block) => OwnerRole::Scope(block.clone()),
                OwnerRole::Binding(binding) => OwnerRole::Binding(self.binding(producer, binding)?),
                OwnerRole::Parameter(binding) => {
                    OwnerRole::Parameter(self.binding(producer, binding)?)
                }
                OwnerRole::Result(consumer) => OwnerRole::Result(self.consumer(consumer)?),
                OwnerRole::Discard(consumer) => OwnerRole::Discard(self.consumer(consumer)?),
            },
            parent: match &owner.parent {
                OwnerParent::Root => OwnerParent::Root,
                OwnerParent::Nested(parent) => OwnerParent::Nested(self.capability(parent)?),
            },
            introduction: self.source_event(&owner.introduction)?,
            endpoints: owner
                .endpoints
                .iter()
                .map(|endpoint| {
                    Ok(OwnerEndpointFact {
                        event: self.source_event(&endpoint.event)?,
                        frame: FrameVariableRef {
                            owner: self.producer(&endpoint.frame.owner)?,
                            variable: endpoint.frame.variable,
                        },
                        kind: match &endpoint.kind {
                            OwnerEndpointKind::ScopeExit { block } => {
                                OwnerEndpointKind::ScopeExit {
                                    block: block.clone(),
                                }
                            }
                            OwnerEndpointKind::CheckedDiscard { consumer } => {
                                OwnerEndpointKind::CheckedDiscard {
                                    consumer: self.consumer(consumer)?,
                                }
                            }
                            OwnerEndpointKind::ParameterExit { binding } => {
                                OwnerEndpointKind::ParameterExit {
                                    binding: self.binding(producer, binding)?,
                                }
                            }
                            OwnerEndpointKind::ResultHandoff { consumer } => {
                                OwnerEndpointKind::ResultHandoff {
                                    consumer: self.consumer(consumer)?,
                                }
                            }
                            OwnerEndpointKind::ModuleTerminal { binding } => {
                                OwnerEndpointKind::ModuleTerminal {
                                    binding: self.binding(producer, binding)?,
                                }
                            }
                        },
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    fn boundary_role(
        &self,
        owner: &ProducerCfgOwnerId,
        role: &BoundaryPortRole,
    ) -> Result<BoundaryPortRole, ValueSchemaRemapError> {
        Ok(match role {
            BoundaryPortRole::Parameter { ordinal, binding } => BoundaryPortRole::Parameter {
                ordinal: *ordinal,
                binding: self.binding(owner, binding)?,
            },
            BoundaryPortRole::Capture { ordinal, binding } => BoundaryPortRole::Capture {
                ordinal: *ordinal,
                binding: self.binding(owner, binding)?,
            },
            BoundaryPortRole::Global(binding) => {
                BoundaryPortRole::Global(self.binding(owner, binding)?)
            }
            BoundaryPortRole::Output(ordinal) => BoundaryPortRole::Output(*ordinal),
            BoundaryPortRole::Effect {
                binding,
                projection,
            } => BoundaryPortRole::Effect {
                binding: self.binding(owner, binding)?,
                projection: projection
                    .iter()
                    .map(|projection| {
                        Ok(match projection {
                            ReferencePlaceProjection::Field(field) => {
                                ReferencePlaceProjection::Field(field.clone())
                            }
                            ReferencePlaceProjection::Index(index) => {
                                ReferencePlaceProjection::Index(index.clone())
                            }
                            ReferencePlaceProjection::Dereference(loan) => {
                                ReferencePlaceProjection::Dereference(Box::new(
                                    self.checked_loan(owner, loan)?,
                                ))
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
        })
    }

    fn boundary_slot(&self, slot: &SourceSlotRef, correspondence: &BoundaryArenaCorrespondence) -> Result<SourceSlotRef, ValueSchemaRemapError> {
        correspondence.slot(slot)
    }

    fn boundary_target(
        &self,
        owner: &ProducerCfgOwnerId,
        target: &ResolvedCallTarget,
    ) -> Result<ResolvedCallTarget, ValueSchemaRemapError> {
        Ok(match target {
            ResolvedCallTarget::LocalCallable(binding) => {
                ResolvedCallTarget::LocalCallable(self.binding(owner, binding)?)
            }
            ResolvedCallTarget::CallableField {
                receiver_place,
                field,
            } => ResolvedCallTarget::CallableField {
                receiver_place: self.place(owner, receiver_place)?,
                field: field.clone(),
            },
            ResolvedCallTarget::Core(_)
            | ResolvedCallTarget::ModuleCallable { .. }
            | ResolvedCallTarget::ExternCallable { .. } => target.clone(),
        })
    }

    pub fn binding_point(
        &self,
        producer: &ProducerCfgOwnerId,
        binding: &ScalarBindingCfgPoint,
    ) -> Result<ScalarBindingCfgPoint, ValueSchemaRemapError> {
        Ok(ScalarBindingCfgPoint {
            occurrence: binding.occurrence.clone(),
            output: binding.output,
            role: binding.role.clone(),
            point: self.point(producer, binding.point)?,
            source: match &binding.source {
                ScalarBindingOutputSource::Direct { occurrence, output_projection, output } => {
                    ScalarBindingOutputSource::Direct {
                        occurrence: occurrence.clone(),
                        output_projection: output_projection.clone(),
                        output: *output,
                    }
                }
                ScalarBindingOutputSource::Call { occurrence, output } => {
                    ScalarBindingOutputSource::Call {
                        occurrence: occurrence.clone(),
                        output: *output,
                    }
                }
                ScalarBindingOutputSource::Expression { occurrence } => {
                    ScalarBindingOutputSource::Expression {
                        occurrence: occurrence.clone(),
                    }
                }
                ScalarBindingOutputSource::Conditional { occurrence, then_block, output } => {
                    ScalarBindingOutputSource::Conditional {
                        occurrence: occurrence.clone(),
                        then_block: then_block.clone(),
                        output: *output,
                    }
                }
                ScalarBindingOutputSource::Block { occurrence, output } => {
                    ScalarBindingOutputSource::Block {
                        occurrence: occurrence.clone(),
                        output: *output,
                    }
                }
            },
        })
    }

    fn import(
        &self,
        producer: &ProducerCfgOwnerId,
    ) -> Result<&ValueImportCorrespondence, ValueSchemaRemapError> {
        if self.imports.iter().filter(|row| row.producer == *producer).count() != 1 {
            return Err(ValueSchemaRemapError::MissingImport(producer.clone()));
        }
        self.imports
            .iter()
            .find(|entry| entry.producer == *producer)
            .ok_or_else(|| ValueSchemaRemapError::MissingImport(producer.clone()))
    }

    fn binding(
        &self,
        producer: &ProducerCfgOwnerId,
        binding: &ReferenceBindingId,
    ) -> Result<ReferenceBindingId, ValueSchemaRemapError> {
        if self.import(producer)?.bindings.iter().filter(|(old, _)| old == binding).count() != 1 {
            return Err(ValueSchemaRemapError::MissingBinding(binding.clone()));
        }
        self.import(producer)?
            .bindings
            .iter()
            .find(|(old, _)| old == binding)
            .map(|(_, new)| new.clone())
            .ok_or_else(|| ValueSchemaRemapError::MissingBinding(binding.clone()))
    }

    fn checked_loan(&self, owner: &ProducerCfgOwnerId, loan: &ReferenceLoanId) -> Result<ReferenceLoanId, ValueSchemaRemapError> {
        let matches = self.import(owner)?.loans.iter().filter(|(old, _)| old == loan).collect::<Vec<_>>();
        let [(_, mapped)] = matches.as_slice() else { return Err(ValueSchemaRemapError::MissingLoan(loan.clone())); };
        Ok(mapped.clone())
    }

    fn checked_environment(&self, owner: &ProducerCfgOwnerId, environment: &CallableEnvironmentId) -> Result<CallableEnvironmentId, ValueSchemaRemapError> {
        let matches = self.import(owner)?.environments.iter().filter(|(old, _)| old == environment).collect::<Vec<_>>();
        let [(_, mapped)] = matches.as_slice() else { return Err(ValueSchemaRemapError::MissingEnvironment(environment.clone())); };
        Ok(mapped.clone())
    }

    fn checked_holder(&self, owner: &ProducerCfgOwnerId, holder: &ConnectedCaptureHolderFact) -> Result<ConnectedCaptureHolderFact, ValueSchemaRemapError> {
        let matches = self.import(owner)?.holders.iter().filter(|(old, _)| old == holder).collect::<Vec<_>>();
        let [(_, mapped)] = matches.as_slice() else { return Err(ValueSchemaRemapError::MissingHolder(holder.clone())); };
        Ok(mapped.clone())
    }

    fn checked_backing(&self, owner: &ProducerCfgOwnerId, backing: &RuntimeBackingFact) -> Result<RuntimeBackingFact, ValueSchemaRemapError> {
        let matches = self.import(owner)?.backings.iter().filter(|(old, _)| old == backing).collect::<Vec<_>>();
        let [(_, mapped)] = matches.as_slice() else { return Err(ValueSchemaRemapError::MissingBacking(backing.clone())); };
        Ok(mapped.clone())
    }

    fn place(
        &self,
        producer: &ProducerCfgOwnerId,
        place: &ReferencePlaceId,
    ) -> Result<ReferencePlaceId, ValueSchemaRemapError> {
        if self.import(producer)?.places.iter().filter(|(old, _)| old == place).count() != 1 {
            return Err(ValueSchemaRemapError::MissingPlace(place.clone()));
        }
        self.import(producer)?
            .places
            .iter()
            .find(|(old, _)| old == place)
            .map(|(_, new)| new.clone())
            .ok_or_else(|| ValueSchemaRemapError::MissingPlace(place.clone()))
    }

    fn origin(
        &self,
        producer: &ProducerCfgOwnerId,
        origin: ReferenceOriginId,
    ) -> Result<ReferenceOriginId, ValueSchemaRemapError> {
        self.import(producer)?
            .origins
            .get(origin.0)
            .copied()
            .ok_or_else(|| ValueSchemaRemapError::MissingOrigin {
                producer: producer.clone(),
                origin,
            })
    }

    fn runtime(
        &self,
        producer: &ProducerCfgOwnerId,
        value: &RuntimeValueId,
    ) -> Result<RuntimeValueId, ValueSchemaRemapError> {
        if self.import(producer)?.runtime_values.iter().filter(|(old, _)| old == value).count() != 1 {
            return Err(ValueSchemaRemapError::MissingRuntime(value.clone()));
        }
        self.import(producer)?
            .runtime_values
            .iter()
            .find(|(old, _)| old == value)
            .map(|(_, new)| new.clone())
            .ok_or_else(|| ValueSchemaRemapError::MissingRuntime(value.clone()))
    }

    fn projection(
        &self,
        projection: &ValueProjection,
    ) -> Result<ValueProjection, ValueSchemaRemapError> {
        Ok(match projection {
            ValueProjection::Field(field) => ValueProjection::Field(field.clone()),
            ValueProjection::VariantField { variant, field } => ValueProjection::VariantField {
                variant: *variant,
                field: *field,
            },
            ValueProjection::FixedIndex(index) => ValueProjection::FixedIndex(*index),
            ValueProjection::RuntimeIndex { selector } => ValueProjection::RuntimeIndex {
                selector: self.value(selector)?,
            },
        })
    }

    fn projections(
        &self,
        projections: &[ValueProjection],
    ) -> Result<Vec<ValueProjection>, ValueSchemaRemapError> {
        projections
            .iter()
            .map(|projection| self.projection(projection))
            .collect()
    }

    fn family(
        &self,
        family: &OwnerRecordFamilyId,
    ) -> Result<OwnerRecordFamilyId, ValueSchemaRemapError> {
        Ok(OwnerRecordFamilyId {
            producer: self.producer(&family.producer)?,
            creation: self.point(&family.producer, family.creation)?,
            projection: self.projections(&family.projection)?,
        })
    }

    fn fixed_lifetime(
        &self,
        lifetime: &FixedLifetime,
    ) -> Result<FixedLifetime, ValueSchemaRemapError> {
        Ok(match lifetime {
            FixedLifetime::StaticStorage(source) => FixedLifetime::StaticStorage(source.clone()),
            FixedLifetime::Capability(capability) => {
                FixedLifetime::Capability(self.capability(capability)?)
            }
        })
    }

    fn dependency_action(
        &self,
        action: &DependencyAction,
    ) -> Result<DependencyAction, ValueSchemaRemapError> {
        Ok(match action {
            DependencyAction::PromoteShared {
                source_owner,
                receiving_owner,
            } => DependencyAction::PromoteShared {
                source_owner: self.capability(source_owner)?,
                receiving_owner: self.capability(receiving_owner)?,
            },
            DependencyAction::TransferUnique {
                source_owner,
                receiving_owner,
            } => DependencyAction::TransferUnique {
                source_owner: self.capability(source_owner)?,
                receiving_owner: self.capability(receiving_owner)?,
            },
            DependencyAction::BorrowFixed {
                referent_lifetime,
                use_owner,
            } => DependencyAction::BorrowFixed {
                referent_lifetime: self.fixed_lifetime(referent_lifetime)?,
                use_owner: self.capability(use_owner)?,
            },
            DependencyAction::RegionMembership {
                region_owner,
                use_owner,
            } => DependencyAction::RegionMembership {
                region_owner: self.capability(region_owner)?,
                use_owner: self.capability(use_owner)?,
            },
            DependencyAction::ManualRaw { boundary } => DependencyAction::ManualRaw {
                boundary: boundary.clone(),
            },
        })
    }

    fn dependency(
        &self,
        producer: &ProducerCfgOwnerId,
        dependency: &EvaluatedDependencyFact,
    ) -> Result<EvaluatedDependencyFact, ValueSchemaRemapError> {
        let source = match &dependency.source {
            DependencySource::Allocation(family) => {
                DependencySource::Allocation(self.family(family)?)
            }
            DependencySource::Environment(environment) => DependencySource::Environment(self.checked_environment(producer, environment)?),
            DependencySource::BindingCell(binding) => {
                DependencySource::BindingCell(self.binding(producer, binding)?)
            }
            DependencySource::FixedReferent { origin, loan } => DependencySource::FixedReferent {
                origin: self.origin(producer, *origin)?,
                loan: self.checked_loan(producer, loan)?,
            },
            DependencySource::Region {
                instance,
                membership,
            } => DependencySource::Region {
                instance: self.binding(producer, instance)?,
                membership: self.runtime(producer, membership)?,
            },
            DependencySource::Manual { boundary } => DependencySource::Manual {
                boundary: boundary.clone(),
            },
        };
        Ok(EvaluatedDependencyFact {
            projection: self.projections(&dependency.projection)?,
            source,
            action: self.dependency_action(&dependency.action)?,
            witness: dependency.witness.clone(),
        })
    }

    fn availability(
        &self,
        producer: &ProducerCfgOwnerId,
        state: &ValueAvailability,
    ) -> Result<ValueAvailability, ValueSchemaRemapError> {
        Ok(match state {
            ValueAvailability::Available => ValueAvailability::Available,
            ValueAvailability::Uninitialized { source } => ValueAvailability::Uninitialized {
                source: source.clone(),
            },
            ValueAvailability::Moved {
                place,
                move_point,
                source,
            } => ValueAvailability::Moved {
                place: self.place(producer, place)?,
                move_point: self.point(producer, *move_point)?,
                source: source.clone(),
            },
            ValueAvailability::PartiallyUnavailable { states } => {
                ValueAvailability::PartiallyUnavailable {
                    states: states
                        .iter()
                        .map(|state| {
                            let availability = match &state.state {
                                SubplaceState::Available => SubplaceState::Available,
                                SubplaceState::Moved { point, source } => SubplaceState::Moved {
                                    point: self.point(producer, *point)?,
                                    source: source.clone(),
                                },
                                SubplaceState::Released { point, source } => {
                                    SubplaceState::Released {
                                        point: self.point(producer, *point)?,
                                        source: source.clone(),
                                    }
                                }
                            };
                            Ok(SubplaceAvailability {
                                projection: self.projections(&state.projection)?,
                                state: availability,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                }
            }
        })
    }

    fn payload(
        &self,
        producer: &ProducerCfgOwnerId,
        payload: &EvaluatedPayloadFact,
    ) -> Result<EvaluatedPayloadFact, ValueSchemaRemapError> {
        Ok(match payload {
            EvaluatedPayloadFact::Plain => EvaluatedPayloadFact::Plain,
            EvaluatedPayloadFact::Formal { port, .. } => return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone())),
            EvaluatedPayloadFact::Checked { origins, state } => {
                let state = match state {
                    CheckedValueState::Live => CheckedValueState::Live,
                    CheckedValueState::Null { source } => CheckedValueState::Null {
                        source: source.clone(),
                    },
                    CheckedValueState::Expired {
                        origin,
                        effect_point,
                    } => CheckedValueState::Expired {
                        origin: self.origin(producer, *origin)?,
                        effect_point: self.point(producer, *effect_point)?,
                    },
                };
                EvaluatedPayloadFact::Checked {
                    origins: origins
                        .iter()
                        .map(|origin| self.origin(producer, *origin))
                        .collect::<Result<BTreeSet<_>, ValueSchemaRemapError>>()?,
                    state,
                }
            }
            EvaluatedPayloadFact::Raw { provenance } => EvaluatedPayloadFact::Raw {
                provenance: match provenance {
                    RawValueProvenance::Static { source } => RawValueProvenance::Static {
                        source: source.clone(),
                    },
                    RawValueProvenance::CompilerStorage {
                        source_place,
                        creation,
                    } => RawValueProvenance::CompilerStorage {
                        source_place: self.place(producer, source_place)?,
                        creation: self.point(producer, *creation)?,
                    },
                    RawValueProvenance::ManualBoundary { operation } => {
                        RawValueProvenance::ManualBoundary {
                            operation: operation.clone(),
                        }
                    }
                },
            },
            EvaluatedPayloadFact::Callable { producers } => EvaluatedPayloadFact::Callable {
                producers: producers
                    .iter()
                    .map(|holder| self.checked_holder(producer, holder))
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
            EvaluatedPayloadFact::FixedAggregate { members } => {
                EvaluatedPayloadFact::FixedAggregate {
                    members: members
                        .iter()
                        .map(|member| {
                            Ok(EvaluatedMemberFact {
                                projection: self.projections(&member.projection)?,
                                value: self.value(&member.value)?,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                }
            }
            EvaluatedPayloadFact::RuntimeBacking { backing, elements } => {
                EvaluatedPayloadFact::RuntimeBacking {
                    backing: self.checked_backing(producer, backing)?,
                    elements: ElementViewFact {
                        element: self.shape(&elements.element)?,
                        backing: self.runtime(producer, &elements.backing)?,
                        selections: elements
                            .selections
                            .iter()
                            .map(|selection| {
                                Ok(EvaluatedElementSelection {
                                    selector: self.value(&selection.selector)?,
                                    case: self.case(&selection.case)?,
                                })
                            })
                            .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    },
                }
            }
        })
    }

    pub fn evaluated_case(
        &self,
        case: &EvaluatedValueCase,
    ) -> Result<EvaluatedValueCase, ValueSchemaRemapError> {
        let producer = &case.id.value.resolved.producer;
        Ok(EvaluatedValueCase {
            id: self.case(&case.id)?,
            path: self.path(&case.path)?,
            availability: self.availability(producer, &case.availability)?,
            payload: self.payload(producer, &case.payload)?,
            dependencies: case
                .dependencies
                .iter()
                .map(|dependency| self.dependency(producer, dependency))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    pub fn evaluated_fact(
        &self,
        value: &EvaluatedValueFact,
    ) -> Result<EvaluatedValueFact, ValueSchemaRemapError> {
        Ok(EvaluatedValueFact {
            id: self.value(&value.id)?,
            shape: self.shape(&value.shape)?,
            cases: value
                .cases
                .iter()
                .map(|case| self.evaluated_case(case))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    pub fn consumer(
        &self,
        consumer: &ValueConsumerId,
    ) -> Result<ValueConsumerId, ValueSchemaRemapError> {
        let kind = match &consumer.kind {
            ValueConsumerKind::Binding {
                binding,
                projection,
            } => ValueConsumerKind::Binding {
                binding: self.binding(&consumer.producer, binding)?,
                projection: self.projections(projection)?,
            },
            ValueConsumerKind::Replacement { operation, target } => {
                ValueConsumerKind::Replacement {
                    operation: self.operation(operation)?,
                    target: *target,
                }
            }
            ValueConsumerKind::PointerOperand { operation, target } => {
                ValueConsumerKind::PointerOperand {
                    operation: self.operation(operation)?,
                    target: *target,
                }
            }
            ValueConsumerKind::IndexOperand {
                operation,
                target,
                projection,
            } => ValueConsumerKind::IndexOperand {
                operation: self.operation(operation)?,
                target: *target,
                projection: *projection,
            },
            ValueConsumerKind::CallArgument {
                occurrence,
                argument,
            } => ValueConsumerKind::CallArgument {
                occurrence: occurrence.clone(),
                argument: *argument,
            },
            ValueConsumerKind::FinalOutput { block, output } => ValueConsumerKind::FinalOutput {
                block: block.clone(),
                output: *output,
            },
            ValueConsumerKind::AggregateMember { output, projection } => {
                ValueConsumerKind::AggregateMember {
                    output: self.output(output)?,
                    projection: self.projections(projection)?,
                }
            }
            ValueConsumerKind::BranchOutput { output, branch } => ValueConsumerKind::BranchOutput {
                output: self.output(output)?,
                branch: self.point(&consumer.producer, *branch)?,
            },
            ValueConsumerKind::Discard => ValueConsumerKind::Discard,
        };
        Ok(ValueConsumerId {
            producer: self.producer(&consumer.producer)?,
            point: self.point(&consumer.producer, consumer.point)?,
            kind,
        })
    }

    pub fn disposition(
        &self,
        producer: &ProducerCfgOwnerId,
        disposition: &ValueDisposition,
    ) -> Result<ValueDisposition, ValueSchemaRemapError> {
        let action = match &disposition.action {
            ValueDispositionAction::CopyTo(consumer) => {
                ValueDispositionAction::CopyTo(self.consumer(consumer)?)
            }
            ValueDispositionAction::MoveTo(consumer) => {
                ValueDispositionAction::MoveTo(self.consumer(consumer)?)
            }
            ValueDispositionAction::BorrowFor(consumer) => {
                ValueDispositionAction::BorrowFor(self.consumer(consumer)?)
            }
            ValueDispositionAction::DiscardAt(capability) => {
                ValueDispositionAction::DiscardAt(self.capability(capability)?)
            }
        };
        Ok(ValueDisposition {
            value: self.value(&disposition.value)?,
            case: self.case(&disposition.case)?,
            point: self.point(producer, disposition.point)?,
            action,
        })
    }

    fn previous_plan(
        &self,
        producer: &ProducerCfgOwnerId,
        plan: &AssignmentPreviousPlan,
    ) -> Result<AssignmentPreviousPlan, ValueSchemaRemapError> {
        let previous = match &plan.previous {
            PreviousOutputPlan::Existing { output, shape } => PreviousOutputPlan::Existing {
                output: self.output(output)?,
                shape: self.shape(shape)?,
            },
            PreviousOutputPlan::InferredDeclaration { binding } => {
                PreviousOutputPlan::InferredDeclaration {
                    binding: self.binding(producer, binding)?,
                }
            }
        };
        Ok(AssignmentPreviousPlan {
            target: plan.target,
            previous,
        })
    }

    pub fn assignment_plan(
        &self,
        plan: &ResolvedAssignmentPlan,
    ) -> Result<ResolvedAssignmentPlan, ValueSchemaRemapError> {
        let producer = &plan.operation.producer;
        let targets = plan
            .targets
            .iter()
            .map(|target| {
                let storage = match &target.storage {
                    ResolvedAssignmentStorage::Existing(place) => {
                        ResolvedAssignmentStorage::Existing(self.place(producer, place)?)
                    }
                    ResolvedAssignmentStorage::Through { pointer } => {
                        ResolvedAssignmentStorage::Through {
                            pointer: self.output(pointer)?,
                        }
                    }
                    ResolvedAssignmentStorage::InferredDeclaration(binding) => {
                        ResolvedAssignmentStorage::InferredDeclaration(
                            self.binding(producer, binding)?,
                        )
                    }
                };
                Ok(ResolvedAssignmentTarget {
                    ordinal: target.ordinal,
                    source_span: target.source_span.clone(),
                    place: target.place.clone(),
                    ty: target.ty.clone(),
                    storage,
                })
            })
            .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
        let consumption = match &plan.consumption {
            AssignmentConsumption::Discarded => AssignmentConsumption::Discarded,
            AssignmentConsumption::Received { outputs, discards } => {
                AssignmentConsumption::Received {
                    outputs: outputs
                        .iter()
                        .map(|output| self.consumer(output))
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    discards: discards
                        .iter()
                        .map(|output| self.output(output))
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                }
            }
        };
        Ok(ResolvedAssignmentPlan {
            operation: self.operation(&plan.operation)?,
            targets,
            replacements: plan.replacements.clone(),
            previous_outputs: plan
                .previous_outputs
                .iter()
                .map(|previous| self.previous_plan(producer, previous))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            consumption,
        })
    }

    fn destination_id(
        &self,
        id: &AssignmentDestinationId,
    ) -> Result<AssignmentDestinationId, ValueSchemaRemapError> {
        Ok(AssignmentDestinationId {
            operation: self.operation(&id.operation)?,
            target: id.target,
        })
    }

    pub fn destination(
        &self,
        destination: &EvaluatedAssignmentDestination,
    ) -> Result<EvaluatedAssignmentDestination, ValueSchemaRemapError> {
        let producer = &destination.id.operation.producer;
        let storage = match &destination.storage {
            EvaluatedDestinationStorage::Direct(place) => {
                EvaluatedDestinationStorage::Direct(self.place(producer, place)?)
            }
            EvaluatedDestinationStorage::Inferred(binding) => {
                EvaluatedDestinationStorage::Inferred(self.binding(producer, binding)?)
            }
            EvaluatedDestinationStorage::Through { pointer, cases } => {
                EvaluatedDestinationStorage::Through {
                    pointer: self.value(pointer)?,
                    cases: cases
                        .iter()
                        .map(|case| {
                            Ok(DestinationCase {
                                case: self.case(&case.case)?,
                                place: self.place(producer, &case.place)?,
                                access: match &case.access {
                                    ReferenceBorrowAccess::DirectStorage => {
                                        ReferenceBorrowAccess::DirectStorage
                                    }
                                    ReferenceBorrowAccess::Through {
                                        pointer_point,
                                        parent_origin,
                                    } => ReferenceBorrowAccess::Through {
                                        pointer_point: self.point(producer, *pointer_point)?,
                                        parent_origin: self.origin(producer, *parent_origin)?,
                                    },
                                },
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                }
            }
        };
        Ok(EvaluatedAssignmentDestination {
            id: self.destination_id(&destination.id)?,
            point: self.point(producer, destination.point)?,
            source_span: destination.source_span.clone(),
            shape: self.shape(&destination.shape)?,
            storage,
            selector_inputs: destination
                .selector_inputs
                .iter()
                .map(|input| self.value(input))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    pub fn snapshot(
        &self,
        snapshot: &AssignmentSnapshot,
    ) -> Result<AssignmentSnapshot, ValueSchemaRemapError> {
        let state = match &snapshot.state {
            SnapshotState::Saved(value) => SnapshotState::Saved(self.value(value)?),
            SnapshotState::NoPriorDeclaration => SnapshotState::NoPriorDeclaration,
            SnapshotState::ElidedDiscard { cases } => SnapshotState::ElidedDiscard {
                cases: cases
                    .iter()
                    .map(|case| self.evaluated_case(case))
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
            SnapshotState::Unavailable { cases } => SnapshotState::Unavailable {
                cases: cases
                    .iter()
                    .map(|case| self.evaluated_case(case))
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
        };
        Ok(AssignmentSnapshot {
            destination: self.destination_id(&snapshot.destination)?,
            point: self.point(&snapshot.destination.operation.producer, snapshot.point)?,
            state,
        })
    }

    pub fn commit(
        &self,
        commit: &AssignmentCommit,
    ) -> Result<AssignmentCommit, ValueSchemaRemapError> {
        let producer = &commit.destination.operation.producer;
        Ok(AssignmentCommit {
            destination: self.destination_id(&commit.destination)?,
            point: self.point(producer, commit.point)?,
            replacement: self.value(&commit.replacement)?,
            previous: self.point(producer, commit.previous)?,
            dispositions: commit
                .dispositions
                .iter()
                .map(|disposition| self.disposition(producer, disposition))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    pub fn result(
        &self,
        result: &AssignmentResult,
    ) -> Result<AssignmentResult, ValueSchemaRemapError> {
        Ok(AssignmentResult {
            operation: self.operation(&result.operation)?,
            output: result.output,
            snapshot: self.point(&result.operation.producer, result.snapshot)?,
            value: self.value(&result.value)?,
            consumers: result
                .consumers
                .iter()
                .map(|consumer| self.consumer(consumer))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    pub fn assignment_evaluation(
        &self,
        evaluation: &ScalarAssignmentEvaluation,
    ) -> Result<ScalarAssignmentEvaluation, ValueSchemaRemapError> {
        Ok(ScalarAssignmentEvaluation {
            operation: self.operation(&evaluation.operation)?,
            rhs: evaluation
                .rhs
                .iter()
                .map(|rhs| {
                    Ok(AssignmentRhsEvaluation {
                        expression_ordinal: rhs.expression_ordinal,
                        producer_output: rhs.producer_output,
                        value: self.value(&rhs.value)?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            destinations: evaluation
                .destinations
                .iter()
                .map(|destination| self.destination(destination))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            previous: evaluation
                .previous
                .iter()
                .map(|snapshot| self.snapshot(snapshot))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            writes: evaluation
                .writes
                .iter()
                .map(|commit| self.commit(commit))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            results: evaluation
                .results
                .iter()
                .map(|result| self.result(result))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    pub fn production(
        &self,
        production: &EvaluatedOutputProduction,
    ) -> Result<EvaluatedOutputProduction, ValueSchemaRemapError> {
        let producer = &production.producer;
        let source = match &production.source {
            OutputProductionSource::Direct => OutputProductionSource::Direct,
            OutputProductionSource::Call {
                occurrence,
                projections,
            } => OutputProductionSource::Call {
                occurrence: occurrence.clone(),
                projections: projections
                    .iter()
                    .map(|projection| {
                        Ok(ScalarCallOutputProjection {
                            output: projection.output,
                            point: self.point(producer, projection.point)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
            OutputProductionSource::Conditional { branch, inputs } => {
                OutputProductionSource::Conditional {
                    branch: self.point(producer, *branch)?,
                    inputs: inputs
                        .iter()
                        .map(|input| {
                            Ok(ConditionalOutputInput {
                                predecessor: self.point(producer, input.predecessor)?,
                                outputs: input
                                    .outputs
                                    .iter()
                                    .map(|value| self.value(value))
                                    .collect::<Result<Vec<_>, _>>()?,
                            })
                        })
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                }
            }
            OutputProductionSource::Block {
                occurrence,
                scope_exit,
                inputs,
            } => OutputProductionSource::Block {
                occurrence: occurrence.clone(),
                scope_exit: self.point(producer, *scope_exit)?,
                inputs: inputs
                    .iter()
                    .map(|value| self.value(value))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            OutputProductionSource::Assignment { operation } => {
                OutputProductionSource::Assignment {
                    operation: self.operation(operation)?,
                }
            }
        };
        Ok(EvaluatedOutputProduction {
            producer: self.producer(producer)?,
            occurrence: production.occurrence.clone(),
            output_projection: production.output_projection.clone(),
            evaluation_point: self.point(producer, production.evaluation_point)?,
            outputs: production
                .outputs
                .iter()
                .map(|value| self.value(value))
                .collect::<Result<Vec<_>, _>>()?,
            source,
        })
    }

    pub fn final_input(
        &self,
        input: &FinalOutputInput,
    ) -> Result<FinalOutputInput, ValueSchemaRemapError> {
        Ok(FinalOutputInput {
            consumer: self.consumer(&input.consumer)?,
            source: self.value(&input.source)?,
            receiving_type: input.receiving_type.clone(),
        })
    }

    fn map_table(
        &self,
        producer: &ProducerCfgOwnerId,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
        headers: &EvaluatedValueTables,
    ) -> Result<EvaluatedValueTables, ValueSchemaRemapError> {
        Ok(EvaluatedValueTables {
            source_literal_constructor_seeds: tables.source_literal_constructor_seeds.iter().map(|seed| {
                Ok(SourceLiteralConstructorSeedRef { event: self.source_event(&seed.event)?, child: self.producer(&seed.child)? })
            }).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_formal_values: self.source_formal_values(tables, correspondence)?,
            source_formal_reads: self.source_formal_reads(tables, correspondence)?,
            source_name_reads: tables.source_name_reads.iter().map(|fact| self.name_read(fact)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_origin_derivations: tables.source_origin_derivations.iter().map(|row| self.origin_derivation(row)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_capture_productions: self.capture_productions(tables, correspondence)?,
            source_capture_pending: tables.source_capture_pending.iter().map(|row| {
                Ok(SourceCapturePendingInput { event: self.source_event(&row.event)?, output: self.value(&row.output)?, child: self.producer(&row.child)? })
            }).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_capture_policy_instances: tables.source_capture_policy_instances.iter().map(|row| {
                Ok(SourceCapturePolicyInstance { event: self.source_event(&row.event)?, formal: correspondence.formal(&row.formal)?, substitutions: row.substitutions.clone(), ty: row.ty.clone(), policy: row.policy })
            }).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            block_output_aliases: tables.block_output_aliases.clone(),
            source_case_views: self.source_case_views(tables, correspondence)?,
            disposition_inputs: tables
                .disposition_inputs
                .iter()
                .map(|input| {
                    Ok(ResolvedValueDispositionInput {
                        source: self.value(&input.source)?,
                        consumer: self.consumer(&input.consumer)?,
                        policy: input.policy,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            owner_receiving_transitions: self.owner_receiving_transitions(tables)?,
            pending_owner_receiving: self.pending_owner_receiving(tables)?,
            source_move_inputs: self.source_move_inputs(tables)?,
            source_grant_creations: self.source_grant_creations(tables, correspondence)?,
            common_loans: self.common_loans(tables)?,
            staged_loan_queries: self.staged_loan_queries(tables, correspondence)?,
            source_use_graphs: self.source_use_graphs(tables, correspondence)?,
            source_backing_producers: tables
                .source_backing_producers
                .iter()
                .map(|fact| {
                    Ok(SourceBackingProducerFact {
                        event: self.source_event(&fact.event)?,
                        target: self.binding(&fact.event.owner, &fact.target)?,
                        value: self.value(&fact.value)?,
                        shape: self.shape(&fact.shape)?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_move_witnesses: self.source_move_witnesses(tables, correspondence)?,
            pending_source_moves: self.pending_source_moves(tables)?,
            source_port_monitors: self.source_port_monitors(tables, correspondence)?,
            source_port_equations: self.source_port_equations(tables, correspondence)?,
            source_saved_ports: self.source_saved_ports(tables, correspondence)?,
            owner_consumers: self.source_owner_consumers(tables)?,
            pending_owner_consumers: self.pending_owner_consumers(tables, correspondence)?,
            source_configurations: self.source_configurations(tables, correspondence)?,
            source_contributors: self.source_contributors(tables, correspondence)?,
            source_instances: self.source_instances(tables, correspondence)?,
            source_populations: self.source_populations(tables)?,
            source_views: self.source_views(tables, correspondence)?,
            source_paths: self.source_paths(tables)?,
            source_binding_shapes: tables
                .source_binding_shapes
                .iter()
                .map(|row| {
                    Ok(SourceBindingShapeFact {
                        binding: self.binding(producer, &row.binding)?,
                        shape: self.shape(&row.shape)?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_backing_observations: self.source_backings(tables)?,
            source_literal_members: tables
                .source_literal_members
                .iter()
                .map(|row| {
                    Ok(SourceLiteralMembersFact {
                        value: self.value(&row.value)?,
                        members: row
                            .members
                            .iter()
                            .map(|member| {
                                Ok(EvaluatedMemberFact {
                                    projection: self.projections(&member.projection)?,
                                    value: self.value(&member.value)?,
                                })
                            })
                            .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            raw_address_productions: tables
                .raw_address_productions
                .iter()
                .map(|production| {
                    Ok(RawAddressProduction {
                        value: self.value(&production.value)?,
                        operands: production.operands.iter().map(|operand| {
                            Ok(RawAddressOperandInput {
                                role: operand.role.clone(), path: operand.path.clone(),
                                expression: operand.expression.clone(), value: self.value(&operand.value)?,
                            })
                        }).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                        source: match &production.source {
                            RawAddressInputSource::CompilerStorage => {
                                RawAddressInputSource::CompilerStorage
                            }
                            RawAddressInputSource::ThroughPointer { pointer, place } => {
                                RawAddressInputSource::ThroughPointer {
                                    pointer: self.value(pointer)?,
                                    place: place.clone(),
                                }
                            }
                        },
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            owner_capabilities: tables
                .owner_capabilities
                .iter()
                .map(|owner| self.owner_fact(owner))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            source_slots: headers.source_slots.clone(),
            boundary_signatures: headers.boundary_signatures.clone(),
            call_argument_inputs: tables
                .call_argument_inputs
                .iter()
                .map(|input| {
                    Ok(CallArgumentInputs {
                        event: SourceEventId {
                            owner: self.producer(&input.event.owner)?,
                            point: self.point(&input.event.owner, input.event.point)?,
                        },
                        occurrence: input.occurrence.clone(),
                        target: self.boundary_target(&input.event.owner, &input.target)?,
                        arguments: input
                            .arguments
                            .iter()
                            .map(|argument| {
                                Ok(BoundaryArgumentInput {
                                    ordinal: argument.ordinal,
                                    source: self.value(&argument.source)?,
                                    shape: self.shape(&argument.shape)?,
                                })
                            })
                            .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            productions: tables
                .productions
                .iter()
                .map(|production| self.production(production))
                .collect::<Result<Vec<_>, _>>()?,
            final_inputs: tables
                .final_inputs
                .iter()
                .map(|input| self.final_input(input))
                .collect::<Result<Vec<_>, _>>()?,
            binding_inputs: tables
                .binding_inputs
                .iter()
                .map(|input| {
                    Ok(BindingOutputInput {
                        producer: self.producer(&input.producer)?,
                        consumer: self.point(&input.producer, input.consumer)?,
                        occurrence: input.occurrence.clone(),
                        output: input.output,
                        source: self.value(&input.source)?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            raw_address_inputs: tables
                .raw_address_inputs
                .iter()
                .map(|input| {
                    Ok(RawAddressSourceInput {
                        value: self.value(&input.value)?,
                        read: self.point(&input.value.resolved.producer, input.read)?,
                        place: self.place(&input.value.resolved.producer, &input.place)?,
                        pointee: self.shape(&input.pointee)?,
                    })
                })
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            assignment_plans: tables
                .assignment_plans
                .iter()
                .map(|plan| self.assignment_plan(plan))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            assignment_evaluations: tables
                .assignment_evaluations
                .iter()
                .map(|evaluation| self.assignment_evaluation(evaluation))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            shapes: tables
                .shapes
                .iter()
                .map(|shape| self.resolved_shape(shape))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            values: tables
                .values
                .iter()
                .map(|value| self.evaluated_fact_in_arena(value, tables, correspondence))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            dispositions: tables
                .dispositions
                .iter()
                .map(|disposition| self.disposition(producer, disposition))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            assignment_previous: tables
                .assignment_previous
                .iter()
                .map(|plan| self.previous_plan(producer, plan))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            destinations: tables
                .destinations
                .iter()
                .map(|destination| self.destination(destination))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            snapshots: tables
                .snapshots
                .iter()
                .map(|snapshot| self.snapshot(snapshot))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            commits: tables
                .commits
                .iter()
                .map(|commit| self.commit(commit))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            results: tables
                .results
                .iter()
                .map(|result| self.result(result))
                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }
}
