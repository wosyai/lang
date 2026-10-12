use super::*;
pub(in crate::scalar) mod original_control;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCheckedPointerCastConstructorRef {
    pub origin: SourceConstructorOriginRef,
    pub event: SourceEventId,
    pub argument: SourceEventId,
}

pub(in crate::scalar) fn expected(cfg: &ScalarReferenceCfg, original: &module_resolver_input::SourceModuleResolverInputEvidence, forest: &source_boundary::SourceCaptureConstructionForest) -> Result<Vec<SourceCheckedPointerCastConstructorRef>, ReferenceAnalysisError> {
    reference::name_resolution::checked_pointer_cast_constructors(cfg, original, forest)
}
