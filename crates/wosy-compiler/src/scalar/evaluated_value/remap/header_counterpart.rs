use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryHeaderRoleAssociation {
    source: SourceSlotFact,
    transported: SourceCoordinateRole,
    destination: SourceSlotFact,
}

impl BoundaryHeaderRoleAssociation {
    pub(super) fn new(source: SourceSlotFact, transported: SourceCoordinateRole, destination: SourceSlotFact) -> Self { Self { source, transported, destination } }
    pub(super) fn source(&self) -> &SourceSlotFact { &self.source }
    pub(super) fn transported(&self) -> &SourceCoordinateRole { &self.transported }
    pub(super) fn destination(&self) -> &SourceSlotFact { &self.destination }
}

pub(super) enum BoundaryHeaderRoleInput<'a> {
    Transported,
    Independent { source: &'a [ValueTableSource], source_forest: &'a SourceCaptureConstructionForest, destination: &'a [ValueTableSource], destination_forest: &'a SourceCaptureConstructionForest },
}

fn lexical(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => lexical(enclosing),
    }
}

fn original_slot<'a>(carriers: &'a [ValueTableSource], slot: &SourceSlotFact) -> Result<&'a ScalarReferenceCfg, ValueSchemaRemapError> {
    let rows = carriers.iter().filter(|row| row.producer == slot.id.owner).collect::<Vec<_>>();
    let [carrier] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(slot.id.owner.clone())); };
    let cfg = carrier.resource_construction.original();
    if cfg.producer_owner != slot.id.owner || cfg.evaluated_values.source_slots.iter().filter(|row| *row == slot).count() != 1 {
        return Err(ValueSchemaRemapError::MissingBoundarySlot(slot.id.clone()));
    }
    Ok(cfg)
}

fn value_constructor(cfg: &ScalarReferenceCfg, id: &EvaluatedValueId, shape: &ValueShapeId) -> bool {
    cfg.producer_owner == id.resolved.producer && shape.producer == cfg.producer_owner
        && id.resolved.occurrence.source == *lexical(&cfg.producer_owner)
        && cfg.evaluated_values.values.iter().filter(|row| row.id == *id && row.shape == *shape).count() == 1
        && cfg.initial_points.iter().filter(|point| point.id == id.point).count() == 1
        && cfg.evaluated_values.shapes.iter().filter(|row| row.id == *shape).count() == 1
}

fn value_pair(remap: &ValueSchemaRemap, source: &EvaluatedValueId, transported: &EvaluatedValueId, actual: &EvaluatedValueId) -> Result<bool, ValueSchemaRemapError> {
    Ok(*transported == remap.value(source)?
        && transported.resolved.occurrence == source.resolved.occurrence
        && source.resolved.occurrence.source == *lexical(&source.resolved.producer)
        && actual.resolved.occurrence.source == *lexical(&actual.resolved.producer)
        && source.resolved.occurrence.range == actual.resolved.occurrence.range
        && transported.resolved.producer == actual.resolved.producer && transported.point == actual.point
        && transported.resolved.output == actual.resolved.output && transported.resolved.output_projection == actual.resolved.output_projection
        && remap.boundary_point(&SourceEventId { owner: source.resolved.producer.clone(), point: source.point })? == (SourceEventId { owner: actual.resolved.producer.clone(), point: actual.point }))
}

fn operation_pair(remap: &ValueSchemaRemap, source: &AssignmentOperationId, transported: &AssignmentOperationId, actual: &AssignmentOperationId) -> Result<bool, ValueSchemaRemapError> {
    Ok(*transported == remap.operation(source)? && transported.occurrence == source.occurrence
        && source.occurrence.source_span.source == *lexical(&source.producer)
        && actual.occurrence.source_span.source == *lexical(&actual.producer)
        && source.occurrence.source_span.range == actual.occurrence.source_span.range
        && source.occurrence.output_projection == actual.occurrence.output_projection && transported.producer == actual.producer)
}

