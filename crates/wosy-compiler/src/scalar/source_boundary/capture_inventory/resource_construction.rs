use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryResourceConstructionEvidence {
    original: ScalarReferenceCfg,
    structs: Vec<ScalarStruct>,
}

impl BoundaryResourceConstructionEvidence {
    pub(super) fn extract(original: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Self {
        Self { original: original.clone(), structs: structs.to_vec() }
    }

    pub(in crate::scalar) fn structs(&self) -> &[ScalarStruct] {
        &self.structs
    }

    pub(in crate::scalar) fn original(&self) -> &ScalarReferenceCfg {
        &self.original
    }
}
