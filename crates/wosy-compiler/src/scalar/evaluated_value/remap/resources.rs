use super::*;
mod children;
mod creators;
pub(super) mod callee_constructor;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod return_transfer_tests;
#[cfg(test)]
mod c3_populated_history_tests;
mod capture_counterpart;
#[cfg(test)]
mod c3_counterpart_tests;
#[cfg(test)]
mod c3_key_builders;
#[cfg(test)]
#[path = "input_counterpart/shared_tests.rs"]
mod i1_shared_tests;
#[cfg(test)]
#[path = "input_counterpart/i2_tests.rs"]
mod i2_input_tests;
mod original;
mod reservation;
mod copied;
pub(super) mod allocation;
pub(super) mod demand;
pub(super) mod independent;
pub(super) mod postvalidation;

pub(super) fn qualified_input_owner(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId) -> Result<ProducerCfgOwnerId, ValueSchemaRemapError> {
    allocation::qualified_owner(remap, owner)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryResourceKey {
    Origin(ReferenceOriginId),
    Binding(ReferenceBindingId),
    Place(ReferencePlaceId),
    Loan(ReferenceLoanId),
    Runtime(RuntimeValueId),
    Environment(CallableEnvironmentId),
    Backing(RuntimeBackingFact),
    Holder(ConnectedCaptureHolderFact),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceRef {
    owner: ProducerCfgOwnerId,
    key: BoundaryResourceKey,
}

impl BoundaryResourceRef {
    pub fn owner(&self) -> &ProducerCfgOwnerId { &self.owner }
    pub fn key(&self) -> &BoundaryResourceKey { &self.key }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceVertexId {
    owner: ProducerCfgOwnerId,
    ordinal: usize,
}

impl BoundaryResourceVertexId {
    pub fn owner(&self) -> &ProducerCfgOwnerId { &self.owner }
    pub fn ordinal(&self) -> usize { self.ordinal }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryResourceValue {
    Origin { id: ReferenceOriginId, fact: ReferenceOrigin },
    Binding(ReferenceBindingId),
    Place(ReferencePlaceId),
    Loan(ReferenceLoanId),
    Runtime(RuntimeValueId),
    Environment { id: CallableEnvironmentId, fact: CallableEnvironmentFact },
    Backing(RuntimeBackingFact),
    Holder(ConnectedCaptureHolderFact),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceVertex {
    id: BoundaryResourceVertexId,
    value: BoundaryResourceValue,
    dependencies: Vec<BoundaryResourceVertexId>,
}

impl BoundaryResourceVertex {
    pub fn id(&self) -> &BoundaryResourceVertexId { &self.id }
    pub fn value(&self) -> &BoundaryResourceValue { &self.value }
    pub fn dependencies(&self) -> &[BoundaryResourceVertexId] { &self.dependencies }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceArena {
    owner: ProducerCfgOwnerId,
    vertices: Vec<BoundaryResourceVertex>,
}

impl BoundaryResourceArena {
    pub fn owner(&self) -> &ProducerCfgOwnerId { &self.owner }
    pub fn vertices(&self) -> &[BoundaryResourceVertex] { &self.vertices }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceAssociation {
    source: BoundaryResourceRef,
    destination: BoundaryResourceRef,
    source_vertex: BoundaryResourceVertexId,
    destination_vertex: BoundaryResourceVertexId,
}

impl BoundaryResourceAssociation {
    pub fn source(&self) -> &BoundaryResourceRef { &self.source }
    pub fn destination(&self) -> &BoundaryResourceRef { &self.destination }
    pub fn source_vertex(&self) -> &BoundaryResourceVertexId { &self.source_vertex }
    pub fn destination_vertex(&self) -> &BoundaryResourceVertexId { &self.destination_vertex }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceAllocation {
    source: BoundaryResourceRef,
    destination: BoundaryResourceRef,
    source_vertex: BoundaryResourceVertexId,
    destination_vertex: BoundaryResourceVertexId,
}

impl BoundaryResourceAllocation {
    pub fn source(&self) -> &BoundaryResourceRef { &self.source }
    pub fn destination(&self) -> &BoundaryResourceRef { &self.destination }
    pub fn source_vertex(&self) -> &BoundaryResourceVertexId { &self.source_vertex }
    pub fn destination_vertex(&self) -> &BoundaryResourceVertexId { &self.destination_vertex }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryResourceProvenance {
    Copied { allocations: Vec<BoundaryResourceAllocation> },
    Independent { originals: Vec<BoundaryResourceConstructionEvidence> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryCaptureResourceTuple {
    production: SourceCaptureProduction,
    paths: Vec<CaptureInputPathFact>,
}

impl BoundaryCaptureResourceTuple {
    pub fn production(&self) -> &SourceCaptureProduction { &self.production }
    pub fn paths(&self) -> &[CaptureInputPathFact] { &self.paths }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryCaptureResourceAssociation {
    source: BoundaryCaptureResourceTuple,
    destination: BoundaryCaptureResourceTuple,
}

impl BoundaryCaptureResourceAssociation {
    pub fn source(&self) -> &BoundaryCaptureResourceTuple { &self.source }
    pub fn destination(&self) -> &BoundaryCaptureResourceTuple { &self.destination }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckedBoundaryResources {
    source_originals: Vec<BoundaryResourceConstructionEvidence>,
    null_constructors: Vec<null_constructor::SourceNullConstructorAssociation>,
    checked_parameters: Vec<checked_parameter::SourceCheckedParameterAssociation>,
    direct_storage: Vec<direct_storage::SourceDirectStorageAssociation>,
    allocation_identity: Vec<allocation_identity::SourceAllocationIdentityAssociation>,
    arenas: Vec<BoundaryResourceArena>,
    associations: Vec<BoundaryResourceAssociation>,
    provenance: BoundaryResourceProvenance,
    capture_inputs: Vec<BoundaryCaptureResourceAssociation>,
    header_inputs: Vec<header_counterpart::BoundaryHeaderRoleAssociation>,
    input_types: input_counterpart::BoundaryInputTypeEvidence,
}

impl CheckedBoundaryResources {
    pub fn source_originals(&self) -> &[BoundaryResourceConstructionEvidence] { &self.source_originals }
    pub fn null_constructors(&self) -> &[null_constructor::SourceNullConstructorAssociation] { &self.null_constructors }
    pub fn checked_parameters(&self) -> &[checked_parameter::SourceCheckedParameterAssociation] { &self.checked_parameters }
    pub fn direct_storage(&self) -> &[direct_storage::SourceDirectStorageAssociation] { &self.direct_storage }
    pub fn allocation_identity(&self) -> &[allocation_identity::SourceAllocationIdentityAssociation] { &self.allocation_identity }
    pub fn arenas(&self) -> &[BoundaryResourceArena] { &self.arenas }
    pub fn associations(&self) -> &[BoundaryResourceAssociation] { &self.associations }
    pub fn provenance(&self) -> &BoundaryResourceProvenance { &self.provenance }
    pub fn capture_inputs(&self) -> &[BoundaryCaptureResourceAssociation] { &self.capture_inputs }
    pub(super) fn header_inputs(&self) -> &[header_counterpart::BoundaryHeaderRoleAssociation] { &self.header_inputs }
    pub(super) fn input_types(&self) -> &input_counterpart::BoundaryInputTypeEvidence { &self.input_types }
}

fn retain_header_originals(originals: &mut Vec<BoundaryResourceConstructionEvidence>, source: &[ValueTableSource]) -> Result<(), ValueSchemaRemapError> {
    for carrier in source {
        let owner = &carrier.producer;
        let rows = originals.iter().filter(|row| row.original().producer_owner == *owner).collect::<Vec<_>>();
        match rows.as_slice() {
            [] => originals.push(carrier.resource_construction.clone()),
            [original] if **original == carrier.resource_construction => {},
            _ => return Err(ValueSchemaRemapError::MissingProducer(owner.clone())),
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemappedSourceUseGraphs {
    pub graphs: Vec<SourceUseGraph>,
    pub resources: CheckedBoundaryResources,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryResourceDemand {
    roots: Vec<BoundaryResourceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BoundaryResourceDemandCut {
    Full,
    SourceUses { root: ProducerCfgOwnerId },
    StagedLoans { root: ProducerCfgOwnerId },
}

fn capture_tuple_roots(tuple: &BoundaryCaptureResourceTuple) -> Vec<BoundaryResourceRef> {
    let mut roots = Vec::new();
    for input in &tuple.production.inputs {
        roots.push(BoundaryResourceRef { owner: tuple.production.event.owner.clone(), key: BoundaryResourceKey::Binding(input.capture.binding.clone()) });
    }
    for path in &tuple.paths {
        for input in &path.inputs {
            roots.push(BoundaryResourceRef { owner: path.owner.clone(), key: BoundaryResourceKey::Binding(input.binding.clone()) });
            match &input.origin {
                CaptureInputOriginFact::Value => roots.push(BoundaryResourceRef {
                    owner: tuple.production.event.owner.clone(),
                    key: BoundaryResourceKey::Binding(input.binding.clone()),
                }),
                CaptureInputOriginFact::Callable { holders } => {
                    for holder in holders {
                        roots.push(BoundaryResourceRef {
                            owner: tuple.production.event.owner.clone(),
                            key: BoundaryResourceKey::Holder(holder.clone()),
                        });
                    }
                }
            }
        }
    }
    roots
}

fn original_capture_tuples(source: &[ValueTableSource]) -> Result<Vec<BoundaryCaptureResourceTuple>, ValueSchemaRemapError> {
    ValueSchemaRemap::validate_capture_sources(source)?;
    let mut tuples = Vec::new();
    for carrier in source {
        let cfg = carrier.resource_construction.original();
        if cfg.producer_owner != carrier.producer {
            return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone()));
        }
        for production in &cfg.evaluated_values.source_capture_productions {
            let invalid = || ValueSchemaRemapError::MissingPoint {
                producer: production.event.owner.clone(), point: production.event.point,
            };
            if cfg.evaluated_values.source_capture_productions.iter().filter(|row| row.event == production.event).count() != 1
                || carrier.tables.source_capture_productions.iter().filter(|row| *row == production).count() != 1
                || production.event.owner != cfg.producer_owner {
                return Err(invalid());
            }
            let points = cfg.initial_points.iter().filter(|point| point.id == production.event.point).collect::<Vec<_>>();
            let [point] = points.as_slice() else { return Err(invalid()); };
            let CfgPointKind::Capture { captures, signature, .. } = &point.kind else { return Err(invalid()); };
            if signature != &production.signature || captures.len() != production.inputs.len()
                || !captures.iter().zip(&production.inputs).all(|(declared, input)| declared == &input.capture) {
                return Err(invalid());
            }
            let mut paths = Vec::new();
            for path in cfg.capture_input_path_facts.iter().filter(|path| path.owner == production.event.owner && path.capture == production.event.point) {
                if path.capture_span != point.source_span || path.inputs.len() != production.inputs.len()
                    || !path.inputs.iter().zip(&production.inputs).all(|(original, port)| original.binding == port.capture.binding) {
                    return Err(invalid());
                }
                if crate::scalar::reference_producers::validate_capture_input_path_fact(cfg, path).is_err() {
                    let holder = path.inputs.iter().find_map(|input| match &input.origin {
                        CaptureInputOriginFact::Value => None,
                        CaptureInputOriginFact::Callable { holders } => holders.first(),
                    });
                    return Err(match holder { Some(holder) => ValueSchemaRemapError::MissingHolder(holder.clone()), None => invalid() });
                }
                paths.push(path.clone());
            }
            tuples.push(BoundaryCaptureResourceTuple { production: production.clone(), paths });
        }
    }
    Ok(tuples)
}

#[cfg(test)]
mod null_constructor_tests;

#[cfg(test)]
mod checked_parameter_tests;

#[cfg(test)]
mod direct_storage_tests;

#[cfg(test)]
mod allocation_identity_tests;
