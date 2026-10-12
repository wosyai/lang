use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFormalMemberPort {
    pub member: SourceShapeMemberPort,
    pub value: SourceFormalValueRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceFormalParameterContract {
    CallerFixed,
    ParameterOwned,
    Deferred,
    Manual,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceFormalOwnerRole {
    Parameter {
        binding: ReferenceBindingId,
        capability: OwnerCapabilityId,
        contract: SourceFormalParameterContract,
    },
    Capture { formal: BoundaryFormalPortRef, binding: ReferenceBindingId },
    Global { formal: BoundaryFormalPortRef, binding: ReferenceBindingId },
    Manual { formal: BoundaryFormalPortRef },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceFormalVertexRole {
    Root,
    Member(SourceShapeMemberPort),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFormalValue {
    pub id: SourceFormalValueRef,
    pub role: SourceFormalVertexRole,
    pub port: BoundaryFormalPortRef,
    pub shape: ValueShapeId,
    pub frame: FrameVariableRef,
    pub value_instance: SourceInstanceRef,
    pub members: Vec<SourceFormalMemberPort>,
    pub grants: Vec<SourceGrantClassRef>,
    pub owner: SourceFormalOwnerRole,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFormalReadWitness {
    pub event: SourceEventId,
    pub source: SourceFormalValueRef,
    pub value: EvaluatedValueId,
    pub input: SourceConfigurationRef,
    pub output: SourceConfigurationRef,
    pub contributor: JointProducerTupleRef,
    pub monitor: JointPortMonitorRef,
    pub selected: JointSelectedPortFact,
    pub projections: Vec<ReferencePlaceProjection>,
    pub indexes: Vec<CfgPointId>,
}

pub(in crate::scalar) fn shape_members(shape: &ResolvedValueShape) -> Vec<ValueShapeId> {
    match &shape.payload {
        ValuePayloadShape::Struct { fields, .. } => fields.iter().map(|field| field.shape.clone()).collect(),
        ValuePayloadShape::Enum { variants, .. } => variants.iter().flat_map(|variant| variant.fields.iter().cloned()).collect(),
        ValuePayloadShape::FixedArray { element, .. } | ValuePayloadShape::RuntimeArray { element }
            | ValuePayloadShape::Checked { referent: element, .. } | ValuePayloadShape::Raw { pointee: element } => vec![element.clone()],
        ValuePayloadShape::Plain(_) | ValuePayloadShape::Callable { .. } | ValuePayloadShape::TemplateParameter { .. } => Vec::new(),
    }
}

pub(in crate::scalar) fn resolve<'a>(tables: &'a EvaluatedValueTables, id: &SourceFormalValueRef) -> Result<&'a SourceFormalValue, ()> {
    let rows = tables.source_formal_values.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(()); };
    if tables.source_formal_values.get(id.index) != Some(row) { return Err(()); }
    Ok(row)
}

pub(in crate::scalar) fn root<'a>(tables: &'a EvaluatedValueTables, port: &BoundaryFormalPortRef, shape: &ValueShapeId) -> Result<&'a SourceFormalValue, ()> {
    let rows = tables.source_formal_values.iter().filter(|row| row.port == *port && row.shape == *shape && row.role == SourceFormalVertexRole::Root).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(()); };
    resolve(tables, &row.id)
}

fn parameter_contract(shape: &ResolvedValueShape) -> SourceFormalParameterContract {
    match shape.payload {
        ValuePayloadShape::FixedArray { .. } | ValuePayloadShape::Checked { .. } => SourceFormalParameterContract::CallerFixed,
        ValuePayloadShape::Raw { .. } => SourceFormalParameterContract::Manual,
        ValuePayloadShape::TemplateParameter { .. } => SourceFormalParameterContract::Deferred,
        _ => SourceFormalParameterContract::ParameterOwned,
    }
}

