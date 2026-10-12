use super::*;
use source_boundary::allocation_identity_constructor::{self as original, SourceAllocationIdentityConstructorRef};
use source_boundary::null_constructor::SourceConstructorOriginDerivation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceAllocationIdentityAssociation {
    pub source: SourceAllocationIdentityConstructorRef,
    pub destination: SourceAllocationIdentityConstructorRef,
}

fn expected(evidence: &BoundaryResourceConstructionEvidence) -> Result<Vec<SourceAllocationIdentityConstructorRef>, ValueSchemaRemapError> {
    let cfg = evidence.original();
    reference::name_resolution::validate_null_constructors(cfg, evidence.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone()))?;
    original::expected(cfg, evidence.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone()))
}

fn mapped(remap: &ValueSchemaRemap, row: &SourceAllocationIdentityConstructorRef) -> Result<SourceAllocationIdentityConstructorRef, ValueSchemaRemapError> {
    let SourceConstructorOriginDerivation::AllocationIdentityAddress(mapped) = remap.origin_derivation(&SourceConstructorOriginDerivation::AllocationIdentityAddress(row.clone()))? else { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); };
    Ok(mapped)
}

pub(super) fn selected(remap: &ValueSchemaRemap, originals: &[BoundaryResourceConstructionEvidence], associations: &[resources::BoundaryResourceAssociation]) -> Result<Vec<SourceAllocationIdentityAssociation>, ValueSchemaRemapError> {
    let mut result = Vec::new();
    for evidence in originals {
        for row in expected(evidence)? {
            let pairs = associations.iter().filter(|pair| pair.source().owner() == &row.origin.owner && pair.source().key() == &resources::BoundaryResourceKey::Origin(row.origin.origin)).collect::<Vec<_>>();
            if pairs.is_empty() { continue; }
            let [pair] = pairs.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); };
            let target = if remap.producers.iter().any(|pair| pair.0 == row.origin.owner) { mapped(remap, &row)? } else { row.clone() };
            if pair.destination().owner() != &target.origin.owner || pair.destination().key() != &resources::BoundaryResourceKey::Origin(target.origin.origin)
                || resources::qualified_input_owner(remap, &row.origin.owner)? != target.origin.owner { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); }
            result.push(SourceAllocationIdentityAssociation { source: row, destination: target });
        }
    }
    Ok(result)
}

pub(super) fn original_pairs(remap: &ValueSchemaRemap, source: &[BoundaryResourceConstructionEvidence], destination: &[BoundaryResourceConstructionEvidence], types: &input_counterpart::BoundaryInputTypeEvidence) -> Result<(), ValueSchemaRemapError> {
    for before in source {
        let owner = &before.original().producer_owner;
        let target = resources::qualified_input_owner(remap, owner)?;
        let rows = destination.iter().filter(|row| row.original().producer_owner == target).collect::<Vec<_>>();
        let [after] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
        let original = expected(before)?;
        let actual = expected(after)?;
        if remap.producers.iter().any(|pair| pair.0 == *owner) {
            if original.iter().map(|row| mapped(remap, row)).collect::<Result<Vec<_>, _>>()? != actual { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
            for (x, y) in original.iter().zip(&actual) {
                let a = &before.original().rebind_associations[x.association.association.0];
                let b = &after.original().rebind_associations[y.association.association.0];
                if a.owner != *owner || b.owner != target || a.occurrence.source_span.range != b.occurrence.source_span.range
                    || a.occurrence.output_projection != b.occurrence.output_projection { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
                for (a, b) in [(a.address_point, b.address_point), (a.raw_point, b.raw_point), (a.length_point, b.length_point), (a.effect_point, b.effect_point), (a.call_point, b.call_point)] {
                    if remap.source_event(&SourceEventId { owner: owner.clone(), point: a })? != (SourceEventId { owner: target.clone(), point: b }) { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: a }); }
                }
            }
            input_counterpart::allocation_identity_originals(remap, types, before, after)?;
        } else if before != *after { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
    }
    Ok(())
}

pub(super) fn owned(remap: &ValueSchemaRemap, source: &[ValueTableSource], resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    name_resolution::validate_owned_originals(source, resources)?;
    if selected(remap, resources.source_originals(), resources.associations())? != resources.allocation_identity() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    if let resources::BoundaryResourceProvenance::Independent { originals } = resources.provenance() {
        original_pairs(remap, resources.source_originals(), originals, resources.input_types())?;
    }
    Ok(())
}
