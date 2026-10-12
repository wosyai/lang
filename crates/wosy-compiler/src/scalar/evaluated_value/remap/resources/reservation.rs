use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReservedResourceVertex {
    pub(super) source: BoundaryResourceRef,
    pub(super) source_vertex: BoundaryResourceVertexId,
    pub(super) destination_vertex: BoundaryResourceVertexId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResourceReservations {
    pub(super) vertices: Vec<ReservedResourceVertex>,
}

impl ResourceReservations {
    pub(super) fn original(closure: &original::OriginalResourceClosure) -> Self {
        let mut vertices: Vec<ReservedResourceVertex> = Vec::new();
        for row in &closure.vertices {
            let ordinal = vertices.iter().filter(|vertex| vertex.source.owner == row.reference.owner).count();
            let id = BoundaryResourceVertexId { owner: row.reference.owner.clone(), ordinal };
            vertices.push(ReservedResourceVertex { source: row.reference.clone(), source_vertex: id.clone(), destination_vertex: id });
        }
        Self { vertices }
    }

    pub(super) fn subset(&self, closure: &original::OriginalResourceClosure) -> Result<Self, ValueSchemaRemapError> {
        let mut vertices = Vec::new();
        for original in &closure.vertices {
            let rows = self.vertices.iter().filter(|row| row.source == original.reference).collect::<Vec<_>>();
            let [row] = rows.as_slice() else { return Err(original::missing(&original.reference)); };
            vertices.push((*row).clone());
        }
        Ok(Self { vertices })
    }

    pub(super) fn copied(remap: &ValueSchemaRemap, closure: &original::OriginalResourceClosure) -> Result<Self, ValueSchemaRemapError> {
        let mut vertices: Vec<ReservedResourceVertex> = Vec::new();
        let mut owners = Vec::new();
        for original in &closure.vertices {
            let owner = allocation::qualified_owner(remap, &original.reference.owner)?;
            if owners.iter().any(|(source, destination)| *destination == owner && *source != original.reference.owner) {
                return Err(ValueSchemaRemapError::AmbiguousBoundarySignature(owner));
            }
            if !owners.contains(&(original.reference.owner.clone(), owner.clone())) {
                owners.push((original.reference.owner.clone(), owner.clone()));
            }
            let ordinal = vertices.iter().filter(|row| row.source.owner == original.reference.owner).count();
            vertices.push(ReservedResourceVertex {
                source: original.reference.clone(),
                source_vertex: BoundaryResourceVertexId { owner: original.reference.owner.clone(), ordinal },
                destination_vertex: BoundaryResourceVertexId { owner, ordinal },
            });
        }
        Ok(Self { vertices })
    }

    pub(super) fn destination(&self, reference: &BoundaryResourceRef) -> Result<BoundaryResourceVertexId, ValueSchemaRemapError> {
        let rows = self.vertices.iter().filter(|row| row.source == *reference).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { return Err(original::missing(reference)); };
        Ok(row.destination_vertex.clone())
    }

    pub(super) fn fill(&self, closure: &original::OriginalResourceClosure, values: &[BoundaryResourceValue]) -> Result<Vec<BoundaryResourceArena>, ValueSchemaRemapError> {
        if self.vertices.len() != closure.vertices.len() || values.len() != self.vertices.len() {
            return Err(ValueSchemaRemapError::EmptyTransportSources);
        }
        let mut arenas: Vec<BoundaryResourceArena> = Vec::new();
        for ((original, reservation), value) in closure.vertices.iter().zip(&self.vertices).zip(values) {
            if original.reference != reservation.source { return Err(original::missing(&original.reference)); }
            let dependencies = original.dependencies.iter().map(|reference| self.destination(reference)).collect::<Result<Vec<_>, _>>()?;
            let owner = &reservation.destination_vertex.owner;
            if !arenas.iter().any(|arena| arena.owner == *owner) {
                arenas.push(BoundaryResourceArena { owner: owner.clone(), vertices: Vec::new() });
            }
            let mut rows = arenas.iter_mut().filter(|arena| arena.owner == *owner).collect::<Vec<_>>();
            let [arena] = rows.as_mut_slice() else { return Err(original::missing(&original.reference)); };
            arena.vertices.push(BoundaryResourceVertex { id: reservation.destination_vertex.clone(), value: value.clone(), dependencies });
        }
        for arena in &arenas {
            for vertex in &arena.vertices {
                for dependency in &vertex.dependencies {
                    if arenas.iter().flat_map(|arena| &arena.vertices).filter(|vertex| vertex.id == *dependency).count() != 1 {
                        return Err(ValueSchemaRemapError::MissingProducer(dependency.owner.clone()));
                    }
                }
            }
        }
        Ok(arenas)
    }
}