fn saved_constructor(cfg: &ScalarReferenceCfg, slot: &SourceSlotFact, selection: &FrozenSelectionOccurrence) -> bool {
    let table = &cfg.evaluated_values;
    if selection.application.frame.owner != cfg.producer_owner || selection.application.operation != selection.destination.operation
        || selection.destination.operation.producer != cfg.producer_owner { return false; }
    let destinations = table.destinations.iter().filter(|row| row.id == selection.destination && row.shape == slot.shape).collect::<Vec<_>>();
    let [destination] = destinations.as_slice() else { return false; };
    let snapshots = table.snapshots.iter().filter(|row| row.destination == selection.destination).collect::<Vec<_>>();
    let [snapshot] = snapshots.as_slice() else { return false; };
    let evaluations = table.assignment_evaluations.iter().filter(|row| row.operation == selection.application.operation).collect::<Vec<_>>();
    let [evaluation] = evaluations.as_slice() else { return false; };
    if evaluation.destinations.get(selection.destination.target) != Some(*destination) || evaluation.previous.get(selection.destination.target) != Some(*snapshot)
        || !cfg.initial_points.iter().any(|point| point.id == destination.point && matches!(point.kind, CfgPointKind::AssignmentDestination { .. }))
        || !cfg.initial_points.iter().any(|point| point.id == snapshot.point && matches!(&point.kind, CfgPointKind::AssignmentSnapshot { destination_point, .. } if *destination_point == destination.point)) { return false; }
    for (role, point) in [(ScalarAssignmentPointRole::Destination, destination.point), (ScalarAssignmentPointRole::PreviousSnapshot, snapshot.point)] {
        if cfg.assignment_points.iter().filter(|row| row.occurrence == selection.destination.operation.occurrence && row.target == selection.destination.target && row.role == role && row.point == point).count() != 1 { return false; }
    }
    let saved = table.source_saved_ports.iter().filter(|row| row.selection == *selection && row.selected.coordinate == slot.id).collect::<Vec<_>>();
    !saved.is_empty() && saved.iter().all(|row| {
        row.snapshot == (SourceEventId { owner: cfg.producer_owner.clone(), point: snapshot.point })
            && table.source_port_monitors.iter().filter(|monitor| monitor.id == row.monitor && monitor.ports.contains(&row.selected)).count() == 1
            && table.source_configurations.iter().any(|configuration| configuration.frame == selection.application.frame && configuration.saved.iter().any(|fact| fact.selection == *selection && fact.snapshot == row.snapshot && fact.coordinate == slot.id))
    })
}

fn saved_pair(remap: &ValueSchemaRemap, source: &FrozenSelectionOccurrence, transported: &FrozenSelectionOccurrence, actual: &FrozenSelectionOccurrence, before: &ScalarReferenceCfg, after: &ScalarReferenceCfg) -> Result<bool, ValueSchemaRemapError> {
    if transported.application.frame != remap.source_frame(&source.application.frame)? || transported.application.frame != actual.application.frame
        || transported.destination.target != actual.destination.target || source.destination.target != transported.destination.target
        || !operation_pair(remap, &source.application.operation, &transported.application.operation, &actual.application.operation)?
        || !operation_pair(remap, &source.destination.operation, &transported.destination.operation, &actual.destination.operation)? { return Ok(false); }
    let originals = before.assignment_points.iter().filter(|row| row.occurrence == source.destination.operation.occurrence && row.target == source.destination.target).collect::<Vec<_>>();
    let targets = after.assignment_points.iter().filter(|row| row.occurrence == actual.destination.operation.occurrence && row.target == actual.destination.target).collect::<Vec<_>>();
    if originals.is_empty() || originals.len() != targets.len() { return Ok(false); }
    for original in originals {
        let point = remap.boundary_point(&SourceEventId { owner: before.producer_owner.clone(), point: original.point })?;
        if targets.iter().filter(|row| row.role == original.role && row.point == point.point && point.owner == after.producer_owner).count() != 1 { return Ok(false); }
    }
    Ok(true)
}

pub(super) fn matches(remap: &ValueSchemaRemap, source: &SourceSlotFact, transported: &SourceCoordinateRole, destination: &SourceSlotFact, input: &BoundaryHeaderRoleInput<'_>, types: &input_counterpart::BoundaryInputTypeEvidence) -> Result<bool, ValueSchemaRemapError> {
    let BoundaryHeaderRoleInput::Independent { source: sources, destination: destinations, .. } = input else { return Ok(*transported == destination.role); };
    let before = original_slot(sources, source)?;
    let after = match original_slot(destinations, destination) {
        Ok(cfg) => cfg,
        Err(ValueSchemaRemapError::MissingBoundarySlot(_)) => return Ok(false),
        Err(error) => return Err(error),
    };
    original_pair(remap, source, transported, destination, before, after, types)
}

