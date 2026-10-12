use super::*;
use source_boundary::formal_value::SourceFormalVertexRole;

impl ValueSchemaRemap {
    pub(super) fn evaluated_fact_in_arena(&self, value: &EvaluatedValueFact, tables: &EvaluatedValueTables, correspondence: &BoundaryArenaCorrespondence) -> Result<EvaluatedValueFact, ValueSchemaRemapError> {
        let mut cases = Vec::new();
        for case in &value.cases {
            let mapped = if let EvaluatedPayloadFact::Formal { port, view } = &case.payload {
                let formals = tables.source_formal_values.iter().filter(|row| row.id == *view).collect::<Vec<_>>();
                let [formal] = formals.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone())); };
                if formal.port != *port || formal.shape != value.shape || case.id.value != value.id {
                    return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone()));
                }
                let owner = &case.id.value.resolved.producer;
                EvaluatedValueCase {
                    id: self.case(&case.id)?, path: self.path(&case.path)?,
                    availability: self.availability(owner, &case.availability)?,
                    payload: EvaluatedPayloadFact::Formal { port: correspondence.formal(port)?, view: self.source_formal_value(view)? },
                    dependencies: case.dependencies.iter().map(|dependency| self.dependency(owner, dependency)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                }
            } else {
                self.evaluated_case(case)?
            };
            cases.push(mapped);
        }
        Ok(EvaluatedValueFact { id: self.value(&value.id)?, shape: self.shape(&value.shape)?, cases })
    }

    pub(super) fn source_formal_value(&self, id: &SourceFormalValueRef) -> Result<SourceFormalValueRef, ValueSchemaRemapError> {
        Ok(SourceFormalValueRef { owner: self.producer(&id.owner)?, index: id.index })
    }

    pub(super) fn source_formal_row(&self, row: &SourceFormalValue, correspondence: &BoundaryArenaCorrespondence) -> Result<SourceFormalValue, ValueSchemaRemapError> {
        let owner = &row.id.owner;
        Ok(SourceFormalValue {
            id: self.source_formal_value(&row.id)?,
            role: match &row.role {
                SourceFormalVertexRole::Root => SourceFormalVertexRole::Root,
                SourceFormalVertexRole::Member(member) => SourceFormalVertexRole::Member(SourceShapeMemberPort { shape: self.shape(&member.shape)?, member: member.member }),
            },
            port: correspondence.formal(&row.port)?,
            shape: self.shape(&row.shape)?,
            frame: self.source_frame(&row.frame)?,
            value_instance: self.source_instance(&row.value_instance)?,
            members: row.members.iter().map(|edge| Ok(SourceFormalMemberPort {
                member: SourceShapeMemberPort { shape: self.shape(&edge.member.shape)?, member: edge.member.member },
                value: self.source_formal_value(&edge.value)?,
            })).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            grants: row.grants.iter().map(|grant| self.grant_class(grant)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            owner: match &row.owner {
                SourceFormalOwnerRole::Parameter { binding, capability, contract } => SourceFormalOwnerRole::Parameter {
                    binding: self.binding(owner, binding)?, capability: self.capability(capability)?, contract: contract.clone(),
                },
                SourceFormalOwnerRole::Capture { formal, binding } => SourceFormalOwnerRole::Capture {
                    formal: correspondence.formal(formal)?, binding: self.binding(owner, binding)?,
                },
                SourceFormalOwnerRole::Global { formal, binding } => SourceFormalOwnerRole::Global {
                    formal: correspondence.formal(formal)?, binding: self.binding(owner, binding)?,
                },
                SourceFormalOwnerRole::Manual { formal } => SourceFormalOwnerRole::Manual { formal: correspondence.formal(formal)? },
            },
        })
    }

    pub(super) fn source_formal_values(&self, tables: &EvaluatedValueTables, correspondence: &BoundaryArenaCorrespondence) -> Result<Vec<SourceFormalValue>, ValueSchemaRemapError> {
        let reservations = tables.source_formal_values.iter().map(|row| Ok((row.id.clone(), self.source_formal_value(&row.id)?))).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
        let mut result = Vec::new();
        for original in &tables.source_formal_values {
            let row = self.source_formal_row(original, correspondence)?;
            for (before, after) in original.members.iter().zip(&row.members) {
                if reservations.iter().filter(|(source, target)| *source == before.value && *target == after.value).count() != 1 {
                    return Err(ValueSchemaRemapError::MissingProducer(before.value.owner.clone()));
                }
            }
            if reservations.iter().filter(|(source, target)| *source == original.id && *target == row.id).count() != 1 {
                return Err(ValueSchemaRemapError::MissingProducer(original.id.owner.clone()));
            }
            result.push(row);
        }
        Ok(result)
    }

    pub(super) fn source_formal_reads(&self, tables: &EvaluatedValueTables, correspondence: &BoundaryArenaCorrespondence) -> Result<Vec<SourceFormalReadWitness>, ValueSchemaRemapError> {
        tables.source_formal_reads.iter().map(|row| Ok(SourceFormalReadWitness {
            event: self.source_event(&row.event)?, source: self.source_formal_value(&row.source)?,
            value: self.value(&row.value)?, input: self.source_configuration(&row.input)?,
            output: self.source_configuration(&row.output)?, contributor: self.source_tuple(&row.contributor)?,
            monitor: self.source_monitor(&row.monitor)?, selected: self.selected_port(&row.selected, correspondence)?,
            projections: row.projections.iter().map(|projection| Ok(match projection {
                ReferencePlaceProjection::Field(field) => ReferencePlaceProjection::Field(field.clone()),
                ReferencePlaceProjection::Index(expression) => ReferencePlaceProjection::Index(expression.clone()),
                ReferencePlaceProjection::Dereference(loan) => ReferencePlaceProjection::Dereference(Box::new(self.checked_loan(&row.event.owner, loan)?)),
            })).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            indexes: row.indexes.iter().map(|point| self.point(&row.event.owner, *point)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })).collect()
    }
}

pub(super) fn validate_tables(tables: &[(&ProducerCfgOwnerId, &EvaluatedValueTables)]) -> Result<(), ValueSchemaRemapError> {
    formal_completeness::validate_tables(tables)?;
    for (_, table) in tables {
        for (index, row) in table.source_formal_values.iter().enumerate() {
            let fail = || ValueSchemaRemapError::MissingBoundaryFormal(row.port.clone());
            if row.id.index != index || row.id.owner != row.port.owner || row.frame.owner != row.id.owner
                || tables.iter().flat_map(|(_, table)| &table.source_formal_values).filter(|other| other.id == row.id).count() != 1 {
                return Err(fail());
            }
            let signatures = tables.iter().flat_map(|(_, table)| &table.boundary_signatures).filter(|signature| signature.owner == row.port.owner).collect::<Vec<_>>();
            let [signature] = signatures.as_slice() else { return Err(fail()); };
            let port = signature.ports.get(row.port.port).ok_or_else(fail)?;
            if row.frame != signature.frame || matches!(port.role, BoundaryPortRole::Output(_) | BoundaryPortRole::Effect { .. }) { return Err(fail()); }
            let schemas = tables.iter().flat_map(|(_, table)| &table.shapes).filter(|schema| schema.id == row.shape).collect::<Vec<_>>();
            let [schema] = schemas.as_slice() else { return Err(fail()); };
            if schema.id.producer != row.id.owner { return Err(fail()); }
            let root_roles = match &row.role {
                SourceFormalVertexRole::Root => row.shape == port.shape,
                SourceFormalVertexRole::Member(member) => {
                    let parents = tables.iter().flat_map(|(_, table)| &table.shapes).filter(|schema| schema.id == member.shape).collect::<Vec<_>>();
                    matches!(parents.as_slice(), [parent] if source_boundary::formal_value::shape_members(parent).get(member.member) == Some(&row.shape))
                },
            };
            if !root_roles { return Err(fail()); }
            let children = source_boundary::formal_value::shape_members(schema);
            if children.len() != row.members.len() { return Err(fail()); }
            for (member, (edge, shape)) in row.members.iter().zip(children).enumerate() {
                let children = tables.iter().flat_map(|(_, table)| &table.source_formal_values).filter(|child| child.id == edge.value).collect::<Vec<_>>();
                let [child] = children.as_slice() else { return Err(fail()); };
                let expected = SourceShapeMemberPort { shape: row.shape.clone(), member };
                if edge.member != expected || child.port != row.port || child.shape != shape || child.role != SourceFormalVertexRole::Member(expected) { return Err(fail()); }
            }
            let SourceInstanceRef::Supported(id) = &row.value_instance else { return Err(fail()); };
            let instances = tables.iter().flat_map(|(_, table)| &table.source_instances).filter(|instance| instance.id == *id).collect::<Vec<_>>();
            let [instance] = instances.as_slice() else { return Err(fail()); };
            if instance.frame != row.frame || instance.shape != row.shape || instance.support != port.coordinate
                || instance.origin != SourceInstanceOrigin::Formal(row.port.clone()) || instance.role != SourceObjectRole::Payload
                || row.grants.iter().any(|grant| grant.owner != row.id.owner) { return Err(fail()); }
            match (&row.owner, &port.role) {
                (SourceFormalOwnerRole::Parameter { binding, capability, .. }, BoundaryPortRole::Parameter { binding: actual, .. }) => {
                    let owners = tables.iter().flat_map(|(_, table)| &table.owner_capabilities).filter(|owner| owner.id == *capability).collect::<Vec<_>>();
                    if binding != actual || !matches!(owners.as_slice(), [owner] if owner.role == OwnerRole::Parameter(binding.clone()) && owner.frame == row.frame) { return Err(fail()); }
                },
                (SourceFormalOwnerRole::Capture { formal, binding }, BoundaryPortRole::Capture { binding: actual, .. })
                | (SourceFormalOwnerRole::Global { formal, binding }, BoundaryPortRole::Global(actual)) => {
                    if formal != &row.port || binding != actual { return Err(fail()); }
                },
                _ => return Err(fail()),
            }
            if tables.iter().flat_map(|(_, table)| &table.source_configurations).any(|configuration|
                configuration.instances.contains(&row.value_instance)
                || configuration.created.iter().any(|created| created.instance == row.value_instance)
                || configuration.owns.iter().any(|owns| owns.payload == row.value_instance || owns.owner_instance == row.value_instance)) { return Err(fail()); }
        }
    }
    Ok(())
}
