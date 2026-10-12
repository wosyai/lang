use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNullConstructor {
    pub origin: SourceConstructorOriginRef,
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub scope: ReferenceScopeId,
    pub expression: ScalarExpression,
    pub definition: ReferenceOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNullConstructorRef {
    pub origin: SourceConstructorOriginRef,
    pub event: SourceEventId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceConstructorOriginDerivation {
    NullExpression(SourceNullConstructorRef),
    CheckedParameter(super::checked_parameter_constructor::SourceCheckedParameterConstructorRef),
    DirectStorageAddress(super::direct_storage_constructor::SourceDirectStorageConstructorRef),
    AllocationIdentityAddress(super::allocation_identity_constructor::SourceAllocationIdentityConstructorRef),
}

impl SourceNullConstructor {
    pub fn derivation(&self) -> SourceConstructorOriginDerivation {
        SourceConstructorOriginDerivation::NullExpression(SourceNullConstructorRef { origin: self.origin.clone(), event: self.event.clone() })
    }
}

#[cfg(test)]
mod tests;

impl SourceConstructorOriginDerivation {
    pub fn origin(&self) -> &SourceConstructorOriginRef {
        match self { Self::NullExpression(row) => &row.origin, Self::CheckedParameter(row) => &row.origin, Self::DirectStorageAddress(row) => &row.origin, Self::AllocationIdentityAddress(row) => &row.origin }
    }

    pub fn event(&self) -> &SourceEventId {
        match self { Self::NullExpression(row) => &row.event, Self::CheckedParameter(row) => &row.event, Self::DirectStorageAddress(row) => &row.event, Self::AllocationIdentityAddress(row) => &row.event }
    }
}
