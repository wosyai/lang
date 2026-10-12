use super::*;
use super::null_constructor::SourceConstructorOriginDerivation;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceAllocationIdentityAssociationRef {
    pub owner: ProducerCfgOwnerId,
    pub association: ReferenceRebindAssociationId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceAllocationIdentityConstructorRef {
    pub origin: SourceConstructorOriginRef,
    pub event: SourceEventId,
    pub association: SourceAllocationIdentityAssociationRef,
}

pub(in crate::scalar) fn derivation(owner: &ProducerCfgOwnerId, point: CfgPointId, origin: ReferenceOriginId, association: ReferenceRebindAssociationId) -> SourceConstructorOriginDerivation {
    SourceConstructorOriginDerivation::AllocationIdentityAddress(SourceAllocationIdentityConstructorRef {
        origin: SourceConstructorOriginRef { owner: owner.clone(), origin },
        event: SourceEventId { owner: owner.clone(), point },
        association: SourceAllocationIdentityAssociationRef { owner: owner.clone(), association },
    })
}

pub(in crate::scalar) fn expected(cfg: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Result<Vec<SourceAllocationIdentityConstructorRef>, ReferenceAnalysisError> {
    reference::name_resolution::allocation_constructors(cfg, structs)
}

#[cfg(test)]
pub(in crate::scalar) mod tests;
