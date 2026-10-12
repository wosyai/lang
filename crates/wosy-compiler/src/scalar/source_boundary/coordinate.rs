use super::*;

pub(in crate::scalar) fn intern_coordinate(
    tables: &mut EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    role: SourceCoordinateRole,
    shape: ValueShapeId,
    source: &SourceSpan,
) -> Result<SourceSlotRef, ReferenceAnalysisError> {
    let shapes = tables.shapes.iter().filter(|row| row.id == shape).collect::<Vec<_>>();
    if shapes.len() != 1 || shape.producer != *owner { return Err(invalid(source)); }
    let rows = tables.source_slots.iter().filter(|row| row.role == role).collect::<Vec<_>>();
    match rows.as_slice() {
        [] => {
            let id = SourceSlotRef { owner: owner.clone(), index: tables.source_slots.len() };
            tables.source_slots.push(SourceSlotFact { id: id.clone(), role, shape });
            Ok(id)
        }
        [row] if row.id.owner == *owner && row.shape == shape
            && tables.source_slots.get(row.id.index) == Some(*row)
            && tables.source_slots.iter().filter(|other| other.id == row.id).count() == 1 => Ok(row.id.clone()),
        _ => Err(invalid(source)),
    }
}