fn construct(cfg: &ScalarReferenceCfg) -> Result<(Vec<SourceFormalValue>, Vec<SourceSupportedClassFact>), ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    let fail = || invalid(&cfg.points[0].source_span);
    let mut rows = Vec::new();
    let mut instances = Vec::new();
    for signature in &tables.boundary_signatures {
        for (ordinal, input) in signature.ports.iter().enumerate() {
            if matches!(input.role, BoundaryPortRole::Output(_) | BoundaryPortRole::Effect { .. }) { continue; }
            let shape = tables.shapes.iter().find(|row| row.id == input.shape).ok_or_else(fail)?;
            if matches!(shape.payload, ValuePayloadShape::Plain(_)) { continue; }
            let port = BoundaryFormalPortRef { owner: signature.owner.clone(), port: ordinal };
            let owner = match &input.role {
                BoundaryPortRole::Parameter { binding, .. } => {
                    let owners = tables.owner_capabilities.iter().filter(|row| row.role == OwnerRole::Parameter(binding.clone())).collect::<Vec<_>>();
                    let [capability] = owners.as_slice() else { return Err(fail()); };
                    if capability.frame != signature.frame { return Err(fail()); }
                    SourceFormalOwnerRole::Parameter { binding: binding.clone(), capability: capability.id.clone(), contract: parameter_contract(shape) }
                },
                BoundaryPortRole::Capture { binding, .. } => SourceFormalOwnerRole::Capture { formal: port.clone(), binding: binding.clone() },
                BoundaryPortRole::Global(binding) => SourceFormalOwnerRole::Global { formal: port.clone(), binding: binding.clone() },
                _ => return Err(fail()),
            };
            let mut vertices = vec![input.shape.clone()];
            let mut next = 0;
            while next < vertices.len() {
                let id = &vertices[next];
                next += 1;
                let shapes = tables.shapes.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
                let [shape] = shapes.as_slice() else { return Err(fail()); };
                if shape.id.producer != signature.owner { return Err(fail()); }
                for child in shape_members(shape) { if !vertices.contains(&child) { vertices.push(child); } }
            }
            let mut roles = vec![(SourceFormalVertexRole::Root, input.shape.clone())];
            for vertex in &vertices {
                let schema = tables.shapes.iter().find(|row| row.id == *vertex).ok_or_else(fail)?;
                for (member, child) in shape_members(schema).into_iter().enumerate() {
                    roles.push((SourceFormalVertexRole::Member(SourceShapeMemberPort { shape: vertex.clone(), member }), child));
                }
            }
            let start = rows.len();
            for (role, vertex) in &roles {
                let id = SourceFormalValueRef { owner: signature.owner.clone(), index: rows.len() };
                let instance = SourceSupportedClassFact {
                    id: SourceSupportedClassRef { owner: signature.owner.clone(), index: instances.len() },
                    variable: SupportedInstanceVariable(instances.len()),
                    frame: signature.frame.clone(), support: input.coordinate.clone(), shape: vertex.clone(),
                    role: SourceObjectRole::Payload, origin: SourceInstanceOrigin::Formal(port.clone()),
                };
                rows.push(SourceFormalValue { id, role: role.clone(), port: port.clone(), shape: vertex.clone(), frame: signature.frame.clone(),
                    value_instance: SourceInstanceRef::Supported(instance.id.clone()), members: Vec::new(), grants: Vec::new(), owner: owner.clone() });
                instances.push(instance);
            }
            for index in start..rows.len() {
                let shape = tables.shapes.iter().find(|shape| shape.id == rows[index].shape).ok_or_else(fail)?;
                let mut members = Vec::new();
                for (member, _) in shape_members(shape).iter().enumerate() {
                    let original = SourceShapeMemberPort { shape: rows[index].shape.clone(), member };
                    let child = roles.iter().position(|(role, _)| *role == SourceFormalVertexRole::Member(original.clone())).ok_or_else(fail)?;
                    members.push(SourceFormalMemberPort { member: original, value: rows[start + child].id.clone() });
                }
                rows[index].members = members;
                if matches!(shape.payload, ValuePayloadShape::Checked { .. }) {
                    let grant = SourceGrantClassRef { owner: signature.owner.clone(), variable: SupportedInstanceVariable(rows[index].id.index) };
                    rows[index].grants.push(grant);
                }
            }
        }
    }
    Ok((rows, instances))
}

