use super::*;

pub(super) fn key(value: &BoundaryResourceValue) -> BoundaryResourceKey {
    match value {
        BoundaryResourceValue::Origin { id, .. } => BoundaryResourceKey::Origin(*id),
        BoundaryResourceValue::Binding(binding) => BoundaryResourceKey::Binding(binding.clone()),
        BoundaryResourceValue::Place(place) => BoundaryResourceKey::Place(place.clone()),
        BoundaryResourceValue::Loan(loan) => BoundaryResourceKey::Loan(loan.clone()),
        BoundaryResourceValue::Runtime(runtime) => BoundaryResourceKey::Runtime(runtime.clone()),
        BoundaryResourceValue::Environment { id, .. } => BoundaryResourceKey::Environment(id.clone()),
        BoundaryResourceValue::Backing(backing) => BoundaryResourceKey::Backing(backing.clone()),
        BoundaryResourceValue::Holder(holder) => BoundaryResourceKey::Holder(holder.clone()),
    }
}

pub(super) fn proposed(remap: &ValueSchemaRemap, reference: &BoundaryResourceRef) -> Result<BoundaryResourceRef, ValueSchemaRemapError> {
    let owner = qualified_owner(remap, &reference.owner)?;
    if !remap.producers.iter().any(|row| row.0 == reference.owner) { return Ok(reference.clone()); }
    let key = match &reference.key {
        BoundaryResourceKey::Origin(origin) => BoundaryResourceKey::Origin(remap.origin(&reference.owner, *origin)?),
        BoundaryResourceKey::Binding(binding) => BoundaryResourceKey::Binding(remap.binding(&reference.owner, binding)?),
        BoundaryResourceKey::Place(place) => BoundaryResourceKey::Place(remap.place(&reference.owner, place)?),
        BoundaryResourceKey::Loan(loan) => BoundaryResourceKey::Loan(remap.checked_loan(&reference.owner, loan)?),
        BoundaryResourceKey::Runtime(runtime) => BoundaryResourceKey::Runtime(remap.runtime(&reference.owner, runtime)?),
        BoundaryResourceKey::Environment(environment) => BoundaryResourceKey::Environment(remap.checked_environment(&reference.owner, environment)?),
        BoundaryResourceKey::Backing(backing) => BoundaryResourceKey::Backing(remap.checked_backing(&reference.owner, backing)?),
        BoundaryResourceKey::Holder(holder) => BoundaryResourceKey::Holder(remap.checked_holder(&reference.owner, holder)?),
    };
    Ok(BoundaryResourceRef { owner, key })
}

pub(super) fn qualified_owner(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId) -> Result<ProducerCfgOwnerId, ValueSchemaRemapError> {
    let rows = remap.producers.iter().filter(|row| row.0 == *owner).collect::<Vec<_>>();
    match rows.as_slice() {
        [] => Ok(owner.clone()),
        [_] => remap.boundary_producer(owner),
        _ => Err(ValueSchemaRemapError::MissingProducer(owner.clone())),
    }
}

pub(super) fn association<'a>(resources: &'a [BoundaryResourceAssociation], reference: &BoundaryResourceRef) -> Result<&'a BoundaryResourceAssociation, ValueSchemaRemapError> {
    let rows = resources.iter().filter(|row| row.source == *reference).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(original::missing(reference)); };
    Ok(row)
}

fn mapped_binding(associations: &[BoundaryResourceAssociation], owner: &ProducerCfgOwnerId, binding: &ReferenceBindingId) -> Result<ReferenceBindingId, ValueSchemaRemapError> {
    let reference = BoundaryResourceRef { owner: owner.clone(), key: BoundaryResourceKey::Binding(binding.clone()) };
    match &association(associations, &reference)?.destination.key {
        BoundaryResourceKey::Binding(binding) => Ok(binding.clone()),
        _ => Err(original::missing(&reference)),
    }
}

fn mapped_holder(associations: &[BoundaryResourceAssociation], owner: &ProducerCfgOwnerId, holder: &ConnectedCaptureHolderFact) -> Result<ConnectedCaptureHolderFact, ValueSchemaRemapError> {
    let reference = BoundaryResourceRef { owner: owner.clone(), key: BoundaryResourceKey::Holder(holder.clone()) };
    match &association(associations, &reference)?.destination.key {
        BoundaryResourceKey::Holder(holder) => Ok(holder.clone()),
        _ => Err(original::missing(&reference)),
    }
}

pub(super) fn mapped_tuple(remap: &ValueSchemaRemap, tuple: &BoundaryCaptureResourceTuple, correspondence: &BoundaryArenaCorrespondence, associations: &[BoundaryResourceAssociation], independent: bool) -> Result<BoundaryCaptureResourceTuple, ValueSchemaRemapError> {
    let mut tables = EvaluatedValueTables::new();
    tables.source_capture_productions.push(tuple.production.clone());
    let productions = remap.capture_productions(&tables, correspondence)?;
    let [production] = productions.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(tuple.production.event.owner.clone())); };
    for (source, destination) in tuple.production.inputs.iter().zip(&production.inputs) {
        if mapped_binding(associations, &tuple.production.event.owner, &source.capture.binding)? != destination.capture.binding {
            return Err(ValueSchemaRemapError::MissingBinding(source.capture.binding.clone()));
        }
    }
    let mut paths = Vec::new();
    for path in &tuple.paths {
        let mut inputs = Vec::new();
        for input in &path.inputs {
            inputs.push(CaptureInputFact {
                binding: mapped_binding(associations, &path.owner, &input.binding)?,
                origin: match &input.origin {
                    CaptureInputOriginFact::Value => CaptureInputOriginFact::Value,
                    CaptureInputOriginFact::Callable { holders } => CaptureInputOriginFact::Callable {
                        holders: holders.iter().map(|holder| mapped_holder(associations, &path.owner, holder)).collect::<Result<Vec<_>, _>>()?,
                    },
                },
            });
        }
        let mut capture_span = path.capture_span.clone();
        if independent && capture_span.source == *copied::lexical(&tuple.production.event.owner) { capture_span.source = copied::lexical(&production.event.owner).clone(); }
        paths.push(CaptureInputPathFact { owner: production.event.owner.clone(), capture: production.event.point, capture_span, inputs });
    }
    Ok(BoundaryCaptureResourceTuple { production: production.clone(), paths })
}

