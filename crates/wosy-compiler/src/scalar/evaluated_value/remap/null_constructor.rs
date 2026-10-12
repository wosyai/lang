use super::*;
use source_boundary::null_constructor::{SourceConstructorOriginDerivation, SourceNullConstructorRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceNullConstructorAssociation {
    pub source: SourceConstructorOriginDerivation,
    pub destination: SourceConstructorOriginDerivation,
}

impl ValueSchemaRemap {
    pub(super) fn origin_derivation(&self, row: &SourceConstructorOriginDerivation) -> Result<SourceConstructorOriginDerivation, ValueSchemaRemapError> {
        if row.origin().owner != row.event().owner { return Err(ValueSchemaRemapError::MissingProducer(row.origin().owner.clone())); }
        Ok(match row {
            SourceConstructorOriginDerivation::NullExpression(row) => SourceConstructorOriginDerivation::NullExpression(SourceNullConstructorRef {
                origin: source_boundary::SourceConstructorOriginRef { owner: self.producer(&row.origin.owner)?, origin: self.origin(&row.origin.owner, row.origin.origin)? },
                event: self.source_event(&row.event)?,
            }),
            SourceConstructorOriginDerivation::CheckedParameter(row) => SourceConstructorOriginDerivation::CheckedParameter(self.checked_parameter(row)?),
            SourceConstructorOriginDerivation::AllocationIdentityAddress(row) => {
                if row.association.owner != row.origin.owner { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); }
                SourceConstructorOriginDerivation::AllocationIdentityAddress(source_boundary::allocation_identity_constructor::SourceAllocationIdentityConstructorRef {
                    origin: source_boundary::SourceConstructorOriginRef { owner: self.producer(&row.origin.owner)?, origin: self.origin(&row.origin.owner, row.origin.origin)? },
                    event: self.source_event(&row.event)?,
                    association: source_boundary::allocation_identity_constructor::SourceAllocationIdentityAssociationRef { owner: self.producer(&row.association.owner)?, association: row.association.association },
                })
            },
            SourceConstructorOriginDerivation::DirectStorageAddress(row) => SourceConstructorOriginDerivation::DirectStorageAddress(source_boundary::direct_storage_constructor::SourceDirectStorageConstructorRef {
                origin: source_boundary::SourceConstructorOriginRef { owner: self.producer(&row.origin.owner)?, origin: self.origin(&row.origin.owner, row.origin.origin)? },
                event: self.source_event(&row.event)?,
            }),
        })
    }
}

pub(super) fn counterpart(remap: &ValueSchemaRemap, source: &ValueTableSource, destination: &ValueTableSource) -> Result<(), ValueSchemaRemapError> {
    let before = source.resource_construction.original();
    let after = destination.resource_construction.original();
    let fail = || ValueSchemaRemapError::MissingProducer(source.producer.clone());
    if remap.producer(&source.producer)? != destination.producer || before.source_null_constructors.len() != after.source_null_constructors.len() { return Err(fail()); }
    for original in &before.source_null_constructors {
        let expected = remap.origin_derivation(&original.derivation())?;
        let rows = after.source_null_constructors.iter().filter(|row| row.derivation() == expected).collect::<Vec<_>>();
        let [actual] = rows.as_slice() else { return Err(fail()); };
        if original.expression != actual.expression || original.source_span.range != actual.source_span.range
            || original.scope != actual.scope || original.definition != actual.definition { return Err(fail()); }
    }
    Ok(())
}

pub(super) fn selected(remap: &ValueSchemaRemap, originals: &[BoundaryResourceConstructionEvidence], associations: &[resources::BoundaryResourceAssociation]) -> Result<Vec<SourceNullConstructorAssociation>, ValueSchemaRemapError> {
    let mut selected = Vec::new();
    for original in originals {
        for row in &original.original().source_null_constructors {
            let pairs = associations.iter().filter(|pair| pair.source().owner() == &row.origin.owner
                && pair.source().key() == &resources::BoundaryResourceKey::Origin(row.origin.origin)).collect::<Vec<_>>();
            if pairs.is_empty() { continue; }
            let [pair] = pairs.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); };
            let mapped = if remap.producers.iter().any(|pair| pair.0 == row.origin.owner) { remap.origin_derivation(&row.derivation())? } else { row.derivation() };
            let SourceConstructorOriginDerivation::NullExpression(target) = &mapped else { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); };
            if pair.destination().owner() != &target.origin.owner || pair.destination().key() != &resources::BoundaryResourceKey::Origin(target.origin.origin)
                || resources::qualified_input_owner(remap, &row.origin.owner)? != target.origin.owner {
                return Err(ValueSchemaRemapError::MissingOrigin { producer: row.origin.owner.clone(), origin: row.origin.origin });
            }
            selected.push(SourceNullConstructorAssociation { source: row.derivation(), destination: mapped });
        }
    }
    Ok(selected)
}

