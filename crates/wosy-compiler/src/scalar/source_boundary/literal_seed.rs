use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLiteralConstructorSeedRef {
    pub event: SourceEventId,
    pub child: ProducerCfgOwnerId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceLiteralSeedParentSource {
    Module(module_source_topology::SourceModuleConstructorStageRef),
    Callable(ProducerCfgOwnerId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLiteralSeedBorrowCreation {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub scope: ReferenceScopeId,
    pub request: ReferenceBorrowRequest,
    pub input: ReferenceBorrowInput,
    pub definitions: Vec<(ReferenceOriginId, ReferenceOrigin)>,
    pub realized: Vec<ReferenceBorrowCase>,
    pub applied: ReferenceBorrowEvaluation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLiteralConstructorSeed {
    pub id: SourceLiteralConstructorSeedRef,
    pub parent_source: SourceLiteralSeedParentSource,
    pub environment: CallableEnvironmentId,
    pub captures: Vec<closure::ClosureCapture>,
    pub inputs: Vec<CallableCaptureInput>,
    pub initial_facts: Vec<ReferenceOrigin>,
    pub inherited_facts: Vec<ReferenceOrigin>,
    pub parent_seed_origins: Vec<SourceParentSeedOrigin>,
    pub borrow_creations: Vec<SourceLiteralSeedBorrowCreation>,
}
