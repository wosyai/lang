use super::*;
use crate::scalar::reference::source_epoch;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseGraphRef {
    pub owner: ProducerCfgOwnerId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseNodeRef {
    pub contributor: JointProducerTupleRef,
    pub monitor: JointPortMonitorRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceUseSubject {
    Payload {
        instance: SourceInstanceRef,
        projection: Vec<ValueProjection>,
    },
    Grant {
        class: SourceGrantClassRef,
        referent: SourceInstanceRef,
    },
    Owner {
        capability: OwnerCapabilityId,
        instance: SourceInstanceRef,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceUseInput {
    Selected(JointSelectedPortFact),
    Storage(SourceSelectedSlotFact),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceUsePurpose {
    Read,
    Borrow,
    Move,
    Write,
    Return,
    Escape,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseSupport {
    pub node: SourceUseNodeRef,
    pub coordinate: SourceSlotRef,
    pub projection: Vec<ValueProjection>,
    pub subject: SourceUseSubject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseNode {
    pub id: SourceUseNodeRef,
    pub event: SourceEventId,
    pub configuration: SourceConfigurationRef,
    pub frame: FrameVariableRef,
    pub supports: Vec<SourceUseSupport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SelectedSourceUse {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub contributor: JointProducerTupleRef,
    pub equation: JointPortMonitorEquation,
    pub configuration: SourceConfigurationRef,
    pub support: SourceUseSupport,
    pub input: SourceUseInput,
    pub purpose: SourceUsePurpose,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceUseCoverageReason {
    Operation(source_epoch::SourceEventKind),
    MissingContributor,
    PendingProducer {
        configuration: SourceConfigurationRef,
        producer: SourcePendingProducerFact,
    },
    MissingEquation(JointProducerTupleRef),
    MissingInput(SourceCoordinateRole),
    MissingSelection(SourceSlotRef),
    Unavailable {
        coordinate: SourceSlotRef,
        state: ValueAvailability,
    },
    CheckedState {
        coordinate: SourceSlotRef,
        state: CheckedValueState,
    },
    PendingView(SourceValueViewRef),
    Projection(ReferencePlaceId),
    AssignmentInput,
    Transport {
        source: SourceSlotRef,
        destination: SourceSlotRef,
    },
    ForeignChild(SourceEventId),
    SourceRecurrence(SourceValueViewRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseCoverage {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub reason: SourceUseCoverageReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceUseEdgeWitness {
    Transport(JointValueTransportFact),
    Production(JointPortProductionFact),
    Parent(SourceGrantCreationWitness),
    Receiving(OwnerReceivingTransitionFact),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseEdge {
    pub equation: JointPortMonitorEquation,
    pub source: SourceUseSupport,
    pub destination: SourceUseSupport,
    pub witness: SourceUseEdgeWitness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUsePredecessor {
    pub edge: usize,
    pub reachable: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceReachableUse {
    pub support: SourceUseSupport,
    pub direct_use: usize,
    pub predecessors: Vec<SourceUsePredecessor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseObservation {
    pub graph: SourceUseGraphRef,
    pub node: SourceUseNodeRef,
    pub reachable: Vec<usize>,
    pub parent_support: Vec<usize>,
    pub coverage: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceReturnUseObservation {
    pub consumer: ValueConsumerId,
    pub input: SourceUseObservation,
    pub output: SourceUseNodeRef,
    pub transition: OwnerReceivingTransitionFact,
    pub subjects: Vec<SourceUseSubject>,
    pub remaining: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUseGraph {
    pub id: SourceUseGraphRef,
    pub epoch: CommonLoanInput,
    pub nodes: Vec<SourceUseNode>,
    pub uses: Vec<SelectedSourceUse>,
    pub edges: Vec<SourceUseEdge>,
    pub reachable: Vec<SourceReachableUse>,
    pub coverage: Vec<SourceUseCoverage>,
    pub returns: Vec<SourceReturnUseObservation>,
}