pub(super) fn validate_edges(closure: &original::OriginalResourceClosure, arenas: &[BoundaryResourceArena], associations: &[BoundaryResourceAssociation]) -> Result<(), ValueSchemaRemapError> {
    for original in &closure.vertices {
        let paired = association(associations, &original.reference)?;
        let targets = arenas.iter().flat_map(|arena| &arena.vertices).filter(|vertex| vertex.id == paired.destination_vertex).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(original::missing(&original.reference)); };
        if key(&target.value) != paired.destination.key { return Err(original::missing(&original.reference)); }
        let mut actual = children::references(&paired.destination.owner, &target.value);
        if target.dependencies.len() != original.dependencies.len() || actual.len() > original.dependencies.len() { return Err(original::missing(&original.reference)); }
        if !matches!(&target.value, BoundaryResourceValue::Holder(_)) && actual.len() != original.dependencies.len() { return Err(original::missing(&original.reference)); }
        for ((source, actual), edge) in original.dependencies.iter().zip(&mut actual).zip(&target.dependencies) {
            let child = association(associations, source)?;
            if matches!(&source.key, BoundaryResourceKey::Runtime(_)) { actual.owner = child.destination.owner.clone(); }
            if child.destination != *actual || child.destination_vertex != *edge { return Err(original::missing(source)); }
        }
        for (source, edge) in original.dependencies.iter().zip(&target.dependencies) {
            let child = association(associations, source)?;
            if child.destination_vertex != *edge || arenas.iter().flat_map(|arena| &arena.vertices)
                .filter(|vertex| vertex.id == *edge && key(&vertex.value) == child.destination.key).count() != 1 { return Err(original::missing(source)); }
        }
    }
    Ok(())
}

pub(in crate::scalar::evaluated_value::remap) fn allocate_copied_resources(
    remap: &ValueSchemaRemap,
    source: &[ValueTableSource],
    forest: &SourceCaptureConstructionForest,
    demand: &BoundaryResourceDemand,
    correspondence: &BoundaryArenaCorrespondence,
) -> Result<CheckedBoundaryResources, ValueSchemaRemapError> {
    let tuples = original_capture_tuples(source)?;
    let mut closure = original::closure(demand, forest)?;
    retain_header_originals(&mut closure.originals, source)?;
    let reservations = reservation::ResourceReservations::copied(remap, &closure)?;
    let values = closure.vertices.iter().zip(&reservations.vertices).map(|(original, reserved)| {
        copied::CopyResourceConstruction::new(remap, &original.reference.owner, &reserved.destination_vertex.owner, forest).value(&original.value)
    }).collect::<Result<Vec<_>, _>>()?;
    let arenas = reservations.fill(&closure, &values)?;
    let mut allocations = Vec::new();
    let mut associations = Vec::new();
    for ((original, reserved), value) in closure.vertices.iter().zip(&reservations.vertices).zip(&values) {
        let destination = BoundaryResourceRef { owner: reserved.destination_vertex.owner.clone(), key: key(value) };
        if proposed(remap, &original.reference)? != destination { return Err(original::missing(&original.reference)); }
        allocations.push(BoundaryResourceAllocation {
            source: original.reference.clone(), destination: destination.clone(), source_vertex: reserved.source_vertex.clone(), destination_vertex: reserved.destination_vertex.clone(),
        });
        associations.push(BoundaryResourceAssociation {
            source: original.reference.clone(), destination, source_vertex: reserved.source_vertex.clone(), destination_vertex: reserved.destination_vertex.clone(),
        });
    }
    validate_edges(&closure, &arenas, &associations)?;
    let mut capture_inputs = Vec::new();
    for tuple in tuples {
        let destination = mapped_tuple(remap, &tuple, correspondence, &associations, false)?;
        capture_inputs.push(BoundaryCaptureResourceAssociation { source: tuple, destination });
    }
    let null_constructors = null_constructor::selected(remap, &closure.originals, &associations)?;
    let checked_parameters = checked_parameter::selected(remap, &closure.originals, &associations)?;
    let direct_storage = direct_storage::selected(remap, &closure.originals, &associations)?;
    let allocation_identity = allocation_identity::selected(remap, &closure.originals, &associations)?;
    Ok(CheckedBoundaryResources { source_originals: closure.originals, null_constructors, checked_parameters, direct_storage, allocation_identity, arenas, associations,
        provenance: BoundaryResourceProvenance::Copied { allocations }, capture_inputs, header_inputs: correspondence.header_inputs.clone(), input_types: correspondence.input_types.clone() })
}
