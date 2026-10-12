use super::*;

pub(super) fn preserve(tables: &EvaluatedValueTables, row: &SourceConfigurationFact, point: &ReferenceFlowPoint) -> Result<JointValueTransportFact, ReferenceAnalysisError> {
    let CfgPointKind::Bind { binding } = &point.kind else { return Err(invalid(&point.source_span)); };
    let owners = tables.owner_capabilities.iter().filter(|owner| owner.role == OwnerRole::Parameter(binding.clone()) && owner.introduction.point == point.id && owner.introduction.owner == row.id.owner).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else { return Err(invalid(&point.source_span)); };
    let signatures = tables.boundary_signatures.iter().filter(|signature| signature.owner == row.id.owner).collect::<Vec<_>>();
    let [signature] = signatures.as_slice() else { return Err(invalid(&point.source_span)); };
    let ports = signature.ports.iter().enumerate().filter(|(_, port)| matches!(&port.role, BoundaryPortRole::Parameter { binding: actual, .. } if actual == binding)).collect::<Vec<_>>();
    let [(ordinal, port)] = ports.as_slice() else { return Err(invalid(&point.source_span)); };
    let formal = BoundaryFormalPortRef { owner: row.id.owner.clone(), port: *ordinal };
    let input = row.formal_inputs.iter().filter(|input| input.port == formal && input.coordinate == port.coordinate && input.shape == port.shape).collect::<Vec<_>>();
    if input.len() != 1 || signature.frame != row.frame || owner.frame != row.frame {
        return Err(invalid(&point.source_span));
    }
    let boundary = selected_slot(tables, row, &SourceCoordinateRole::Boundary(port.role.clone())).map_err(|_| invalid(&point.source_span))?;
    let selected = selected_slot(tables, row, &SourceCoordinateRole::Binding(binding.clone())).map_err(|_| invalid(&point.source_span))?;
    if boundary.view != selected.view || boundary.availability != selected.availability
        || selected.availability != ValueAvailability::Available
        || tables.source_views[selected.view.index].shape != port.shape {
        return Err(invalid(&point.source_span));
    }
    Ok(JointValueTransportFact {
        source: selected.coordinate.clone(), destination: selected.coordinate.clone(),
        kind: JointValueTransportKind::Copy, child: JointValueChildPort::LocalInput,
    })
}
