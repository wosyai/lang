use super::*;

pub(super) fn fields(
    source_owner: &ProducerCfgOwnerId,
    source: &EvaluatedValueTables,
    destination_owner: &ProducerCfgOwnerId,
    destination: &EvaluatedValueTables,
    cut: &BoundaryResourceDemandCut,
    resources: &CheckedBoundaryResources,
) -> Result<(), ValueSchemaRemapError> {
    let originals = demand::table_roots(source_owner, source, cut);
    let mapped = demand::table_roots(destination_owner, destination, cut);
    if originals.len() != mapped.len() { return Err(ValueSchemaRemapError::MissingProducer(source_owner.clone())); }
    for (original, mapped) in originals.iter().zip(&mapped) {
        let association = allocation::association(&resources.associations, original)?;
        let vertices = resources.arenas.iter().flat_map(|arena| &arena.vertices).filter(|vertex| vertex.id == association.destination_vertex).collect::<Vec<_>>();
        let [vertex] = vertices.as_slice() else { return Err(original::missing(original)); };
        if association.destination != *mapped || allocation::key(&vertex.value) != mapped.key { return Err(original::missing(original)); }
    }
    Ok(())
}

pub(super) fn capture_inputs(
    remap: &ValueSchemaRemap,
    source: &[ValueTableSource],
    correspondence: &BoundaryArenaCorrespondence,
    resources: &CheckedBoundaryResources,
) -> Result<(), ValueSchemaRemapError> {
    let originals = original_capture_tuples(source)?;
    if resources.capture_inputs.len() != originals.len() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    for (original, association) in originals.iter().zip(&resources.capture_inputs) {
        if association.source != *original {
            return Err(ValueSchemaRemapError::MissingPoint { producer: original.production.event.owner.clone(), point: original.production.event.point });
        }
        let independent = matches!(&resources.provenance, BoundaryResourceProvenance::Independent { .. });
        let transported = allocation::mapped_tuple(remap, original, correspondence, &resources.associations, independent)?;
        if independent {
            capture_counterpart::validate_owned(remap, original, &transported, &association.destination, correspondence, resources)?;
        } else if association.destination != transported {
            return Err(ValueSchemaRemapError::MissingPoint { producer: original.production.event.owner.clone(), point: original.production.event.point });
        }
        for root in capture_tuple_roots(original) {
            let association = allocation::association(&resources.associations, &root)?;
            if resources.arenas.iter().flat_map(|arena| &arena.vertices).filter(|vertex| vertex.id == association.destination_vertex && allocation::key(&vertex.value) == association.destination.key).count() != 1 {
                return Err(original::missing(&root));
            }
        }
    }
    Ok(())
}

pub(in crate::scalar::evaluated_value::remap) fn full(remap: &ValueSchemaRemap, source: &[ValueTableSource], mapped: &[RemappedValueTable], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    capture_inputs(remap, source, correspondence, resources)?;
    header_counterpart::postvalidate(remap, source, correspondence, resources)?;
    header_counterpart::full_headers(remap, source, mapped, correspondence, resources)?;
    null_constructor::full(remap, source, mapped, resources)?;
    checked_parameter::full(remap, source, mapped)?;
    for original in source {
        let rows = mapped.iter().filter(|row| row.source == original.producer).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); };
        if row.resources != *resources { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); }
        fields(&original.producer, &original.tables, &row.destination, &row.tables, &BoundaryResourceDemandCut::Full, resources)?;
    }
    for tuple in &resources.capture_inputs {
        let independent = matches!(&resources.provenance, BoundaryResourceProvenance::Independent { .. });
        let transported = allocation::mapped_tuple(remap, &tuple.source, correspondence, &resources.associations, independent)?;
        let productions = mapped.iter().filter(|row| row.source == tuple.source.production.event.owner).flat_map(|row| &row.tables.source_capture_productions).filter(|production| production.event == transported.production.event).collect::<Vec<_>>();
        let [production] = productions.as_slice() else {
            return Err(ValueSchemaRemapError::MissingPoint { producer: tuple.source.production.event.owner.clone(), point: tuple.source.production.event.point });
        };
        if **production != transported.production {
            return Err(ValueSchemaRemapError::MissingPoint { producer: tuple.source.production.event.owner.clone(), point: tuple.source.production.event.point });
        }
        if independent { capture_counterpart::validate_owned(remap, &tuple.source, &transported, &tuple.destination, correspondence, resources)?; }
        else if tuple.destination != transported { return Err(ValueSchemaRemapError::MissingPoint { producer: tuple.source.production.event.owner.clone(), point: tuple.source.production.event.point }); }
    }
    Ok(())
}

pub(in crate::scalar::evaluated_value::remap) fn selective(
    remap: &ValueSchemaRemap,
    root: &ProducerCfgOwnerId,
    source: &[ValueTableSource],
    mapped: &EvaluatedValueTables,
    headers: &[BoundaryTableHeaders],
    correspondence: &BoundaryArenaCorrespondence,
    cut: &BoundaryResourceDemandCut,
    resources: &CheckedBoundaryResources,
) -> Result<(), ValueSchemaRemapError> {
    let rows = source.iter().filter(|row| row.producer == *root).collect::<Vec<_>>();
    let [carrier] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); };
    fields(root, &carrier.tables, &remap.boundary_producer(root)?, mapped, cut, resources)?;
    capture_inputs(remap, source, correspondence, resources)?;
    header_counterpart::postvalidate(remap, source, correspondence, resources)?;
    header_counterpart::selective_headers(headers, resources)?;
    null_constructor::selective(remap, carrier, source, mapped, resources)?;
    checked_parameter::metadata(remap, carrier, mapped)?;
    for tuple in &resources.capture_inputs {
        for input in &tuple.destination.production.inputs {
            let signatures = headers.iter().flat_map(|row| &row.tables.boundary_signatures).filter(|signature| signature.owner == input.formal.owner).collect::<Vec<_>>();
            let [signature] = signatures.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone())); };
            let Some(port) = signature.ports.get(input.formal.port) else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone())); };
            if !matches!(&port.role, BoundaryPortRole::Capture { binding, .. } if binding == &input.capture.binding)
                || headers.iter().flat_map(|row| &row.tables.source_slots).filter(|slot| slot.id == input.coordinate && slot.role == SourceCoordinateRole::Binding(input.capture.binding.clone())).count() != 1 {
                return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone()));
            }
        }
    }
    Ok(())
}
