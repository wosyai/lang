use super::*;

pub(in crate::scalar::evaluated_value::remap) fn validate_independent_resources(
    remap: &ValueSchemaRemap,
    source: &[ValueTableSource],
    source_forest: &SourceCaptureConstructionForest,
    destination: &[ValueTableSource],
    destination_forest: &SourceCaptureConstructionForest,
    demand: &BoundaryResourceDemand,
    correspondence: &BoundaryArenaCorrespondence,
) -> Result<CheckedBoundaryResources, ValueSchemaRemapError> {
    let source_tuples = original_capture_tuples(source)?;
    let destination_tuples = original_capture_tuples(destination)?;
    let target_catalog = original::catalog(&destination_forest.sources)?;
    let actual_reservations = reservation::ResourceReservations::original(&target_catalog);
    let mut source_closure = original::closure(demand, source_forest)?;
    retain_header_originals(&mut source_closure.originals, source)?;
    let source_reservations = reservation::ResourceReservations::original(&source_closure);
    let proposed = source_closure.vertices.iter().map(|row| allocation::proposed(remap, &row.reference)).collect::<Result<Vec<_>, _>>()?;
    let target_demand = BoundaryResourceDemand { roots: proposed.clone() };
    let mut target_closure = original::closure(&target_demand, destination_forest)?;
    retain_header_originals(&mut target_closure.originals, destination)?;
    let input_types = input_counterpart::retain_creator_types(remap, &correspondence.input_types, &source_closure.originals, &target_closure.originals, source_forest, destination_forest)?;
    let target_reservations = actual_reservations.subset(&target_closure)?;
    let target_values = target_closure.vertices.iter().map(|row| row.value.clone()).collect::<Vec<_>>();
    let arenas = target_reservations.fill(&target_closure, &target_values)?;
    let mut associations = Vec::new();
    for ((source, reserved), destination) in source_closure.vertices.iter().zip(&source_reservations.vertices).zip(&proposed) {
        let targets = target_closure.vertices.iter().filter(|row| row.reference == *destination).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(original::missing(&source.reference)); };
        let expected = copied::CopyResourceConstruction::paired(remap, &source.reference.owner, &destination.owner, source_forest).value(&source.value)?;
        if expected != target.value { return Err(original::missing(&source.reference)); }
        associations.push(BoundaryResourceAssociation {
            source: source.reference.clone(), destination: destination.clone(), source_vertex: reserved.source_vertex.clone(),
            destination_vertex: target_reservations.destination(destination)?,
        });
    }
    allocation::validate_edges(&source_closure, &arenas, &associations)?;
    for source in &source_closure.vertices {
        let pair = allocation::association(&associations, &source.reference)?;
        let targets = target_closure.vertices.iter().filter(|row| row.reference == pair.destination).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(original::missing(&source.reference)); };
        if source.dependencies.len() != target.dependencies.len() { return Err(original::missing(&source.reference)); }
        for (original, actual) in source.dependencies.iter().zip(&target.dependencies) {
            if allocation::association(&associations, original)?.destination != *actual { return Err(original::missing(original)); }
        }
    }
    let mut capture_inputs = Vec::new();
    for source in source_tuples {
        let expected = allocation::mapped_tuple(remap, &source, correspondence, &associations, true)?;
        let targets = destination_tuples.iter().filter(|tuple| tuple.production.event == expected.production.event).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: source.production.event.owner.clone(), point: source.production.event.point }); };
        capture_counterpart::validate(remap, &source, &expected, target, correspondence, &source_closure.originals, &target_closure.originals, &input_types, &associations)?;
        capture_inputs.push(BoundaryCaptureResourceAssociation { source, destination: (*target).clone() });
    }
    let null_constructors = null_constructor::selected(remap, &source_closure.originals, &associations)?;
    null_constructor::selected_originals(remap, &source_closure.originals, &target_closure.originals, &null_constructors)?;
    let checked_parameters = checked_parameter::selected(remap, &source_closure.originals, &associations)?;
    checked_parameter::original_pairs(remap, &source_closure.originals, &target_closure.originals)?;
    let direct_storage = direct_storage::selected(remap, &source_closure.originals, &associations)?;
    direct_storage::original_pairs(remap, &source_closure.originals, &target_closure.originals, &input_types)?;
    let allocation_identity = allocation_identity::selected(remap, &source_closure.originals, &associations)?;
    allocation_identity::original_pairs(remap, &source_closure.originals, &target_closure.originals, &input_types)?;
    Ok(CheckedBoundaryResources {
        source_originals: source_closure.originals, null_constructors, checked_parameters, direct_storage, allocation_identity, arenas, associations,
        provenance: BoundaryResourceProvenance::Independent { originals: target_closure.originals }, capture_inputs, header_inputs: correspondence.header_inputs.clone(), input_types,
    })
}
