use super::*;
pub(super) mod template;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedValueOutputId {
    pub producer: ProducerCfgOwnerId,
    pub occurrence: SourceSpan,
    pub output_projection: Vec<usize>,
    pub output: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedValueId {
    pub resolved: ResolvedValueOutputId,
    pub point: CfgPointId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueShapeId {
    pub producer: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueCaseId {
    pub value: EvaluatedValueId,
    pub alternative: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedValueShape {
    pub id: ValueShapeId,
    pub ty: ScalarType,
    pub payload: ValuePayloadShape,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValuePayloadShape {
    TemplateParameter {
        scope: ProducerCfgOwnerId,
        declaration: SourceSpan,
    },
    Plain(PlainValueKind),
    Checked {
        mutability: ScalarReferenceMutability,
        referent: ValueShapeId,
    },
    Raw {
        pointee: ValueShapeId,
    },
    Callable {
        signature: ScalarType,
    },
    Struct {
        structure: ScalarStructId,
        fields: Vec<ValueShapeField>,
    },
    Enum {
        enumeration: ScalarEnumId,
        variants: Vec<ValueShapeVariant>,
    },
    FixedArray {
        element: ValueShapeId,
        length: u64,
    },
    RuntimeArray {
        element: ValueShapeId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PlainValueKind {
    Unit,
    Bool,
    Char,
    ArtifactId,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueShapeField {
    pub field: ScalarStructFieldId,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueShapeVariant {
    pub ordinal: usize,
    pub fields: Vec<ValueShapeId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedValueFact {
    pub id: EvaluatedValueId,
    pub shape: ValueShapeId,
    pub cases: Vec<EvaluatedValueCase>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedValueCase {
    pub id: ValueCaseId,
    pub path: EvaluatedControlPathId,
    pub availability: ValueAvailability,
    pub payload: EvaluatedPayloadFact,
    pub dependencies: Vec<EvaluatedDependencyFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedControlPathId {
    pub producer: ProducerCfgOwnerId,
    pub ordinal: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValueAvailability {
    Available,
    Uninitialized {
        source: SourceSpan,
    },
    Moved {
        place: ReferencePlaceId,
        move_point: CfgPointId,
        source: SourceSpan,
    },
    PartiallyUnavailable {
        states: Vec<SubplaceAvailability>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SubplaceAvailability {
    pub projection: Vec<ValueProjection>,
    pub state: SubplaceState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SubplaceState {
    Available,
    Moved {
        point: CfgPointId,
        source: SourceSpan,
    },
    Released {
        point: CfgPointId,
        source: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValueProjection {
    Field(ScalarStructFieldId),
    VariantField { variant: usize, field: usize },
    FixedIndex(u64),
    RuntimeIndex { selector: EvaluatedValueId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EvaluatedPayloadFact {
    Plain,
    Formal { port: BoundaryFormalPortRef, view: SourceFormalValueRef },
    Checked {
        origins: BTreeSet<ReferenceOriginId>,
        state: CheckedValueState,
    },
    Raw {
        provenance: RawValueProvenance,
    },
    Callable {
        producers: Vec<ConnectedCaptureHolderFact>,
    },
    FixedAggregate {
        members: Vec<EvaluatedMemberFact>,
    },
    RuntimeBacking {
        backing: RuntimeBackingFact,
        elements: ElementViewFact,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CheckedValueState {
    Live,
    Null {
        source: SourceSpan,
    },
    Expired {
        origin: ReferenceOriginId,
        effect_point: CfgPointId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RawValueProvenance {
    Static {
        source: SourceSpan,
    },
    CompilerStorage {
        source_place: ReferencePlaceId,
        creation: CfgPointId,
    },
    ManualBoundary {
        operation: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedMemberFact {
    pub projection: Vec<ValueProjection>,
    pub value: EvaluatedValueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ElementViewFact {
    pub element: ValueShapeId,
    pub backing: RuntimeValueId,
    pub selections: Vec<EvaluatedElementSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedElementSelection {
    pub selector: EvaluatedValueId,
    pub case: ValueCaseId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerCapabilityId {
    pub producer: ProducerCfgOwnerId,
    pub introduction: CfgPointId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerRecordFamilyId {
    pub producer: ProducerCfgOwnerId,
    pub creation: CfgPointId,
    pub projection: Vec<ValueProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FixedLifetime {
    StaticStorage(SourceSpan),
    Capability(OwnerCapabilityId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DependencyAction {
    PromoteShared {
        source_owner: OwnerCapabilityId,
        receiving_owner: OwnerCapabilityId,
    },
    TransferUnique {
        source_owner: OwnerCapabilityId,
        receiving_owner: OwnerCapabilityId,
    },
    BorrowFixed {
        referent_lifetime: FixedLifetime,
        use_owner: OwnerCapabilityId,
    },
    RegionMembership {
        region_owner: OwnerCapabilityId,
        use_owner: OwnerCapabilityId,
    },
    ManualRaw {
        boundary: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedDependencyFact {
    pub projection: Vec<ValueProjection>,
    pub source: DependencySource,
    pub action: DependencyAction,
    pub witness: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DependencySource {
    Allocation(OwnerRecordFamilyId),
    Environment(CallableEnvironmentId),
    BindingCell(ReferenceBindingId),
    FixedReferent {
        origin: ReferenceOriginId,
        loan: ReferenceLoanId,
    },
    Region {
        instance: ReferenceBindingId,
        membership: RuntimeValueId,
    },
    Manual {
        boundary: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueDisposition {
    pub value: EvaluatedValueId,
    pub case: ValueCaseId,
    pub point: CfgPointId,
    pub action: ValueDispositionAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValueDispositionAction {
    CopyTo(ValueConsumerId),
    MoveTo(ValueConsumerId),
    BorrowFor(ValueConsumerId),
    DiscardAt(OwnerCapabilityId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueConsumerId {
    pub producer: ProducerCfgOwnerId,
    pub point: CfgPointId,
    pub kind: ValueConsumerKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValueConsumerKind {
    Binding {
        binding: ReferenceBindingId,
        projection: Vec<ValueProjection>,
    },
    Replacement {
        operation: AssignmentOperationId,
        target: usize,
    },
    PointerOperand {
        operation: AssignmentOperationId,
        target: usize,
    },
    IndexOperand {
        operation: AssignmentOperationId,
        target: usize,
        projection: usize,
    },
    CallArgument {
        occurrence: ScalarCallOccurrence,
        argument: usize,
    },
    FinalOutput {
        block: ScalarBlockOccurrence,
        output: usize,
    },
    AggregateMember {
        output: ResolvedValueOutputId,
        projection: Vec<ValueProjection>,
    },
    BranchOutput {
        output: ResolvedValueOutputId,
        branch: CfgPointId,
    },
    Discard,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedOutputProduction {
    pub producer: ProducerCfgOwnerId,
    pub occurrence: SourceSpan,
    pub output_projection: Vec<usize>,
    pub evaluation_point: CfgPointId,
    pub outputs: Vec<EvaluatedValueId>,
    pub source: OutputProductionSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OutputProductionSource {
    Direct,
    Call {
        occurrence: ScalarCallOccurrence,
        projections: Vec<ScalarCallOutputProjection>,
    },
    Conditional {
        branch: CfgPointId,
        inputs: Vec<ConditionalOutputInput>,
    },
    Block {
        occurrence: ScalarBlockOccurrence,
        scope_exit: CfgPointId,
        inputs: Vec<EvaluatedValueId>,
    },
    Assignment {
        operation: AssignmentOperationId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConditionalOutputInput {
    pub predecessor: CfgPointId,
    pub outputs: Vec<EvaluatedValueId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockOutputAlias {
    pub selected: ScalarBlockOccurrence,
    pub source: ScalarBlockOccurrence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FinalOutputInput {
    pub consumer: ValueConsumerId,
    pub source: EvaluatedValueId,
    pub receiving_type: ScalarType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedValueDispositionInput {
    pub source: EvaluatedValueId,
    pub consumer: ValueConsumerId,
    pub policy: CopyPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BindingOutputInput {
    pub producer: ProducerCfgOwnerId,
    pub consumer: CfgPointId,
    pub occurrence: ScalarBindingOccurrence,
    pub output: usize,
    pub source: EvaluatedValueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RawAddressSourceInput {
    pub value: EvaluatedValueId,
    pub read: CfgPointId,
    pub place: ReferencePlaceId,
    pub pointee: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RawAddressInputSource {
    CompilerStorage,
    ThroughPointer {
        pointer: EvaluatedValueId,
        place: ScalarPlace,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RawAddressProduction {
    pub value: EvaluatedValueId,
    pub source: RawAddressInputSource,
    pub operands: Vec<RawAddressOperandInput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RawAddressOperandRole {
    Pointer,
    Index,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RawAddressOperandStep {
    Base,
    Pointer,
    Index,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RawAddressOperandInput {
    pub role: RawAddressOperandRole,
    pub path: Vec<RawAddressOperandStep>,
    pub expression: ScalarExpression,
    pub value: EvaluatedValueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedValueTables {
    pub source_literal_constructor_seeds: Vec<SourceLiteralConstructorSeedRef>,
    pub source_formal_values: Vec<SourceFormalValue>,
    pub source_formal_reads: Vec<SourceFormalReadWitness>,
    pub source_name_reads: Vec<SourceNameReadFact>,
    pub source_origin_derivations: Vec<source_boundary::null_constructor::SourceConstructorOriginDerivation>,
    pub source_capture_productions: Vec<SourceCaptureProduction>,
    pub source_capture_pending: Vec<SourceCapturePendingInput>,
    pub source_capture_policy_instances: Vec<SourceCapturePolicyInstance>,
    pub source_case_views: Vec<SourceCaseViewFact>,
    pub disposition_inputs: Vec<ResolvedValueDispositionInput>,
    pub source_move_inputs: Vec<SourceMoveInput>,
    pub source_backing_producers: Vec<SourceBackingProducerFact>,
    pub source_grant_creations: Vec<SourceGrantCreationWitness>,
    pub common_loans: Vec<CommonLoanAnalysis>,
    pub staged_loan_queries: Vec<StagedLoanQueryAnalysis>,
    pub source_use_graphs: Vec<SourceUseGraph>,
    pub source_move_witnesses: Vec<SourceMoveWitnessFact>,
    pub pending_source_moves: Vec<SourceMovePendingFact>,
    pub owner_receiving_transitions: Vec<OwnerReceivingTransitionFact>,
    pub pending_owner_receiving: Vec<OwnerReceivingPendingFact>,
    pub source_port_monitors: Vec<JointPortMonitorFact>,
    pub source_port_equations: Vec<JointPortMonitorEquation>,
    pub source_saved_ports: Vec<SourceSavedPortFact>,
    pub owner_consumers: Vec<EvaluatedOwnerConsumer>,
    pub pending_owner_consumers: Vec<PendingOwnerConsumer>,
    pub source_configurations: Vec<SourceConfigurationFact>,
    pub source_contributors: Vec<JointProducerTupleFact>,
    pub source_instances: Vec<SourceSupportedClassFact>,
    pub source_populations: Vec<SourcePopulationFact>,
    pub source_views: Vec<SourceValueViewFact>,
    pub source_paths: Vec<EvaluatedPathFact>,
    pub source_binding_shapes: Vec<SourceBindingShapeFact>,
    pub source_backing_observations: Vec<SourceBackingObservation>,
    pub source_literal_members: Vec<SourceLiteralMembersFact>,
    pub productions: Vec<EvaluatedOutputProduction>,
    pub block_output_aliases: Vec<BlockOutputAlias>,
    pub final_inputs: Vec<FinalOutputInput>,
    pub binding_inputs: Vec<BindingOutputInput>,
    pub raw_address_inputs: Vec<RawAddressSourceInput>,
    pub raw_address_productions: Vec<RawAddressProduction>,
    pub source_slots: Vec<SourceSlotFact>,
    pub boundary_signatures: Vec<BoundarySignatureFact>,
    pub call_argument_inputs: Vec<CallArgumentInputs>,
    pub owner_capabilities: Vec<OwnerCapabilityFact>,
    pub assignment_plans: Vec<ResolvedAssignmentPlan>,
    pub assignment_evaluations: Vec<ScalarAssignmentEvaluation>,
    pub shapes: Vec<ResolvedValueShape>,
    pub values: Vec<EvaluatedValueFact>,
    pub dispositions: Vec<ValueDisposition>,
    pub assignment_previous: Vec<AssignmentPreviousPlan>,
    pub destinations: Vec<EvaluatedAssignmentDestination>,
    pub snapshots: Vec<AssignmentSnapshot>,
    pub commits: Vec<AssignmentCommit>,
    pub results: Vec<AssignmentResult>,
}

impl EvaluatedValueTables {
    pub fn new() -> Self {
        Self {
            source_literal_constructor_seeds: Vec::new(),
            source_formal_values: Vec::new(),
            source_formal_reads: Vec::new(),
            source_name_reads: Vec::new(),
            source_origin_derivations: Vec::new(),
            source_capture_productions: Vec::new(),
            source_capture_pending: Vec::new(),
            source_capture_policy_instances: Vec::new(),
            source_case_views: Vec::new(),
            disposition_inputs: Vec::new(),
            source_move_inputs: Vec::new(),
            source_backing_producers: Vec::new(),
            source_grant_creations: Vec::new(),
            common_loans: Vec::new(),
            staged_loan_queries: Vec::new(),
            source_use_graphs: Vec::new(),
            source_move_witnesses: Vec::new(),
            pending_source_moves: Vec::new(),
            owner_receiving_transitions: Vec::new(),
            pending_owner_receiving: Vec::new(),
            source_port_monitors: Vec::new(),
            source_port_equations: Vec::new(),
            source_saved_ports: Vec::new(),
            owner_consumers: Vec::new(),
            pending_owner_consumers: Vec::new(),
            source_configurations: Vec::new(),
            source_contributors: Vec::new(),
            source_instances: Vec::new(),
            source_populations: Vec::new(),
            source_views: Vec::new(),
            source_paths: Vec::new(),
            source_binding_shapes: Vec::new(),
            source_backing_observations: Vec::new(),
            source_literal_members: Vec::new(),
            productions: Vec::new(),
            block_output_aliases: Vec::new(),
            final_inputs: Vec::new(),
            binding_inputs: Vec::new(),
            raw_address_inputs: Vec::new(),
            raw_address_productions: Vec::new(),
            source_slots: Vec::new(),
            boundary_signatures: Vec::new(),
            call_argument_inputs: Vec::new(),
            owner_capabilities: Vec::new(),
            assignment_plans: Vec::new(),
            assignment_evaluations: Vec::new(),
            shapes: Vec::new(),
            values: Vec::new(),
            dispositions: Vec::new(),
            assignment_previous: Vec::new(),
            destinations: Vec::new(),
            snapshots: Vec::new(),
            commits: Vec::new(),
            results: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueShapeError {
    UnresolvedType(ScalarType),
    MissingStruct(ScalarStructId),
    MissingEnum(ScalarEnumId),
}

struct ShapeConstruction<'a> {
    producer: &'a ProducerCfgOwnerId,
    structs: &'a [ScalarStruct],
    enums: &'a [ScalarEnum],
    generic_scope: &'a ProducerCfgOwnerId,
    generic_declarations: &'a [ScalarGenericParameter],
    reservations: Vec<(ScalarType, ValueShapeId)>,
    shapes: Vec<ResolvedValueShape>,
}

impl ShapeConstruction<'_> {
    fn intern(&mut self, ty: &ScalarType) -> Result<ValueShapeId, ValueShapeError> {
        if let Some((_, id)) = self.reservations.iter().find(|(known, _)| scalar_type_equal(known, ty)) {
            return Ok(id.clone());
        }
        let index = match self.reservations.iter().map(|(_, id)| id.index).max() {
            Some(index) => index + 1,
            None => 0,
        };
        let id = ValueShapeId {
            producer: self.producer.clone(),
            index,
        };
        self.reservations.push((ty.clone(), id.clone()));
        let payload = match ty {
            ScalarType::Unit => ValuePayloadShape::Plain(PlainValueKind::Unit),
            ScalarType::Bool => ValuePayloadShape::Plain(PlainValueKind::Bool),
            ScalarType::Char => ValuePayloadShape::Plain(PlainValueKind::Char),
            ScalarType::ArtifactId => ValuePayloadShape::Plain(PlainValueKind::ArtifactId),
            ScalarType::I8 => ValuePayloadShape::Plain(PlainValueKind::I8),
            ScalarType::I16 => ValuePayloadShape::Plain(PlainValueKind::I16),
            ScalarType::I32 => ValuePayloadShape::Plain(PlainValueKind::I32),
            ScalarType::I64 => ValuePayloadShape::Plain(PlainValueKind::I64),
            ScalarType::I128 => ValuePayloadShape::Plain(PlainValueKind::I128),
            ScalarType::U8 => ValuePayloadShape::Plain(PlainValueKind::U8),
            ScalarType::U16 => ValuePayloadShape::Plain(PlainValueKind::U16),
            ScalarType::U32 => ValuePayloadShape::Plain(PlainValueKind::U32),
            ScalarType::U64 => ValuePayloadShape::Plain(PlainValueKind::U64),
            ScalarType::U128 => ValuePayloadShape::Plain(PlainValueKind::U128),
            ScalarType::F32 => ValuePayloadShape::Plain(PlainValueKind::F32),
            ScalarType::F64 => ValuePayloadShape::Plain(PlainValueKind::F64),
            ScalarType::CheckedReference { mutability, inner } => ValuePayloadShape::Checked {
                mutability: *mutability,
                referent: self.intern(inner)?,
            },
            ScalarType::RawPointer(inner) => ValuePayloadShape::Raw {
                pointee: self.intern(inner)?,
            },
            ScalarType::Callable {
                outputs,
                parameters,
            } => {
                for output in &outputs.outputs {
                    self.intern(&output.ty)?;
                }
                for parameter in parameters {
                    self.intern(parameter)?;
                }
                ValuePayloadShape::Callable {
                    signature: ty.clone(),
                }
            }
            ScalarType::Struct(structure) => {
                let declaration = self
                    .structs
                    .iter()
                    .find(|item| item.id == *structure)
                    .ok_or_else(|| ValueShapeError::MissingStruct(structure.clone()))?;
                let fields = declaration
                    .fields
                    .iter()
                    .map(|field| {
                        Ok(ValueShapeField {
                            field: field.id.clone(),
                            shape: self.intern(&field.ty)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ValueShapeError>>()?;
                ValuePayloadShape::Struct {
                    structure: structure.clone(),
                    fields,
                }
            }
            ScalarType::Enum(enumeration) => {
                let declaration = self
                    .enums
                    .iter()
                    .find(|item| item.id == *enumeration)
                    .ok_or_else(|| ValueShapeError::MissingEnum(enumeration.clone()))?;
                ValuePayloadShape::Enum {
                    enumeration: enumeration.clone(),
                    variants: declaration
                        .variants
                        .iter()
                        .map(|variant| ValueShapeVariant {
                            ordinal: variant.tag as usize,
                            fields: Vec::new(),
                        })
                        .collect(),
                }
            }
            ScalarType::Array {
                element, length, ..
            } => ValuePayloadShape::FixedArray {
                element: self.intern(element)?,
                length: *length,
            },
            ScalarType::RuntimeArray { element, .. } => ValuePayloadShape::RuntimeArray {
                element: self.intern(element)?,
            },
            ScalarType::Named { name, .. } => ValuePayloadShape::TemplateParameter {
                scope: self.generic_scope.clone(),
                declaration: template::declaration_span(self.generic_scope, self.generic_declarations, name)
                    .ok_or_else(|| ValueShapeError::UnresolvedType(ty.clone()))?,
            },
            ScalarType::Qualified { .. } | ScalarType::Error => {
                return Err(ValueShapeError::UnresolvedType(ty.clone()));
            }
        };
        self.shapes.push(ResolvedValueShape {
            id: id.clone(),
            ty: ty.clone(),
            payload,
        });
        Ok(id)
    }
}

pub fn intern_value_shape(
    shapes: &mut Vec<ResolvedValueShape>,
    producer: &ProducerCfgOwnerId,
    ty: &ScalarType,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    generic_scope: &ProducerCfgOwnerId,
    generic_declarations: &[ScalarGenericParameter],
) -> Result<ValueShapeId, ValueShapeError> {
    if !template::valid_declarations(generic_scope, generic_declarations) {
        return Err(ValueShapeError::UnresolvedType(ty.clone()));
    }
    template::validate_type_input(ty, structs, enums, generic_scope, generic_declarations, &mut Vec::new())?;
    template::validate_reused_nominals(ty, shapes, producer, structs, enums, &mut Vec::new())?;
    template::validate_shapes(shapes, producer, generic_scope, generic_declarations)?;
    let mut construction = ShapeConstruction {
        producer,
        structs,
        enums,
        generic_scope,
        generic_declarations,
        reservations: shapes
            .iter()
            .filter(|shape| shape.id.producer == *producer)
            .map(|shape| (shape.ty.clone(), shape.id.clone()))
            .collect(),
        shapes: Vec::new(),
    };
    construction.reservations.sort_by_key(|(_, id)| id.index);
    let id = construction.intern(ty)?;
    construction.shapes.sort_by_key(|shape| shape.id.index);
    shapes.extend(construction.shapes);
    Ok(id)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValuePointRemap {
    pub producer: ProducerCfgOwnerId,
    pub point: CfgPointId,
    pub imported: CfgPointId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueSchemaRemap {
    pub producers: Vec<(ProducerCfgOwnerId, ProducerCfgOwnerId)>,
    pub points: Vec<ValuePointRemap>,
    pub shapes: Vec<(ValueShapeId, ValueShapeId)>,
    pub imports: Vec<ValueImportCorrespondence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueSchemaRemapError {
    EmptyConstructionForest,
    EmptyTransportSources,
    MissingBoundarySlot(SourceSlotRef),
    AmbiguousBoundarySlot(SourceSlotRef),
    MissingBoundarySignature(ProducerCfgOwnerId),
    AmbiguousBoundarySignature(ProducerCfgOwnerId),
    MissingBoundaryFormal(BoundaryFormalPortRef),
    AmbiguousBoundaryFormal(BoundaryFormalPortRef),
    MissingOwnerCapability(OwnerCapabilityId),
    MissingOwnerEndpoint(OwnerEndpointFact),
    MissingProducer(ProducerCfgOwnerId),
    MissingPoint {
        producer: ProducerCfgOwnerId,
        point: CfgPointId,
    },
    MissingShape(ValueShapeId),
    MissingImport(ProducerCfgOwnerId),
    MissingBinding(ReferenceBindingId),
    MissingPlace(ReferencePlaceId),
    MissingOrigin {
        producer: ProducerCfgOwnerId,
        origin: ReferenceOriginId,
    },
    MissingLoan(ReferenceLoanId),
    MissingRuntime(RuntimeValueId),
    MissingEnvironment(CallableEnvironmentId),
    MissingBacking(RuntimeBackingFact),
    MissingHolder(ConnectedCaptureHolderFact),
}

impl ValueSchemaRemap {
    pub fn producer(
        &self,
        producer: &ProducerCfgOwnerId,
    ) -> Result<ProducerCfgOwnerId, ValueSchemaRemapError> {
        if self.producers.iter().filter(|(old, _)| old == producer).count() != 1 {
            return Err(ValueSchemaRemapError::MissingProducer(producer.clone()));
        }
        self.producers
            .iter()
            .find(|(original, _)| original == producer)
            .map(|(_, imported)| imported.clone())
            .ok_or_else(|| ValueSchemaRemapError::MissingProducer(producer.clone()))
    }

    pub fn point(
        &self,
        producer: &ProducerCfgOwnerId,
        point: CfgPointId,
    ) -> Result<CfgPointId, ValueSchemaRemapError> {
        if self.points.iter().filter(|row| row.producer == *producer && row.point == point).count() != 1 {
            return Err(ValueSchemaRemapError::MissingPoint { producer: producer.clone(), point });
        }
        self.points
            .iter()
            .find(|entry| entry.producer == *producer && entry.point == point)
            .map(|entry| entry.imported)
            .ok_or_else(|| ValueSchemaRemapError::MissingPoint {
                producer: producer.clone(),
                point,
            })
    }

    pub fn output(
        &self,
        output: &ResolvedValueOutputId,
    ) -> Result<ResolvedValueOutputId, ValueSchemaRemapError> {
        Ok(ResolvedValueOutputId {
            producer: self.producer(&output.producer)?,
            occurrence: output.occurrence.clone(),
            output_projection: output.output_projection.clone(),
            output: output.output,
        })
    }

    pub fn value(
        &self,
        value: &EvaluatedValueId,
    ) -> Result<EvaluatedValueId, ValueSchemaRemapError> {
        Ok(EvaluatedValueId {
            resolved: self.output(&value.resolved)?,
            point: self.point(&value.resolved.producer, value.point)?,
        })
    }

    pub fn shape(&self, shape: &ValueShapeId) -> Result<ValueShapeId, ValueSchemaRemapError> {
        if self.shapes.iter().filter(|(old, _)| old == shape).count() != 1 {
            return Err(ValueSchemaRemapError::MissingShape(shape.clone()));
        }
        self.shapes
            .iter()
            .find(|(original, _)| original == shape)
            .map(|(_, imported)| imported.clone())
            .ok_or_else(|| ValueSchemaRemapError::MissingShape(shape.clone()))
    }

    pub fn case(&self, case: &ValueCaseId) -> Result<ValueCaseId, ValueSchemaRemapError> {
        Ok(ValueCaseId {
            value: self.value(&case.value)?,
            alternative: case.alternative,
        })
    }

    pub fn capability(
        &self,
        capability: &OwnerCapabilityId,
    ) -> Result<OwnerCapabilityId, ValueSchemaRemapError> {
        Ok(OwnerCapabilityId {
            producer: self.producer(&capability.producer)?,
            introduction: self.point(&capability.producer, capability.introduction)?,
        })
    }

    pub fn path(
        &self,
        path: &EvaluatedControlPathId,
    ) -> Result<EvaluatedControlPathId, ValueSchemaRemapError> {
        Ok(EvaluatedControlPathId {
            producer: self.producer(&path.producer)?,
            ordinal: path.ordinal,
        })
    }

    pub fn operation(
        &self,
        operation: &AssignmentOperationId,
    ) -> Result<AssignmentOperationId, ValueSchemaRemapError> {
        Ok(AssignmentOperationId {
            producer: self.producer(&operation.producer)?,
            occurrence: operation.occurrence.clone(),
        })
    }

    pub fn resolved_shape(
        &self,
        shape: &ResolvedValueShape,
    ) -> Result<ResolvedValueShape, ValueSchemaRemapError> {
        let payload = match &shape.payload {
            ValuePayloadShape::TemplateParameter { scope, declaration } => ValuePayloadShape::TemplateParameter {
                scope: scope.clone(),
                declaration: declaration.clone(),
            },
            ValuePayloadShape::Plain(kind) => ValuePayloadShape::Plain(*kind),
            ValuePayloadShape::Checked {
                mutability,
                referent,
            } => ValuePayloadShape::Checked {
                mutability: *mutability,
                referent: self.shape(referent)?,
            },
            ValuePayloadShape::Raw { pointee } => ValuePayloadShape::Raw {
                pointee: self.shape(pointee)?,
            },
            ValuePayloadShape::Callable { signature } => ValuePayloadShape::Callable {
                signature: signature.clone(),
            },
            ValuePayloadShape::Struct { structure, fields } => ValuePayloadShape::Struct {
                structure: structure.clone(),
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok(ValueShapeField {
                            field: field.field.clone(),
                            shape: self.shape(&field.shape)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
            ValuePayloadShape::Enum {
                enumeration,
                variants,
            } => ValuePayloadShape::Enum {
                enumeration: enumeration.clone(),
                variants: variants
                    .iter()
                    .map(|variant| {
                        Ok(ValueShapeVariant {
                            ordinal: variant.ordinal,
                            fields: variant
                                .fields
                                .iter()
                                .map(|field| self.shape(field))
                                .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                        })
                    })
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            },
            ValuePayloadShape::FixedArray { element, length } => ValuePayloadShape::FixedArray {
                element: self.shape(element)?,
                length: *length,
            },
            ValuePayloadShape::RuntimeArray { element } => ValuePayloadShape::RuntimeArray {
                element: self.shape(element)?,
            },
        };
        Ok(ResolvedValueShape {
            id: self.shape(&shape.id)?,
            ty: shape.ty.clone(),
            payload,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluatedValueBindingError {
    WrongProducer {
        expected: ProducerCfgOwnerId,
        supplied: ResolvedValueOutputId,
    },
    MissingPoint {
        output: ResolvedValueOutputId,
        point: CfgPointId,
    },
}

pub fn bind_evaluated_value_id(
    cfg: &ScalarReferenceCfg,
    output: ResolvedValueOutputId,
    point: CfgPointId,
) -> Result<EvaluatedValueId, EvaluatedValueBindingError> {
    if cfg.producer_owner != output.producer {
        return Err(EvaluatedValueBindingError::WrongProducer {
            expected: cfg.producer_owner.clone(),
            supplied: output,
        });
    }
    if !cfg.points.iter().any(|event| event.id == point) {
        return Err(EvaluatedValueBindingError::MissingPoint { output, point });
    }
    Ok(EvaluatedValueId {
        resolved: output,
        point,
    })
}

mod remap;
#[cfg(test)]
mod tests;
pub use remap::ValueImportCorrespondence;
pub use remap::BoundaryGraphDestination;
pub use remap::{CheckedBoundaryResources, RemappedSourceUseGraphs};