pub(super) fn selected_originals(remap: &ValueSchemaRemap, source: &[BoundaryResourceConstructionEvidence], destination: &[BoundaryResourceConstructionEvidence], selected: &[SourceNullConstructorAssociation]) -> Result<(), ValueSchemaRemapError> {
    for original in source {
        let owner = &original.original().producer_owner;
        let target = resources::qualified_input_owner(remap, owner)?;
        let rows = destination.iter().filter(|row| row.original().producer_owner == target).collect::<Vec<_>>();
        let [actual] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
        if !remap.producers.iter().any(|pair| pair.0 == *owner) && *original != **actual { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
        if remap.producers.iter().any(|pair| pair.0 == *owner) {
            let before = &original.original().source_null_constructors;
            let after = &actual.original().source_null_constructors;
            if before.len() != after.len() { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
            for row in before {
                let expected = remap.origin_derivation(&row.derivation())?;
                let rows = after.iter().filter(|row| row.derivation() == expected).collect::<Vec<_>>();
                let [target] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
                if row.expression != target.expression || row.source_span.range != target.source_span.range || row.scope != target.scope || row.definition != target.definition {
                    return Err(ValueSchemaRemapError::MissingProducer(owner.clone()));
                }
            }
        }
        for pair in selected {
            let SourceConstructorOriginDerivation::NullExpression(before) = &pair.source else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
            if before.origin.owner != *owner { continue; }
            let before_rows = original.original().source_null_constructors.iter().filter(|row| row.derivation() == pair.source).collect::<Vec<_>>();
            let after_rows = actual.original().source_null_constructors.iter().filter(|row| row.derivation() == pair.destination).collect::<Vec<_>>();
            let ([before], [after]) = (before_rows.as_slice(), after_rows.as_slice()) else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
            if before.expression != after.expression || before.source_span.range != after.source_span.range || before.scope != after.scope || before.definition != after.definition {
                return Err(ValueSchemaRemapError::MissingProducer(owner.clone()));
            }
        }
    }
    Ok(())
}

pub(super) fn owned(remap: &ValueSchemaRemap, source: &[ValueTableSource], resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    name_resolution::validate_owned_originals(source, resources)?;
    let required = selected(remap, resources.source_originals(), resources.associations())?;
    if required != resources.null_constructors() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    if let resources::BoundaryResourceProvenance::Independent { originals } = resources.provenance() {
        selected_originals(remap, resources.source_originals(), originals, &required)?;
    }
    checked_parameter::owned(remap, source, resources)?;
    direct_storage::owned(remap, source, resources)?;
    allocation_identity::owned(remap, source, resources)?;
    Ok(())
}

pub(super) fn full(remap: &ValueSchemaRemap, source: &[ValueTableSource], mapped: &[RemappedValueTable], resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    owned(remap, source, resources)?;
    for original in source {
        let rows = mapped.iter().filter(|row| row.source == original.producer).collect::<Vec<_>>();
        let [actual] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); };
        let expected = original.resource_construction.original().source_null_constructors.iter().map(|row| remap.origin_derivation(&row.derivation())).collect::<Result<Vec<_>, _>>()?;
        if expected != actual.tables.source_origin_derivations.iter().filter(|row| matches!(row, SourceConstructorOriginDerivation::NullExpression(_))).cloned().collect::<Vec<_>>() { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); }
    }
    Ok(())
}

pub(super) fn selective(remap: &ValueSchemaRemap, carrier: &ValueTableSource, source: &[ValueTableSource], mapped: &EvaluatedValueTables, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    owned(remap, source, resources)?;
    let expected = carrier.resource_construction.original().source_null_constructors.iter().map(|row| remap.origin_derivation(&row.derivation())).collect::<Result<Vec<_>, _>>()?;
    if expected != mapped.source_origin_derivations.iter().filter(|row| matches!(row, SourceConstructorOriginDerivation::NullExpression(_))).cloned().collect::<Vec<_>>() { return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone())); }
    Ok(())
}
