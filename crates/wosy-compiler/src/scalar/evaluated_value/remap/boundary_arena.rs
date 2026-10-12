use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryArenaCorrespondence {
    slots: Vec<(SourceSlotRef, SourceSlotRef)>,
    formals: Vec<(BoundaryFormalPortRef, BoundaryFormalPortRef)>,
    pub(super) header_inputs: Vec<header_counterpart::BoundaryHeaderRoleAssociation>,
    pub(super) input_types: input_counterpart::BoundaryInputTypeEvidence,
}

impl BoundaryArenaCorrespondence {
    pub(super) fn slot(&self, id: &SourceSlotRef) -> Result<SourceSlotRef, ValueSchemaRemapError> {
        let matches = self.slots.iter().filter(|(source, _)| source == id).collect::<Vec<_>>();
        match matches.as_slice() {
            [( _, target)] => Ok(target.clone()),
            [] => Err(ValueSchemaRemapError::MissingBoundarySlot(id.clone())),
            _ => Err(ValueSchemaRemapError::AmbiguousBoundarySlot(id.clone())),
        }
    }

    pub(super) fn formal(&self, id: &BoundaryFormalPortRef) -> Result<BoundaryFormalPortRef, ValueSchemaRemapError> {
        let matches = self.formals.iter().filter(|(source, _)| source == id).collect::<Vec<_>>();
        match matches.as_slice() {
            [( _, target)] => Ok(target.clone()),
            [] => Err(ValueSchemaRemapError::MissingBoundaryFormal(id.clone())),
            _ => Err(ValueSchemaRemapError::AmbiguousBoundaryFormal(id.clone())),
        }
    }
}

impl ValueSchemaRemap {
    pub(super) fn boundary_producer(&self, owner: &ProducerCfgOwnerId) -> Result<ProducerCfgOwnerId, ValueSchemaRemapError> {
        if self.producers.iter().filter(|(source, _)| source == owner).count() != 1 {
            return Err(ValueSchemaRemapError::MissingProducer(owner.clone()));
        }
        self.producer(owner)
    }

    pub(super) fn boundary_shape(&self, shape: &ValueShapeId) -> Result<ValueShapeId, ValueSchemaRemapError> {
        self.boundary_producer(&shape.producer)?;
        if self.shapes.iter().filter(|(source, _)| source == shape).count() != 1 {
            return Err(ValueSchemaRemapError::MissingShape(shape.clone()));
        }
        self.shape(shape)
    }

