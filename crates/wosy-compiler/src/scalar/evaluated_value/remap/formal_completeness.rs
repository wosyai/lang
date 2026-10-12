use super::*;
use source_boundary::formal_value::SourceFormalVertexRole;

pub(super) fn validate_tables(tables: &[(&ProducerCfgOwnerId, &EvaluatedValueTables)]) -> Result<(), ValueSchemaRemapError> {
    for (owner, table) in tables {
        let mut expected = Vec::new();
        for signature in &table.boundary_signatures {
            for (ordinal, input) in signature.ports.iter().enumerate() {
                if matches!(input.role, BoundaryPortRole::Output(_) | BoundaryPortRole::Effect { .. }) { continue; }
                let fail = || ValueSchemaRemapError::MissingBoundaryFormal(BoundaryFormalPortRef { owner: signature.owner.clone(), port: ordinal });
                let schemas = tables.iter().flat_map(|(_, table)| &table.shapes).filter(|schema| schema.id == input.shape).collect::<Vec<_>>();
                let [schema] = schemas.as_slice() else { return Err(fail()); };
                if matches!(schema.payload, ValuePayloadShape::Plain(_)) { continue; }
                let port = BoundaryFormalPortRef { owner: signature.owner.clone(), port: ordinal };
                expected.push((port.clone(), SourceFormalVertexRole::Root, input.shape.clone()));
                let mut pending = vec![input.shape.clone()];
                let mut visited = Vec::new();
                while let Some(id) = pending.pop() {
                    if visited.contains(&id) { continue; }
                    visited.push(id.clone());
                    let schemas = tables.iter().flat_map(|(_, table)| &table.shapes).filter(|schema| schema.id == id).collect::<Vec<_>>();
                    let [schema] = schemas.as_slice() else { return Err(fail()); };
                    if id.producer != signature.owner { return Err(fail()); }
                    for (member, child) in source_boundary::formal_value::shape_members(schema).into_iter().enumerate() {
                        expected.push((port.clone(), SourceFormalVertexRole::Member(SourceShapeMemberPort { shape: id.clone(), member }), child.clone()));
                        pending.push(child);
                    }
                }
            }
        }
        if expected.len() != table.source_formal_values.len() { return Err(ValueSchemaRemapError::MissingProducer((*owner).clone())); }
        for (port, role, shape) in &expected {
            if table.source_formal_values.iter().filter(|row| row.port == *port && row.role == *role && row.shape == *shape).count() != 1 {
                return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone()));
            }
        }
        for value in &table.values {
            for case in &value.cases {
                let EvaluatedPayloadFact::Formal { port, view } = &case.payload else { continue; };
                let observations = table.source_case_views.iter().filter(|observation| observation.case == case.id).collect::<Vec<_>>();
                if observations.is_empty() { return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone())); }
                for observation in observations {
                    if !table.source_views.iter().any(|source| source.id == observation.view && source.payload == SourcePayloadView::Formal(view.clone()))
                        || !table.source_port_equations.iter().flat_map(|equation| &equation.productions).any(|production| production.observation == *observation) {
                        return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone()));
                    }
                }
            }
        }
        for equation in &table.source_port_equations {
            for production in &equation.productions {
                let cases = tables.iter().flat_map(|(_, table)| &table.values).flat_map(|value| &value.cases).filter(|case| case.id == production.observation.case).collect::<Vec<_>>();
                let [case] = cases.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer((*owner).clone())); };
                let EvaluatedPayloadFact::Formal { port, view } = &case.payload else { continue; };
                let rules = table.source_contributors.iter().filter(|rule| rule.id == equation.contributor).collect::<Vec<_>>();
                let [rule] = rules.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone())); };
                let witnesses = table.source_formal_reads.iter().filter(|witness|
                    witness.event == rule.event && witness.source == *view && witness.value == case.id.value
                    && witness.input == rule.input && witness.output == rule.output && witness.contributor == rule.id
                    && witness.monitor == equation.output && witness.selected.coordinate == production.observation.port
                    && witness.selected.observation == production.observation).collect::<Vec<_>>();
                let [witness] = witnesses.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone())); };
                let monitors = table.source_port_monitors.iter().filter(|monitor| monitor.id == equation.output).collect::<Vec<_>>();
                if !matches!(monitors.as_slice(), [monitor] if monitor.contributor == rule.id && monitor.ports.contains(&witness.selected)) {
                    return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone()));
                }
            }
        }
        for witness in &table.source_formal_reads {
            let productions = table.source_port_equations.iter().filter(|equation| equation.output == witness.monitor && equation.contributor == witness.contributor)
                .flat_map(|equation| &equation.productions).filter(|production| production.observation == witness.selected.observation).collect::<Vec<_>>();
            if productions.len() != 1 {
                return Err(ValueSchemaRemapError::MissingProducer(witness.event.owner.clone()));
            }
        }
    }
    Ok(())
}
