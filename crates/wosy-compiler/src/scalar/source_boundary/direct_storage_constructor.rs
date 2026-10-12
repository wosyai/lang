use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceDirectStorageConstructorRef {
    pub origin: SourceConstructorOriginRef,
    pub event: SourceEventId,
}

pub(in crate::scalar) fn derivation(owner: &ProducerCfgOwnerId, point: CfgPointId, origin: ReferenceOriginId) -> null_constructor::SourceConstructorOriginDerivation {
    null_constructor::SourceConstructorOriginDerivation::DirectStorageAddress(SourceDirectStorageConstructorRef {
        origin: SourceConstructorOriginRef { owner: owner.clone(), origin }, event: SourceEventId { owner: owner.clone(), point },
    })
}

pub(in crate::scalar) fn expected(cfg: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Result<Vec<SourceDirectStorageConstructorRef>, ReferenceAnalysisError> {
    reference::name_resolution::direct_constructors(cfg, structs)
}

#[cfg(test)]
pub(in crate::scalar) mod tests;
