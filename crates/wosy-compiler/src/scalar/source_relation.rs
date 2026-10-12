use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceConfigurationRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointProducerTupleRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSupportedClassRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RefinedPopulationRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceInstanceRef {
    Supported(SourceSupportedClassRef),
    Residual(RefinedPopulationRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceCoordinateRole {
    Boundary(BoundaryPortRole),
    Binding(ReferenceBindingId),
    Evaluated(EvaluatedValueId),
    Owner(OwnerCapabilityId),
    Saved(FrozenSelectionOccurrence),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceValueViewRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceInstanceOrigin {
    Formal(BoundaryFormalPortRef),
    Creation(SourceEventId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceObjectRole {
    Payload,
    BindingCell,
    Environment,
    Allocator,
    Status,
    Owner(OwnerCapabilityId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSupportedClassFact {
    pub id: SourceSupportedClassRef,
    pub variable: SupportedInstanceVariable,
    pub frame: FrameVariableRef,
    pub support: SourceSlotRef,
    pub shape: ValueShapeId,
    pub role: SourceObjectRole,
    pub origin: SourceInstanceOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceCardinality {
    Zero,
    One,
    Many,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourcePairTruth {
    AllPairs,
    SomePairs,
    NoPairs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceTargetTruth {
    EverySource,
    SomeSource,
    NoSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceFunctionalTruth {
    EverySourceOne,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceCompatibleTruth {
    EveryCompatiblePair,
    SomeCompatiblePair,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceDependencyLabel {
    CapturedBinding,
    CheckedReferent,
    Payload,
    AllocatorInstance,
    CellContent,
    Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceObligationState {
    Outstanding,
    Transferred,
    Discharged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourcePopulationFact {
    pub id: RefinedPopulationRef,
    pub site: SourceEventId,
    pub shape: ValueShapeId,
    pub role: SourceObjectRole,
    pub owner: OwnerCapabilityId,
    pub cardinality: SourceCardinality,
    pub obligations: SourceObligationState,
    pub availability: ValueAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourcePayloadView {
    Plain,
    Unwritten,
    Formal(SourceFormalValueRef),
    Raw {
        provenance: RawValueProvenance,
        referent: SourceInstanceRef,
    },
    StaticRaw(SourceSpan),
    Aggregate(Vec<SourceMemberView>),
    Backing {
        instance: SourceInstanceRef,
        element: ValueShapeId,
    },
    Checked {
        state: CheckedValueState,
        referents: Vec<SourceCheckedReferentView>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCheckedReferentView {
    pub instance: SourceInstanceRef,
    pub coordinate: SourceSlotRef,
    pub projection: Vec<ValueProjection>,
    pub grant: SourceSpan,
    pub mode: ScalarReferenceMutability,
    pub loan: SourceCheckedGrantState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceGrantClassRef {
    pub owner: ProducerCfgOwnerId,
    pub variable: SupportedInstanceVariable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceGrantClassFact {
    pub class: SourceGrantClassRef,
    pub site: SourceEventId,
    pub referent: SourceInstanceRef,
    pub coordinate: SourceSlotRef,
    pub projection: Vec<ValueProjection>,
    pub mode: ScalarReferenceMutability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceGrantAncestryFact {
    pub ancestor: SourceGrantClassRef,
    pub descendant: SourceGrantClassRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceCheckedGrantState {
    Selected(SourceGrantClassRef),
    Pending { event: SourceEventId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceGrantParent {
    Root,
    Selected {
        class: SourceGrantClassRef,
        source: EvaluatedValueId,
        selection: JointSelectedPortFact,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceGrantCreationWitness {
    pub event: SourceEventId,
    pub input: SourceConfigurationRef,
    pub output: SourceConfigurationRef,
    pub contributor: JointProducerTupleRef,
    pub child: SourceGrantClassRef,
    pub parent: SourceGrantParent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSavedViewFact {
    pub coordinate: SourceSlotRef,
    pub snapshot: SourceEventId,
    pub selection: FrozenSelectionOccurrence,
    pub holder: ReferenceBindingId,
    pub view: SourceValueViewRef,
    pub availability: ValueAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMemberView {
    pub projection: Vec<ValueProjection>,
    pub source: EvaluatedValueId,
    pub view: SourceValueViewRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceValueViewFact {
    pub id: SourceValueViewRef,
    pub shape: ValueShapeId,
    pub payload: SourcePayloadView,
    pub dependencies: Vec<EvaluatedDependencyFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSelectedSlotFact {
    pub coordinate: SourceSlotRef,
    pub view: SourceValueViewRef,
    pub availability: ValueAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceOwnsFact {
    pub capability: OwnerCapabilityId,
    pub owner_instance: SourceInstanceRef,
    pub payload: SourceInstanceRef,
    pub endpoint: OwnerEndpointFact,
    pub kind: RuntimeBackingKind,
    pub state: SourceObligationState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceDependsFact {
    pub source: SourceInstanceRef,
    pub label: SourceDependencyLabel,
    pub target: SourceInstanceRef,
    pub truth: SourcePairTruth,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceHasTargetFact {
    pub source: SourceInstanceRef,
    pub label: SourceDependencyLabel,
    pub target: RefinedPopulationRef,
    pub truth: SourceTargetTruth,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFunctionalFact {
    pub source: SourceInstanceRef,
    pub label: SourceDependencyLabel,
    pub truth: SourceFunctionalTruth,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceTargetRelationFact {
    pub left: SourceInstanceRef,
    pub left_label: SourceDependencyLabel,
    pub right: SourceInstanceRef,
    pub right_label: SourceDependencyLabel,
    pub truth: SourceCompatibleTruth,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCreatedAtFact {
    pub instance: SourceInstanceRef,
    pub event: SourceEventId,
    pub shape: ValueShapeId,
    pub role: SourceObjectRole,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFormalInputFact {
    pub port: BoundaryFormalPortRef,
    pub coordinate: SourceSlotRef,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourcePendingProducerFact {
    pub event: SourceEventId,
    pub outputs: Vec<EvaluatedValueId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceConfigurationFact {
    pub id: SourceConfigurationRef,
    pub frame: FrameVariableRef,
    pub slots: Vec<SourceSelectedSlotFact>,
    pub instances: Vec<SourceInstanceRef>,
    pub created: Vec<SourceCreatedAtFact>,
    pub owns: Vec<SourceOwnsFact>,
    pub depends: Vec<SourceDependsFact>,
    pub has_target: Vec<SourceHasTargetFact>,
    pub functional: Vec<SourceFunctionalFact>,
    pub same_target: Vec<SourceTargetRelationFact>,
    pub different_target: Vec<SourceTargetRelationFact>,
    pub formal_inputs: Vec<SourceFormalInputFact>,
    pub pending: Vec<SourcePendingProducerFact>,
    pub saved: Vec<SourceSavedViewFact>,
    pub owner_views: Vec<SourceOwnerViewFact>,
    pub grants: Vec<SourceGrantClassFact>,
    pub grant_ancestry: Vec<SourceGrantAncestryFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceOwnerViewFact {
    pub coordinate: SourceSlotRef,
    pub projection: Vec<ValueProjection>,
    pub payload: SourceInstanceRef,
    pub capability: OwnerCapabilityId,
    pub owner_instance: SourceInstanceRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCaseViewFact {
    pub case: ValueCaseId,
    pub view: SourceValueViewRef,
    pub port: SourceSlotRef,
    pub path: EvaluatedControlPathId,
    pub configuration: SourceConfigurationRef,
    pub contributor: JointProducerTupleRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceShapeMemberPort {
    pub shape: ValueShapeId,
    pub member: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum JointValueTransportKind {
    Preserve,
    Copy,
    Move,
    Project(SourceShapeMemberPort),
    Snapshot,
    Result,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum JointValueChildPort {
    LocalInput,
    SelectedChild { event: SourceEventId, port: usize },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointValueTransportFact {
    pub source: SourceSlotRef,
    pub destination: SourceSlotRef,
    pub kind: JointValueTransportKind,
    pub child: JointValueChildPort,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointPortMonitorRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointSelectedPortFact {
    pub coordinate: SourceSlotRef,
    pub observation: SourceCaseViewFact,
    pub sources: Vec<SourceCaseViewFact>,
    pub moves: Vec<SourceMoveWitnessRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMoveInput {
    pub event: SourceEventId,
    pub source: EvaluatedValueId,
    pub place: ReferencePlaceId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceBackingProducerFact {
    pub event: SourceEventId,
    pub target: ReferenceBindingId,
    pub value: EvaluatedValueId,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMoveWitnessRef {
    pub owner: ProducerCfgOwnerId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMoveWitnessFact {
    pub id: SourceMoveWitnessRef,
    pub event: SourceEventId,
    pub input: SourceConfigurationRef,
    pub output: SourceConfigurationRef,
    pub contributor: JointProducerTupleRef,
    pub source: EvaluatedValueId,
    pub selection: JointSelectedPortFact,
    pub place: ReferencePlaceId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceMovePendingReason {
    Deferred,
    Through,
    UnsupportedPlace(ReferencePlaceId),
    SelectedSource(SourceMoveInput),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMovePendingFact {
    pub event: SourceEventId,
    pub reason: SourceMovePendingReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointPortMonitorFact {
    pub id: JointPortMonitorRef,
    pub contributor: JointProducerTupleRef,
    pub ports: Vec<JointSelectedPortFact>,
    pub pending: Vec<SourceSlotRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointSelectedChildFact {
    pub event: SourceEventId,
    pub monitor: JointPortMonitorRef,
    pub ports: Vec<SourceSlotRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointPortProductionFact {
    pub observation: SourceCaseViewFact,
    pub inputs: Vec<SourceSlotRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointValueProductionPort {
    pub destination: SourceSlotRef,
    pub inputs: Vec<SourceSlotRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointPortMonitorEquation {
    pub contributor: JointProducerTupleRef,
    pub input: JointPortMonitorRef,
    pub children: Vec<JointSelectedChildFact>,
    pub productions: Vec<JointPortProductionFact>,
    pub moves: Vec<SourceMoveWitnessRef>,
    pub output: JointPortMonitorRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSavedPortFact {
    pub source_place: ReferencePlaceId,
    pub snapshot: SourceEventId,
    pub selection: FrozenSelectionOccurrence,
    pub monitor: JointPortMonitorRef,
    pub selected: JointSelectedPortFact,
    pub duties: Vec<SourceOwnsFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerDependencySelection {
    pub case: ValueCaseId,
    pub projection: Vec<ValueProjection>,
    pub path: EvaluatedControlPathId,
    pub source_instance: SourceInstanceRef,
    pub owner_instance: SourceInstanceRef,
    pub dependency: EvaluatedDependencyFact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerReceivingTransitionFact {
    pub event: SourceEventId,
    pub input: SourceConfigurationRef,
    pub output: SourceConfigurationRef,
    pub receiving: OwnerCapabilityId,
    pub receiving_instance: SourceInstanceRef,
    pub dependencies: Vec<OwnerDependencySelection>,
    pub endpoints: Vec<OwnerEndpointFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnerReceivingPendingReason {
    DeferredPolicy,
    SelectedSource,
    DependencyAction {
        case: ValueCaseId,
        dependency: EvaluatedDependencyFact,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerReceivingPendingFact {
    pub event: SourceEventId,
    pub input: SourceConfigurationRef,
    pub source: EvaluatedValueId,
    pub reason: OwnerReceivingPendingReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedOwnerConsumer {
    pub consumer: ValueConsumerId,
    pub input: ValueCaseId,
    pub path: EvaluatedControlPathId,
    pub receiving: OwnerCapabilityId,
    pub contributor: JointProducerTupleRef,
    pub dependencies: Vec<OwnerDependencySelection>,
    pub disposition: ValueDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnerConsumerRequirement {
    SelectedRemainingUses(SourceUseObservation),
    CandidateLocalQueries(StagedLoanQueryRef),
    StagedLoanValidation(SourceEventId),
    RemainingDependencyUses(SourceConfigurationRef),
    SourceMoveWitness(EvaluatedValueId),
    CompleteSourcePredicateProduct(SourceConfigurationRef),
    FiniteSupportNormalization(SourceConfigurationRef),
    FixedReturnMaterialization(SourceInstanceRef),
    BoundarySubstitution(BoundaryFormalPortRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingOwnerConsumer {
    pub candidate: EvaluatedOwnerConsumer,
    pub requirements: Vec<OwnerConsumerRequirement>,
    pub monitor: JointPortMonitorRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum JointProducerRuleKind {
    Root,
    Step,
    Branch { predecessor: SourceEventId },
    Call { child: ProducerCfgOwnerId },
    Return,
    Backedge { predecessor: SourceEventId },
    Capture { child: ProducerCfgOwnerId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointProducerInputPort {
    pub event: SourceEventId,
    pub configuration: SourceConfigurationRef,
    pub ports: Vec<SourceSlotRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JointProducerTupleFact {
    pub id: JointProducerTupleRef,
    pub event: SourceEventId,
    pub input: SourceConfigurationRef,
    pub output: SourceConfigurationRef,
    pub kind: JointProducerRuleKind,
    pub children: Vec<JointProducerInputPort>,
    pub preserved: Vec<SourceInstanceRef>,
    pub fresh: Vec<SourceInstanceRef>,
    pub outputs: Vec<EvaluatedValueId>,
    pub transports: Vec<JointValueTransportFact>,
    pub productions: Vec<JointValueProductionPort>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedPathFact {
    pub id: EvaluatedControlPathId,
    pub event: SourceEventId,
    pub frame: FrameVariableRef,
    pub configuration: SourceConfigurationRef,
    pub contributor: JointProducerTupleRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceBindingShapeFact {
    pub binding: ReferenceBindingId,
    pub shape: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceBackingObservation {
    pub view: SourceValueViewRef,
    pub backing: RuntimeBackingFact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLiteralMembersFact {
    pub value: EvaluatedValueId,
    pub members: Vec<EvaluatedMemberFact>,
}