pub(super) fn formal_matches(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &BoundaryPortRole, transported: &BoundaryPortRole, destination: &BoundaryPortRole, types: &input_counterpart::BoundaryInputTypeEvidence) -> Result<bool, ValueSchemaRemapError> {
    if remap.boundary_role(owner, source)? != *transported { return Ok(false); }
    match types {
        input_counterpart::BoundaryInputTypeEvidence::Transported => Ok(*transported == *destination),
        input_counterpart::BoundaryInputTypeEvidence::Independent(checked) => checked.formal_role_pair(remap, owner, source, transported, destination),
    }
}

fn original_pair(remap: &ValueSchemaRemap, source: &SourceSlotFact, transported: &SourceCoordinateRole, destination: &SourceSlotFact, before: &ScalarReferenceCfg, after: &ScalarReferenceCfg, types: &input_counterpart::BoundaryInputTypeEvidence) -> Result<bool, ValueSchemaRemapError> {
    match (&source.role, transported, &destination.role) {
        (SourceCoordinateRole::Evaluated(original), SourceCoordinateRole::Evaluated(mapped), SourceCoordinateRole::Evaluated(actual)) => {
            Ok(value_constructor(before, original, &source.shape) && value_constructor(after, actual, &destination.shape) && value_pair(remap, original, mapped, actual)?)
        }
        (SourceCoordinateRole::Saved(original), SourceCoordinateRole::Saved(mapped), SourceCoordinateRole::Saved(actual)) => {
            Ok(saved_constructor(before, source, original) && saved_constructor(after, destination, actual) && saved_pair(remap, original, mapped, actual, before, after)?)
        }
        (SourceCoordinateRole::Boundary(original), SourceCoordinateRole::Boundary(mapped), SourceCoordinateRole::Boundary(actual)) => formal_matches(remap, &source.id.owner, original, mapped, actual, types),
        (SourceCoordinateRole::Binding(_), SourceCoordinateRole::Binding(_), SourceCoordinateRole::Binding(_))
        | (SourceCoordinateRole::Owner(_), SourceCoordinateRole::Owner(_), SourceCoordinateRole::Owner(_)) => Ok(*transported == destination.role),
        _ => Ok(false),
    }
}

fn snapshot<'a>(originals: &'a [BoundaryResourceConstructionEvidence], owner: &ProducerCfgOwnerId) -> Result<&'a ScalarReferenceCfg, ValueSchemaRemapError> {
    let rows = originals.iter().filter(|row| row.original().producer_owner == *owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
    Ok(row.original())
}