pub(in crate::scalar) fn register(cfg: &mut ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    if !cfg.evaluated_values.source_formal_values.is_empty() || !cfg.evaluated_values.source_instances.is_empty() { return Err(invalid(&cfg.points[0].source_span)); }
    validate_signatures(cfg)?;
    let (rows, instances) = construct(cfg)?;
    let mut candidate = cfg.clone();
    candidate.evaluated_values.source_formal_values = rows;
    candidate.evaluated_values.source_instances = instances;
    validate(&candidate)?;
    cfg.evaluated_values.source_formal_values = candidate.evaluated_values.source_formal_values;
    cfg.evaluated_values.source_instances = candidate.evaluated_values.source_instances;
    Ok(())
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    validate_signatures(cfg)?;
    owner_lifetime::validate(cfg)?;
    let tables = &cfg.evaluated_values;
    let fail = || invalid(&cfg.points[0].source_span);
    let mut required = Vec::new();
    for signature in &tables.boundary_signatures {
        for (ordinal, port) in signature.ports.iter().enumerate() {
            if matches!(port.role, BoundaryPortRole::Output(_) | BoundaryPortRole::Effect { .. }) { continue; }
            let schemas = tables.shapes.iter().filter(|shape| shape.id == port.shape).collect::<Vec<_>>();
            let [schema] = schemas.as_slice() else { return Err(fail()); };
            if matches!(schema.payload, ValuePayloadShape::Plain(_)) { continue; }
            let formal = BoundaryFormalPortRef { owner: signature.owner.clone(), port: ordinal };
            required.push((formal.clone(), SourceFormalVertexRole::Root, port.shape.clone()));
            let mut pending = vec![port.shape.clone()];
            let mut visited = Vec::new();
            while let Some(id) = pending.pop() {
                if visited.contains(&id) { continue; }
                visited.push(id.clone());
                let schemas = tables.shapes.iter().filter(|shape| shape.id == id).collect::<Vec<_>>();
                let [schema] = schemas.as_slice() else { return Err(fail()); };
                if id.producer != signature.owner { return Err(fail()); }
                for (member, child) in shape_members(schema).into_iter().enumerate() {
                    required.push((formal.clone(), SourceFormalVertexRole::Member(SourceShapeMemberPort { shape: id.clone(), member }), child.clone()));
                    pending.push(child);
                }
            }
        }
    }
    if required.len() != tables.source_formal_values.len() { return Err(fail()); }
    for (port, role, shape) in &required {
        if tables.source_formal_values.iter().filter(|row| row.port == *port && row.role == *role && row.shape == *shape).count() != 1 { return Err(fail()); }
    }
    for (index, row) in tables.source_formal_values.iter().enumerate() {
        if row.id.index != index || row.id.owner != row.port.owner
            || !required.contains(&(row.port.clone(), row.role.clone(), row.shape.clone())) { return Err(fail()); }
        let signatures = tables.boundary_signatures.iter().filter(|signature| signature.owner == row.port.owner).collect::<Vec<_>>();
        let [signature] = signatures.as_slice() else { return Err(fail()); };
        let port = signature.ports.get(row.port.port).ok_or_else(fail)?;
        let schemas = tables.shapes.iter().filter(|shape| shape.id == row.shape).collect::<Vec<_>>();
        let [schema] = schemas.as_slice() else { return Err(fail()); };
        if row.frame != signature.frame { return Err(fail()); }
        match (&row.owner, &port.role) {
            (SourceFormalOwnerRole::Parameter { binding, capability, contract }, BoundaryPortRole::Parameter { binding: actual, .. }) => {
                let owners = tables.owner_capabilities.iter().filter(|owner| owner.id == *capability && owner.role == OwnerRole::Parameter(actual.clone())).collect::<Vec<_>>();
                let [owner] = owners.as_slice() else { return Err(fail()); };
                let root_shape = tables.shapes.iter().find(|shape| shape.id == port.shape).ok_or_else(fail)?;
                if binding != actual || owner.frame != row.frame || *contract != parameter_contract(root_shape) { return Err(fail()); }
            },
            (SourceFormalOwnerRole::Capture { formal, binding }, BoundaryPortRole::Capture { binding: actual, .. })
                | (SourceFormalOwnerRole::Global { formal, binding }, BoundaryPortRole::Global(actual)) => {
                    if formal != &row.port || binding != actual { return Err(fail()); }
                },
            _ => return Err(fail()),
        }
        let members = shape_members(schema);
        if members.len() != row.members.len() { return Err(fail()); }
        for (member, (edge, shape)) in row.members.iter().zip(members).enumerate() {
            let original = SourceShapeMemberPort { shape: row.shape.clone(), member };
            let child = resolve(tables, &edge.value).map_err(|_| fail())?;
            if edge.member != original || child.port != row.port || child.shape != shape
                || child.role != SourceFormalVertexRole::Member(original) { return Err(fail()); }
        }
        let SourceInstanceRef::Supported(id) = &row.value_instance else { return Err(invalid(&cfg.points[0].source_span)); };
        let actual = tables.source_instances.get(id.index).ok_or_else(fail)?;
        if actual.id != *id || actual.shape != row.shape || actual.frame != row.frame
            || actual.origin != SourceInstanceOrigin::Formal(row.port.clone())
            || actual.support != port.coordinate || actual.role != SourceObjectRole::Payload
            || actual.variable != SupportedInstanceVariable(id.index)
            || tables.source_formal_values.iter().filter(|formal| formal.value_instance == row.value_instance).count() != 1 { return Err(fail()); }
        if tables.source_configurations.iter().any(|configuration|
            configuration.instances.contains(&row.value_instance)
                || configuration.created.iter().any(|created| created.instance == row.value_instance)
                || configuration.owns.iter().any(|owns| owns.payload == row.value_instance || owns.owner_instance == row.value_instance)) { return Err(fail()); }
    }
    if tables.source_instances.iter().filter(|instance| matches!(instance.origin, SourceInstanceOrigin::Formal(_))).count() != tables.source_formal_values.len() { return Err(fail()); }
    Ok(())
}
