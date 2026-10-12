use super::*;
pub(super) mod parent_seed;
pub use parent_seed::{SourceConstructorOriginRef, SourceParentSeedOrigin};
pub(in crate::scalar) mod null_constructor;
pub(in crate::scalar) mod checked_parameter_constructor;
pub(in crate::scalar) mod direct_storage_constructor;
pub(in crate::scalar) mod allocation_identity_constructor;
pub(in crate::scalar) mod checked_pointer_cast_constructor;
pub use null_constructor::SourceNullConstructor;
mod literal_seed;
pub use literal_seed::{SourceLiteralConstructorSeed, SourceLiteralConstructorSeedRef, SourceLiteralSeedBorrowCreation, SourceLiteralSeedParentSource};
pub(super) mod module_source_topology;
pub(super) mod module_resolver_input;
pub use module_resolver_input::SourceModuleResolverInputState;
pub use reference::source_epoch::SourceEventId;
pub(super) mod formal_value;
pub use formal_value::{SourceFormalMemberPort, SourceFormalOwnerRole, SourceFormalParameterContract, SourceFormalReadWitness, SourceFormalValue};
mod capture_inventory;
pub use capture_inventory::*;
mod construction_forest;
pub use construction_forest::*;
pub(super) mod coordinate;
mod capture_construction;
pub(super) use capture_construction::*;
mod capture_preflight;
pub(super) use capture_preflight::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BoundaryPortRole {
    Parameter {
        ordinal: usize,
        binding: ReferenceBindingId,
    },
    Capture {
        ordinal: usize,
        binding: ReferenceBindingId,
    },
    Global(ReferenceBindingId),
    Output(usize),
    Effect {
        binding: ReferenceBindingId,
        projection: Vec<ReferencePlaceProjection>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSlotRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSlotFact {
    pub id: SourceSlotRef,
    pub role: SourceCoordinateRole,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundaryFormalPort {
    pub role: BoundaryPortRole,
    pub shape: ValueShapeId,
    pub coordinate: SourceSlotRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundaryFormalPortRef {
    pub owner: ProducerCfgOwnerId,
    pub port: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFormalValueRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundarySignatureFact {
    pub owner: ProducerCfgOwnerId,
    pub entry: SourceEventId,
    pub frame: FrameVariableRef,
    pub ports: Vec<BoundaryFormalPort>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundaryArgumentInput {
    pub ordinal: usize,
    pub source: EvaluatedValueId,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CallArgumentInputs {
    pub event: SourceEventId,
    pub occurrence: ScalarCallOccurrence,
    pub target: ResolvedCallTarget,
    pub arguments: Vec<BoundaryArgumentInput>,
}

#[cfg_attr(test, track_caller)]
fn invalid(source: &SourceSpan) -> ReferenceAnalysisError {
    let error = ReferenceAnalysisError::MissingContext {
        source_span: source.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    #[cfg(test)]
    selected_source::snapshot23_observation::constructed(1, std::panic::Location::caller(), &error);
    error
}

impl reference::ReferenceFlowBuilder {
    pub(super) fn register_boundary_signature(
        &mut self,
        signature: &ScalarType,
        captures: &[closure::ClosureCapture],
        globals: &BTreeMap<String, (ReferenceBindingId, ScalarType)>,
    ) -> Result<(), ReferenceAnalysisError> {
        let entries = self
            .cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Entry))
            .collect::<Vec<_>>();
        let [entry] = entries.as_slice() else {
            return Err(invalid(&self.cfg.points[0].source_span));
        };
        let source = entry.source_span.clone();
        let entry = SourceEventId {
            owner: self.cfg.producer_owner.clone(),
            point: entry.id,
        };
        let ScalarType::Callable {
            parameters,
            outputs,
        } = signature
        else {
            return Err(invalid(&source));
        };
        if parameters.len() != self.cfg.reference_parameters.len()
            || !self.cfg.evaluated_values.boundary_signatures.is_empty()
        {
            return Err(invalid(&source));
        }
        let mut declarations = parameters
            .iter()
            .zip(&self.cfg.reference_parameters)
            .enumerate()
            .map(|(ordinal, (ty, binding))| {
                (
                    BoundaryPortRole::Parameter {
                        ordinal,
                        binding: binding.clone(),
                    },
                    ty.clone(),
                )
            })
            .collect::<Vec<_>>();
        declarations.extend(captures.iter().enumerate().map(|(ordinal, capture)| {
            (
                BoundaryPortRole::Capture {
                    ordinal,
                    binding: capture.binding.clone(),
                },
                capture.ty.clone(),
            )
        }));
        for (binding, ty) in globals.values() {
            if self
                .cfg
                .points
                .iter()
                .any(|point| uses_global(&self.cfg, &point.kind, binding))
            {
                declarations.push((BoundaryPortRole::Global(binding.clone()), ty.clone()));
            }
        }
        for destination in &self.cfg.evaluated_values.destinations {
            let EvaluatedDestinationStorage::Direct(place) = &destination.storage else {
                continue;
            };
            if !globals
                .values()
                .any(|(binding, _)| binding == &place.binding)
            {
                continue;
            }
            let role = BoundaryPortRole::Effect {
                binding: place.binding.clone(),
                projection: place.projections.clone(),
            };
            if declarations.iter().any(|(previous, _)| previous == &role) {
                continue;
            }
            let shapes = self
                .cfg
                .evaluated_values
                .shapes
                .iter()
                .filter(|shape| shape.id == destination.shape)
                .collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else {
                return Err(invalid(&destination.source_span));
            };
            declarations.push((role, shape.ty.clone()));
        }
        declarations.extend(
            outputs
                .outputs
                .iter()
                .enumerate()
                .map(|(ordinal, output)| (BoundaryPortRole::Output(ordinal), output.ty.clone())),
        );
        let mut ports = Vec::new();
        for (role, ty) in declarations {
            let shape = intern_value_shape(
                &mut self.cfg.evaluated_values.shapes,
                &self.cfg.producer_owner,
                &ty,
                &self.structs,
                &self.enums,
                &self.generic_scope,
                &self.generic_declarations,
            )
            .map_err(|_| invalid(&source))?;
            let coordinate = SourceSlotRef {
                owner: self.cfg.producer_owner.clone(),
                index: self.cfg.evaluated_values.source_slots.len(),
            };
            self.cfg.evaluated_values.source_slots.push(SourceSlotFact {
                id: coordinate.clone(),
                role: SourceCoordinateRole::Boundary(role.clone()),
                shape: shape.clone(),
            });
            ports.push(BoundaryFormalPort {
                role,
                shape,
                coordinate,
            });
        }
        self.cfg
            .evaluated_values
            .boundary_signatures
            .push(BoundarySignatureFact {
                owner: self.cfg.producer_owner.clone(),
                entry,
                frame: FrameVariableRef {
                    owner: self.cfg.producer_owner.clone(),
                    variable: SupportedInstanceVariable(0),
                },
                ports,
            });
        validate_signatures(&self.cfg)
    }

    pub(super) fn register_call_argument_inputs(
        &mut self,
        expression: &ScalarExpression,
        anchor: CfgPointId,
        arguments: Vec<BoundaryArgumentInput>,
    ) -> Result<(), ReferenceAnalysisError> {
        let ScalarExpression::Call { occurrence, .. } = expression else {
            return Err(invalid(&self.cfg.points[anchor.0].source_span));
        };
        let CfgPointKind::Call {
            target, output: 0, ..
        } = &self.cfg.points[anchor.0].kind
        else {
            return Err(invalid(&self.cfg.points[anchor.0].source_span));
        };
        let row = CallArgumentInputs {
            event: SourceEventId {
                owner: self.cfg.producer_owner.clone(),
                point: anchor,
            },
            occurrence: occurrence.as_ref().clone(),
            target: target.clone(),
            arguments,
        };
        validate_call_arguments(&self.cfg, &row)?;
        if self
            .cfg
            .evaluated_values
            .call_argument_inputs
            .iter()
            .any(|known| known.event == row.event)
        {
            return Err(invalid(&self.cfg.points[anchor.0].source_span));
        }
        self.cfg.evaluated_values.call_argument_inputs.push(row);
        Ok(())
    }
}

fn uses_global(cfg: &ScalarReferenceCfg, kind: &CfgPointKind, binding: &ReferenceBindingId) -> bool {
    match kind {
        CfgPointKind::Read { binding: actual }
        | CfgPointKind::Bind { binding: actual }
        | CfgPointKind::Backing { target: actual, .. } => actual == binding,
        CfgPointKind::ReadPlace { candidates, .. } => {
            candidates.iter().any(|place| &place.binding == binding)
        }
        CfgPointKind::Assign { target, .. }
        | CfgPointKind::BackingPlace { target, .. }
        | CfgPointKind::BindArrayElement { target, .. }
        | CfgPointKind::SelectArrayElement { target, .. } => &target.binding == binding,
        CfgPointKind::CopyProjectedBackings { source, target } => {
            source == binding || &target.binding == binding
        }
        CfgPointKind::CopyCallableFields { source, target } => {
            target == binding || match source {
                CallableFieldSource::Assignment(source) => source == binding,
                CallableFieldSource::Evaluated(source) => uses_global(cfg, &cfg.points[source.point.0].kind, binding),
            }
        }
        CfgPointKind::Capture { captures, .. } => {
            captures.iter().any(|capture| &capture.binding == binding)
        }
        CfgPointKind::Call {
            target: ResolvedCallTarget::LocalCallable(actual),
            ..
        } => actual == binding,
        CfgPointKind::Call {
            target: ResolvedCallTarget::CallableField { receiver_place, .. },
            ..
        } => &receiver_place.binding == binding,
        CfgPointKind::Move { place, .. } | CfgPointKind::DeferredMove { place, .. } => {
            &place.binding == binding
        }
        _ => false,
    }
}

pub(super) fn validate_signatures(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    for signature in &cfg.evaluated_values.boundary_signatures {
        let point = cfg
            .points
            .get(signature.entry.point.0)
            .ok_or_else(|| invalid(&cfg.points[0].source_span))?;
        if signature.owner != cfg.producer_owner
            || signature.entry.owner != signature.owner
            || signature.frame.owner != signature.owner
            || !matches!(point.kind, CfgPointKind::Entry)
            || cfg
                .evaluated_values
                .boundary_signatures
                .iter()
                .filter(|row| row.owner == signature.owner)
                .count()
                != 1
            || signature
                .ports
                .iter()
                .filter(|port| matches!(port.role, BoundaryPortRole::Parameter { .. }))
                .count()
                != cfg.reference_parameters.len()
        {
            return Err(invalid(&point.source_span));
        }
        for (index, port) in signature.ports.iter().enumerate() {
            let slots = cfg
                .evaluated_values
                .source_slots
                .iter()
                .filter(|slot| slot.id == port.coordinate)
                .collect::<Vec<_>>();
            let [slot] = slots.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            if slot.id.owner != signature.owner
                || slot.role != SourceCoordinateRole::Boundary(port.role.clone())
                || slot.shape != port.shape
                || cfg.evaluated_values.source_slots.get(slot.id.index) != Some(slot)
                || port.shape.producer != signature.owner
                || !cfg
                    .evaluated_values
                    .shapes
                    .iter()
                    .any(|shape| shape.id == port.shape)
                || signature.ports[..index].iter().any(|previous| {
                    previous.role == port.role || previous.coordinate == port.coordinate
                })
            {
                return Err(invalid(&point.source_span));
            }
            if let BoundaryPortRole::Parameter { ordinal, binding } = &port.role {
                if cfg.reference_parameters.get(*ordinal) != Some(binding) {
                    return Err(invalid(&point.source_span));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_call_arguments(
    cfg: &ScalarReferenceCfg,
    input: &CallArgumentInputs,
) -> Result<(), ReferenceAnalysisError> {
    let point = cfg
        .points
        .get(input.event.point.0)
        .ok_or_else(|| invalid(&cfg.points[0].source_span))?;
    let CfgPointKind::Call {
        expression: ScalarExpression::Call { occurrence, .. },
        output: 0,
        target,
        argument_points,
    } = &point.kind
    else {
        return Err(invalid(&point.source_span));
    };
    if input.event.owner != cfg.producer_owner
        || input.occurrence != **occurrence
        || &input.target != target
        || input.arguments.len() != argument_points.len()
        || point.source_span != input.occurrence.source_span
    {
        return Err(invalid(&point.source_span));
    }
    for (ordinal, (argument, actual)) in input.arguments.iter().zip(argument_points).enumerate() {
        let values = cfg
            .evaluated_values
            .values
            .iter()
            .filter(|value| value.id == argument.source)
            .collect::<Vec<_>>();
        let [value] = values.as_slice() else {
            return Err(invalid(&point.source_span));
        };
        if argument.ordinal != ordinal
            || argument.source.point != *actual
            || value.shape != argument.shape
            || argument.source.resolved.producer != cfg.producer_owner
            || argument.shape.producer != cfg.producer_owner
            || argument.source.point >= point.id
            || !cfg
                .evaluated_values
                .shapes
                .iter()
                .any(|shape| shape.id == argument.shape)
        {
            return Err(invalid(&point.source_span));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