pub(super) fn postvalidate(remap: &ValueSchemaRemap, source: &[ValueTableSource], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    input_counterpart::postvalidate(remap, source, correspondence, resources)?;
    name_resolution::validate_owned_originals(source, resources)?;
    if resources.header_inputs() != correspondence.header_inputs { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    let demanded = source.iter().flat_map(|row| &row.tables.source_slots).collect::<Vec<_>>();
    if demanded.len() != resources.header_inputs().len() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    for (slot, association) in demanded.into_iter().zip(resources.header_inputs()) {
        let invalid = || ValueSchemaRemapError::MissingBoundarySlot(slot.id.clone());
        let before = snapshot(resources.source_originals(), &slot.id.owner)?;
        let carriers = source.iter().filter(|row| row.producer == slot.id.owner).collect::<Vec<_>>();
        let [carrier] = carriers.as_slice() else { return Err(invalid()); };
        if carrier.resource_construction.original() != before || before.evaluated_values.source_slots.iter().filter(|row| *row == slot).count() != 1
            || association.source() != slot || association.transported() != &remap.source_role(&slot.id.owner, &slot.role)?
            || correspondence.slot(&slot.id)? != association.destination().id || remap.boundary_shape(&slot.shape)? != association.destination().shape
            || remap.boundary_producer(&slot.id.owner)? != association.destination().id.owner { return Err(invalid()); }
        match resources.provenance() {
            resources::BoundaryResourceProvenance::Copied { .. } => {
                if association.transported() != &association.destination().role { return Err(invalid()); }
            }
            resources::BoundaryResourceProvenance::Independent { originals } => {
                let after = snapshot(originals, &association.destination().id.owner)?;
                if after.evaluated_values.source_slots.iter().filter(|row| *row == association.destination()).count() != 1
                    || !original_pair(remap, slot, association.transported(), association.destination(), before, after, resources.input_types())? { return Err(invalid()); }
            }
        }
    }
    for carrier in source {
        if snapshot(resources.source_originals(), &carrier.producer)? != carrier.resource_construction.original() { return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone())); }
        if let resources::BoundaryResourceProvenance::Independent { originals } = resources.provenance() {
            snapshot(originals, &remap.boundary_producer(&carrier.producer)?)?;
        }
        for signature in &carrier.tables.boundary_signatures {
            for (port, formal) in signature.ports.iter().enumerate() {
                let original = BoundaryFormalPortRef { owner: signature.owner.clone(), port };
                let mapped = correspondence.formal(&original)?;
                let coordinate = correspondence.slot(&formal.coordinate)?;
                let rows = resources.header_inputs().iter().filter(|association| association.source().id == formal.coordinate && association.destination().id == coordinate).collect::<Vec<_>>();
                let [association] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(original)); };
                let SourceCoordinateRole::Boundary(actual) = &association.destination().role else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(original)); };
                if !formal_matches(remap, &signature.owner, &formal.role, &remap.boundary_role(&signature.owner, &formal.role)?, actual, resources.input_types())?
                    || association.destination().shape != remap.boundary_shape(&formal.shape)? || mapped.owner != association.destination().id.owner { return Err(ValueSchemaRemapError::MissingBoundaryFormal(original)); }
            }
        }
    }
    Ok(())
}

pub(super) fn full_headers(remap: &ValueSchemaRemap, source: &[ValueTableSource], mapped: &[RemappedValueTable], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    for carrier in source {
        let rows = mapped.iter().filter(|row| row.source == carrier.producer).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone())); };
        let expected = match resources.provenance() {
            resources::BoundaryResourceProvenance::Independent { originals } => snapshot(originals, &row.destination)?.evaluated_values.source_slots.clone(),
            resources::BoundaryResourceProvenance::Copied { .. } => resources.header_inputs().iter().filter(|pair| pair.source().id.owner == carrier.producer).map(|pair| pair.destination().clone()).collect(),
        };
        if row.tables.source_slots != expected { return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone())); }
        for pair in resources.header_inputs().iter().filter(|pair| pair.source().id.owner == carrier.producer) {
            if let SourceCoordinateRole::Evaluated(value) = pair.transported() {
                if row.tables.values.iter().filter(|fact| fact.id == *value && fact.shape == pair.destination().shape).count() != 1 { return Err(ValueSchemaRemapError::MissingBoundarySlot(pair.source().id.clone())); }
            }
        }
        if row.tables.source_saved_ports != remap.source_saved_ports(&carrier.tables, correspondence)?
            || row.tables.snapshots != carrier.tables.snapshots.iter().map(|snapshot| remap.snapshot(snapshot)).collect::<Result<Vec<_>, _>>()?
            || row.tables.destinations != carrier.tables.destinations.iter().map(|destination| remap.destination(destination)).collect::<Result<Vec<_>, _>>()?
            || row.tables.assignment_evaluations != carrier.tables.assignment_evaluations.iter().map(|evaluation| remap.assignment_evaluation(evaluation)).collect::<Result<Vec<_>, _>>()? {
            return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone()));
        }
    }
    Ok(())
}

pub(super) fn selective_headers(headers: &[BoundaryTableHeaders], resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    if let resources::BoundaryResourceProvenance::Independent { originals } = resources.provenance() {
        for header in headers {
            let cfg = snapshot(originals, &header.producer)?;
            if header.tables.source_slots != cfg.evaluated_values.source_slots || header.tables.boundary_signatures != cfg.evaluated_values.boundary_signatures { return Err(ValueSchemaRemapError::MissingProducer(header.producer.clone())); }
        }
    }
    for pair in resources.header_inputs() {
        if headers.iter().filter(|row| row.producer == pair.destination().id.owner).flat_map(|row| &row.tables.source_slots).filter(|slot| *slot == pair.destination()).count() != 1 { return Err(ValueSchemaRemapError::MissingBoundarySlot(pair.source().id.clone())); }
    }
    Ok(())
}
