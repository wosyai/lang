use super::*;
use source_boundary::checked_parameter_constructor::{self as original, SourceCheckedParameterConstructorRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCheckedParameterAssociation {
    pub source: SourceCheckedParameterConstructorRef,
    pub destination: SourceCheckedParameterConstructorRef,
}

impl ValueSchemaRemap {
    pub(super) fn checked_parameter(&self, row: &SourceCheckedParameterConstructorRef) -> Result<SourceCheckedParameterConstructorRef, ValueSchemaRemapError> {
        if row.origin.owner != row.event.owner || row.origin.owner != row.declaration.owner || !matches!(row.declaration.site, SourceNameDeclarationSite::Parameter { .. }) {
            return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone()));
        }
        Ok(SourceCheckedParameterConstructorRef {
            origin: source_boundary::SourceConstructorOriginRef { owner: self.producer(&row.origin.owner)?, origin: self.origin(&row.origin.owner, row.origin.origin)? },
            event: self.source_event(&row.event)?, declaration: self.name_declaration(&row.declaration)?, binding: self.binding(&row.origin.owner, &row.binding)?,
        })
    }
}

fn expected(cfg: &ScalarReferenceCfg) -> Result<Vec<SourceCheckedParameterConstructorRef>, ValueSchemaRemapError> {
    original::expected(cfg).map_err(|_| ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone()))
}

pub(super) fn counterpart(remap: &ValueSchemaRemap, source: &ScalarReferenceCfg, destination: &ScalarReferenceCfg) -> Result<(), ValueSchemaRemapError> {
    let before = expected(source)?;
    let after = expected(destination)?;
    let mapped = before.iter().map(|row| remap.checked_parameter(row)).collect::<Result<Vec<_>, _>>()?;
    if remap.producer(&source.producer_owner)? != destination.producer_owner || mapped != after { return Err(ValueSchemaRemapError::MissingProducer(source.producer_owner.clone())); }
    for (before, after) in before.iter().zip(&after) {
        let (ReferenceOrigin::BorrowedFrom { loan: a, .. }, ReferenceOrigin::BorrowedFrom { loan: b, .. }) = (&source.facts[before.origin.origin.0], &destination.facts[after.origin.origin.0]) else { return Err(ValueSchemaRemapError::MissingProducer(source.producer_owner.clone())); };
        if a.mode != b.mode || a.origin_place.place != b.origin_place.place || a.creation_span != b.creation_span
            || a.origin_place.function_span != b.origin_place.function_span || a.origin_place.block_span != b.origin_place.block_span {
            return Err(ValueSchemaRemapError::MissingProducer(source.producer_owner.clone()));
        }
    }
    Ok(())
}

pub(super) fn selected(remap: &ValueSchemaRemap, originals: &[BoundaryResourceConstructionEvidence], associations: &[resources::BoundaryResourceAssociation]) -> Result<Vec<SourceCheckedParameterAssociation>, ValueSchemaRemapError> {
    let mut result = Vec::new();
    for original in originals {
        for row in expected(original.original())? {
            let pairs = associations.iter().filter(|pair| pair.source().owner() == &row.origin.owner && pair.source().key() == &resources::BoundaryResourceKey::Origin(row.origin.origin)).collect::<Vec<_>>();
            if pairs.is_empty() { continue; }
            let [pair] = pairs.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); };
            let target = if remap.producers.iter().any(|pair| pair.0 == row.origin.owner) { remap.checked_parameter(&row)? } else { row.clone() };
            if pair.destination().owner() != &target.origin.owner || pair.destination().key() != &resources::BoundaryResourceKey::Origin(target.origin.origin)
                || resources::qualified_input_owner(remap, &row.origin.owner)? != target.origin.owner { return Err(ValueSchemaRemapError::MissingProducer(row.origin.owner.clone())); }
            result.push(SourceCheckedParameterAssociation { source: row, destination: target });
        }
    }
    Ok(result)
}

pub(super) fn original_pairs(remap: &ValueSchemaRemap, source: &[BoundaryResourceConstructionEvidence], destination: &[BoundaryResourceConstructionEvidence]) -> Result<(), ValueSchemaRemapError> {
    for original in source {
        let owner = &original.original().producer_owner;
        let target = resources::qualified_input_owner(remap, owner)?;
        let rows = destination.iter().filter(|row| row.original().producer_owner == target).collect::<Vec<_>>();
        let [actual] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
        if remap.producers.iter().any(|pair| pair.0 == *owner) { counterpart(remap, original.original(), actual.original())?; }
        else if *original != **actual { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
    }
    Ok(())
}

pub(super) fn owned(remap: &ValueSchemaRemap, source: &[ValueTableSource], resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    name_resolution::validate_owned_originals(source, resources)?;
    if selected(remap, resources.source_originals(), resources.associations())? != resources.checked_parameters() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    if let resources::BoundaryResourceProvenance::Independent { originals } = resources.provenance() { original_pairs(remap, resources.source_originals(), originals)?; }
    Ok(())
}

pub(super) fn metadata(remap: &ValueSchemaRemap, carrier: &ValueTableSource, mapped: &EvaluatedValueTables) -> Result<(), ValueSchemaRemapError> {
    let cfg = carrier.resource_construction.original();
    let required = original::required(cfg, &cfg.source_null_constructors, carrier.resource_construction.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(carrier.producer.clone()))?;
    let expected = required.iter().map(|row| remap.origin_derivation(row)).collect::<Result<Vec<_>, _>>()?;
    if expected != mapped.source_origin_derivations { return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone())); }
    Ok(())
}

pub(super) fn full(remap: &ValueSchemaRemap, source: &[ValueTableSource], mapped: &[RemappedValueTable]) -> Result<(), ValueSchemaRemapError> {
    for carrier in source {
        let rows = mapped.iter().filter(|row| row.source == carrier.producer).collect::<Vec<_>>();
        let [actual] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone())); };
        metadata(remap, carrier, &actual.tables)?;
    }
    Ok(())
}
