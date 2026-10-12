use super::*;
#[path = "epoch_capture_validation.rs"]
mod capture_validation;

#[path = "capture_declaration.rs"]
mod capture_declaration;

pub use capture_declaration::SourceCaptureDeclaration;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceEventId {
    pub owner: ProducerCfgOwnerId,
    pub point: CfgPointId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEvent {
    pub id: SourceEventId,
    pub source_span: SourceSpan,
    pub kind: SourceEventKind,
    pub successors: Vec<SourceEventId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceEventKind {
    Entry,
    ScopeEnter,
    ScopeExit,
    Evaluate,
    ValueOutputProjection,
    AssignmentDestination,
    AssignmentSnapshot,
    AssignmentResult,
    AssignmentDispose,
    Capture,
    Borrow,
    AllocationIdentityAddress,
    Bind,
    BindArrayElement,
    Backing,
    BackingPlace,
    CopyCallableFields,
    CopyProjectedBackings,
    SelectArrayElement,
    Assign,
    AssignThrough,
    Read,
    ReadPlace,
    Call,
    Move,
    DeferredMove,
    DeferredArrayElement,
    MoveThrough,
    DeferredMoveThrough,
    Release,
    Invalidate,
    Branch,
    Join,
    LoopTest,
    ReturnOutput,
    Return,
}

impl From<&CfgPointKind> for SourceEventKind {
    fn from(kind: &CfgPointKind) -> Self {
        match kind {
            CfgPointKind::Entry => Self::Entry,
            CfgPointKind::ScopeEnter => Self::ScopeEnter,
            CfgPointKind::ScopeExit => Self::ScopeExit,
            CfgPointKind::Evaluate(_) => Self::Evaluate,
            CfgPointKind::ValueOutputProjection { .. } => Self::ValueOutputProjection,
            CfgPointKind::AssignmentDestination { .. } => Self::AssignmentDestination,
            CfgPointKind::AssignmentSnapshot { .. } => Self::AssignmentSnapshot,
            CfgPointKind::AssignmentResult { .. } => Self::AssignmentResult,
            CfgPointKind::AssignmentDispose { .. } => Self::AssignmentDispose,
            CfgPointKind::Capture { .. } => Self::Capture,
            CfgPointKind::Borrow { .. } => Self::Borrow,
            CfgPointKind::AllocationIdentityAddress { .. } => Self::AllocationIdentityAddress,
            CfgPointKind::Bind { .. } => Self::Bind,
            CfgPointKind::BindArrayElement { .. } => Self::BindArrayElement,
            CfgPointKind::Backing { .. } => Self::Backing,
            CfgPointKind::BackingPlace { .. } => Self::BackingPlace,
            CfgPointKind::CopyCallableFields { .. } => Self::CopyCallableFields,
            CfgPointKind::CopyProjectedBackings { .. } => Self::CopyProjectedBackings,
            CfgPointKind::SelectArrayElement { .. } => Self::SelectArrayElement,
            CfgPointKind::Assign { .. } => Self::Assign,
            CfgPointKind::AssignThrough { .. } => Self::AssignThrough,
            CfgPointKind::Read { .. } => Self::Read,
            CfgPointKind::ReadPlace { .. } => Self::ReadPlace,
            CfgPointKind::Call { .. } => Self::Call,
            CfgPointKind::Move { .. } => Self::Move,
            CfgPointKind::DeferredMove { .. } => Self::DeferredMove,
            CfgPointKind::DeferredArrayElement { .. } => Self::DeferredArrayElement,
            CfgPointKind::MoveThrough { .. } => Self::MoveThrough,
            CfgPointKind::DeferredMoveThrough { .. } => Self::DeferredMoveThrough,
            CfgPointKind::Release { .. } => Self::Release,
            CfgPointKind::Invalidate { .. } => Self::Invalidate,
            CfgPointKind::Branch { .. } => Self::Branch,
            CfgPointKind::Join => Self::Join,
            CfgPointKind::LoopTest { .. } => Self::LoopTest,
            CfgPointKind::ReturnOutput { .. } => Self::ReturnOutput,
            CfgPointKind::Return => Self::Return,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceOutputPort {
    pub value: EvaluatedValueId,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCaptureTemplate {
    pub event: SourceEventId,
    pub literal_owner: ProducerCfgOwnerId,
    pub literal_span: SourceSpan,
    pub signature: ValueShapeId,
    pub inputs: Vec<SourceCapturePort>,
    pub parameters: Vec<SourceParameterPort>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCapturePort {
    pub ordinal: usize,
    pub name: String,
    pub binding: ReferenceBindingId,
    pub value_shape: ValueShapeId,
    pub place_shape: ValueShapeId,
    pub formal_shape: ValueShapeId,
    pub use_span: SourceSpan,
    pub place: ScalarPlace,
    pub storage: SourceCaptureStorage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceCaptureStorage {
    Binding(ReferenceBindingId),
    CheckedSlot(ReferenceBindingId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceParameterPort {
    pub ordinal: usize,
    pub name: String,
    pub binding: ReferenceBindingId,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEpoch {
    events: Vec<SourceEvent>,
    shapes: Vec<ResolvedValueShape>,
    outputs: Vec<SourceOutputPort>,
    captures: Vec<SourceCaptureTemplate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceEpochError {
    DuplicateOwner(ProducerCfgOwnerId),
    InvalidOriginalPointId {
        event: SourceEventId,
        expected: CfgPointId,
    },
    MissingOriginalSuccessor {
        event: SourceEventId,
        successor: SourceEventId,
    },
    DuplicateShape(ValueShapeId),
    DuplicateOutput(EvaluatedValueId),
    ForeignOutputOwner {
        value: EvaluatedValueId,
        owner: ProducerCfgOwnerId,
    },
    MissingOutputEvent {
        value: EvaluatedValueId,
        event: SourceEventId,
    },
    ForeignOutputSource {
        value: EvaluatedValueId,
        event: SourceEventId,
        source: SourceIdentity,
    },
    MissingOutputShape {
        value: EvaluatedValueId,
        shape: ValueShapeId,
    },
    MissingShapeReference {
        shape: ValueShapeId,
        target: ValueShapeId,
    },
    MissingCallableSignatureType {
        shape: ValueShapeId,
        ty: ScalarType,
    },
    AmbiguousCallableSignatureType {
        shape: ValueShapeId,
        ty: ScalarType,
        matches: Vec<ValueShapeId>,
    },
    UnresolvedShapeType {
        shape: ValueShapeId,
        ty: ScalarType,
    },
    InvalidCallableSignature {
        shape: ValueShapeId,
        signature: ScalarType,
    },
    MalformedCaptureLiteral {
        event: SourceEventId,
        source_span: SourceSpan,
        literal: ByteSpan,
        environment_literal: SourceSpan,
    },
    InvalidCaptureSignature {
        event: SourceEventId,
        signature: ScalarType,
    },
    CaptureParameterArity {
        event: SourceEventId,
        signature: ScalarType,
        names: usize,
        spans: usize,
        types: usize,
    },
    MissingLexicalOwner {
        event: SourceEventId,
        owner: ProducerCfgOwnerId,
    },
    DuplicateCaptureBinding {
        event: SourceEventId,
        binding: ReferenceBindingId,
    },
    MissingCheckedCaptureStorage {
        event: SourceEventId,
        binding: ReferenceBindingId,
    },
    ForeignCheckedCaptureStorage {
        event: SourceEventId,
        binding: ReferenceBindingId,
        storage_binding: ReferenceBindingId,
    },
    CaptureShapeConstruction {
        event: SourceEventId,
        producer: ProducerCfgOwnerId,
        ty: ScalarType,
        error: ValueShapeError,
    },
}

impl SourceEpoch {
    pub fn events(&self) -> &[SourceEvent] {
        &self.events
    }

    pub fn shapes(&self) -> &[ResolvedValueShape] {
        &self.shapes
    }

    pub fn outputs(&self) -> &[SourceOutputPort] {
        &self.outputs
    }

    pub fn captures(&self) -> &[SourceCaptureTemplate] {
        &self.captures
    }

    pub fn freeze(
        cfgs: &[&ScalarReferenceCfg],
        structs: &[ScalarStruct],
        enums: &[ScalarEnum],
        contexts: &[SourceCapturePolicyContext],
    ) -> Result<Self, SourceEpochError> {
        let mut epoch = Self {
            events: Vec::new(),
            shapes: Vec::new(),
            outputs: Vec::new(),
            captures: Vec::new(),
        };
        let mut owners = Vec::new();
        for cfg in cfgs {
            if owners.contains(&cfg.producer_owner) {
                return Err(SourceEpochError::DuplicateOwner(cfg.producer_owner.clone()));
            }
            owners.push(cfg.producer_owner.clone());
            for (index, point) in cfg.initial_points.iter().enumerate() {
                let id = SourceEventId {
                    owner: cfg.producer_owner.clone(),
                    point: point.id,
                };
                if point.id != CfgPointId(index) {
                    return Err(SourceEpochError::InvalidOriginalPointId {
                        event: id,
                        expected: CfgPointId(index),
                    });
                }
                let mut successors = Vec::new();
                for point in &point.successors {
                    let successor = SourceEventId {
                        owner: cfg.producer_owner.clone(),
                        point: *point,
                    };
                    if point.0 >= cfg.initial_points.len() {
                        return Err(SourceEpochError::MissingOriginalSuccessor {
                            event: id,
                            successor,
                        });
                    }
                    successors.push(successor);
                }
                epoch.events.push(SourceEvent {
                    id,
                    source_span: point.source_span.clone(),
                    kind: (&point.kind).into(),
                    successors,
                });
            }
            for shape in &cfg.evaluated_values.shapes {
                if epoch.shapes.iter().any(|row| row.id == shape.id) {
                    return Err(SourceEpochError::DuplicateShape(shape.id.clone()));
                }
                epoch.shapes.push(shape.clone());
            }
            for value in &cfg.evaluated_values.values {
                if epoch.outputs.iter().any(|row| row.value == value.id) {
                    return Err(SourceEpochError::DuplicateOutput(value.id.clone()));
                }
                if value.id.resolved.producer != cfg.producer_owner {
                    return Err(SourceEpochError::ForeignOutputOwner {
                        value: value.id.clone(),
                        owner: cfg.producer_owner.clone(),
                    });
                }
                let event = SourceEventId {
                    owner: cfg.producer_owner.clone(),
                    point: value.id.point,
                };
                let point = cfg.initial_points.get(value.id.point.0).ok_or_else(|| {
                    SourceEpochError::MissingOutputEvent {
                        value: value.id.clone(),
                        event: event.clone(),
                    }
                })?;
                if value.id.resolved.occurrence.source != point.source_span.source {
                    return Err(SourceEpochError::ForeignOutputSource {
                        value: value.id.clone(),
                        event,
                        source: point.source_span.source.clone(),
                    });
                }
                epoch.outputs.push(SourceOutputPort {
                    value: value.id.clone(),
                    shape: value.shape.clone(),
                });
            }
        }
        for cfg in cfgs { Self::lexical_context(&cfg.producer_owner, contexts)?; }
        epoch.construct_captures(cfgs, &owners, structs, enums, contexts)?;
        for output in &epoch.outputs {
            if !epoch.shapes.iter().any(|shape| shape.id == output.shape) {
                return Err(SourceEpochError::MissingOutputShape {
                    value: output.value.clone(),
                    shape: output.shape.clone(),
                });
            }
        }
        for shape in &epoch.shapes {
            let context = Self::lexical_context(&shape.id.producer, contexts)?;
            if !evaluated_value::template::valid_node(shape, context) {
                return Err(SourceEpochError::UnresolvedShapeType { shape: shape.id.clone(), ty: shape.ty.clone() });
            }
            if matches!(
                shape.ty,
                ScalarType::Qualified { .. } | ScalarType::Error
            ) {
                return Err(SourceEpochError::UnresolvedShapeType {
                    shape: shape.id.clone(),
                    ty: shape.ty.clone(),
                });
            }
            match &shape.payload {
                ValuePayloadShape::TemplateParameter { .. } => {}
                ValuePayloadShape::Plain(_) => {}
                ValuePayloadShape::Checked { referent, .. } => {
                    epoch.validate_child(shape, referent)?
                }
                ValuePayloadShape::Raw { pointee } => epoch.validate_child(shape, pointee)?,
                ValuePayloadShape::Struct { fields, .. } => {
                    for field in fields {
                        epoch.validate_child(shape, &field.shape)?;
                    }
                }
                ValuePayloadShape::Enum { variants, .. } => {
                    for variant in variants {
                        for field in &variant.fields {
                            epoch.validate_child(shape, field)?;
                        }
                    }
                }
                ValuePayloadShape::FixedArray { element, .. }
                | ValuePayloadShape::RuntimeArray { element } => {
                    epoch.validate_child(shape, element)?
                }
                ValuePayloadShape::Callable { signature } => match signature {
                    ScalarType::Callable {
                        outputs,
                        parameters,
                    } => {
                        for output in &outputs.outputs {
                            epoch.validate_signature_type(shape, &output.ty)?;
                        }
                        for parameter in parameters {
                            epoch.validate_signature_type(shape, parameter)?;
                        }
                    }
                    _ => {
                        return Err(SourceEpochError::InvalidCallableSignature {
                            shape: shape.id.clone(),
                            signature: signature.clone(),
                        })
                    }
                },
            }
        }
        epoch.validate_template_graph(contexts)?;
        epoch.validate_capture_closure(structs, enums, contexts)?;
        Ok(epoch)
    }

    fn validate_child(
        &self,
        shape: &ResolvedValueShape,
        target: &ValueShapeId,
    ) -> Result<(), SourceEpochError> {
        if self.shapes.iter().any(|row| row.id == *target) {
            return Ok(());
        }
        Err(SourceEpochError::MissingShapeReference {
            shape: shape.id.clone(),
            target: target.clone(),
        })
    }

    fn validate_signature_type(
        &self,
        shape: &ResolvedValueShape,
        ty: &ScalarType,
    ) -> Result<(), SourceEpochError> {
        let matches = self
            .shapes
            .iter()
            .filter(|row| row.id.producer == shape.id.producer && row.ty == *ty)
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Err(SourceEpochError::MissingCallableSignatureType {
                shape: shape.id.clone(),
                ty: ty.clone(),
            }),
            1 => Ok(()),
            _ => Err(SourceEpochError::AmbiguousCallableSignatureType {
                shape: shape.id.clone(),
                ty: ty.clone(),
                matches,
            }),
        }
    }
}

#[cfg(test)]
#[path = "source_epoch_tests.rs"]
mod tests;

#[path = "source_capture.rs"]
mod source_capture;

#[cfg(test)]
#[path = "source_capture_tests.rs"]
mod capture_tests;