    pub(super) fn boundary_point(&self, event: &SourceEventId) -> Result<SourceEventId, ValueSchemaRemapError> {
        self.boundary_producer(&event.owner)?;
        if self.points.iter().filter(|row| row.producer == event.owner && row.point == event.point).count() != 1 {
            return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point });
        }
        self.source_event(event)
    }

    pub(super) fn validate_header(tables: &EvaluatedValueTables, owner: &ProducerCfgOwnerId) -> Result<(), ValueSchemaRemapError> {
        for slot in &tables.source_slots {
            if slot.id.owner != *owner || slot.shape.producer != *owner
                || tables.source_slots.get(slot.id.index) != Some(slot)
                || tables.source_slots.iter().filter(|row| row.id == slot.id || (row.role == slot.role && row.shape == slot.shape)).count() != 1
            { return Err(ValueSchemaRemapError::AmbiguousBoundarySlot(slot.id.clone())); }
            let shapes = tables.shapes.iter().filter(|shape| shape.id == slot.shape).collect::<Vec<_>>();
            let [_] = shapes.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(slot.shape.clone())); };
        }
        for signature in &tables.boundary_signatures {
            if signature.owner != *owner || signature.entry.owner != *owner || signature.frame.owner != *owner
                || tables.boundary_signatures.iter().filter(|row| row.owner == signature.owner).count() != 1
            { return Err(ValueSchemaRemapError::AmbiguousBoundarySignature(signature.owner.clone())); }
            for (port, formal) in signature.ports.iter().enumerate() {
                let id = BoundaryFormalPortRef { owner: owner.clone(), port };
                let slots = tables.source_slots.iter().filter(|slot| slot.id == formal.coordinate).collect::<Vec<_>>();
                let [slot] = slots.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(id)); };
                if slot.role != SourceCoordinateRole::Boundary(formal.role.clone()) || slot.shape != formal.shape
                    || signature.ports.iter().filter(|row| row.role == formal.role || row.coordinate == formal.coordinate).count() != 1
                { return Err(ValueSchemaRemapError::AmbiguousBoundaryFormal(id)); }
            }
        }
        Ok(())
    }

    pub(super) fn boundary_arena_correspondence(&self, source: &[ValueTableSource], source_forest: &SourceCaptureConstructionForest, destination: &[ValueTableSource], destination_forest: &SourceCaptureConstructionForest) -> Result<BoundaryArenaCorrespondence, ValueSchemaRemapError> {
        self.validate_forest_sources(source, source_forest)?;
        self.validate_forest_sources(destination, destination_forest)?;
        self.boundary_header_core(&source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &destination.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), header_counterpart::BoundaryHeaderRoleInput::Independent { source, source_forest, destination, destination_forest })
    }

    pub(super) fn boundary_header_correspondence(&self, source: &[BoundaryTableHeaders], destination: &[BoundaryTableHeaders]) -> Result<BoundaryArenaCorrespondence, ValueSchemaRemapError> {
        self.boundary_header_core(source, destination, header_counterpart::BoundaryHeaderRoleInput::Transported)
    }

    fn boundary_header_core(&self, source: &[BoundaryTableHeaders], destination: &[BoundaryTableHeaders], input: header_counterpart::BoundaryHeaderRoleInput<'_>) -> Result<BoundaryArenaCorrespondence, ValueSchemaRemapError> {
        let input_types = self.checked_qualified_header_shapes(source, destination, &input)?;
        let mut correspondence = BoundaryArenaCorrespondence { slots: Vec::new(), formals: Vec::new(), header_inputs: Vec::new(), input_types };
        for inventory in source {
            Self::validate_header(&inventory.tables, &inventory.producer)?;
            let owner = self.boundary_producer(&inventory.producer)?;
            let targets = destination.iter().filter(|row| row.producer == owner).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner)); };
            Self::validate_header(&target.tables, &owner)?;
            for slot in &inventory.tables.source_slots {
                let shape = self.boundary_shape(&slot.shape)?;
                let role = self.source_role(&inventory.producer, &slot.role)?;
                let mut matches = Vec::new();
                for row in &target.tables.source_slots {
                    if row.id.owner == owner && row.shape == shape && header_counterpart::matches(self, slot, &role, row, &input, &correspondence.input_types)? { matches.push(row); }
                }
                let mapped = match matches.as_slice() {
                    [mapped] => *mapped,
                    [] => return Err(ValueSchemaRemapError::MissingBoundarySlot(slot.id.clone())),
                    _ => return Err(ValueSchemaRemapError::AmbiguousBoundarySlot(slot.id.clone())),
                };
                correspondence.slots.push((slot.id.clone(), mapped.id.clone()));
                correspondence.header_inputs.push(header_counterpart::BoundaryHeaderRoleAssociation::new(slot.clone(), role, mapped.clone()));
            }
            for signature in &inventory.tables.boundary_signatures {
                let entry = self.boundary_point(&signature.entry)?;
                let signatures = target.tables.boundary_signatures.iter().filter(|row| row.owner == owner && row.entry == entry
                    && row.frame.owner == owner && row.frame.variable == signature.frame.variable).collect::<Vec<_>>();
                let matched = match signatures.as_slice() {
                    [signature] => *signature,
                    [] => return Err(ValueSchemaRemapError::MissingBoundarySignature(signature.owner.clone())),
                    _ => return Err(ValueSchemaRemapError::AmbiguousBoundarySignature(signature.owner.clone())),
                };
                for (port, formal) in signature.ports.iter().enumerate() {
                    let id = BoundaryFormalPortRef { owner: signature.owner.clone(), port };
                    let role = self.boundary_role(&signature.owner, &formal.role)?;
                    let shape = self.boundary_shape(&formal.shape)?;
                    let coordinate = correspondence.slot(&formal.coordinate)?;
                    let mut matches = Vec::new();
                    for (position, row) in matched.ports.iter().enumerate() {
                        if row.shape == shape && row.coordinate == coordinate && header_counterpart::formal_matches(self, &signature.owner, &formal.role, &role, &row.role, &correspondence.input_types)? { matches.push((position, row)); }
                    }
                    let [(position, _)] = matches.as_slice() else {
                        return Err(if matches.is_empty() { ValueSchemaRemapError::MissingBoundaryFormal(id) } else { ValueSchemaRemapError::AmbiguousBoundaryFormal(id) });
                    };
                    correspondence.formals.push((id, BoundaryFormalPortRef { owner: owner.clone(), port: *position }));
                }
            }
        }
        Ok(correspondence)
    }
}
