use super::*;

pub(super) fn validate_tables(tables: &[(&ProducerCfgOwnerId, &EvaluatedValueTables)]) -> Result<(), ValueSchemaRemapError> {
    formal_graph::validate_tables(tables)?;
    let configuration = |id: &SourceConfigurationRef| tables.iter().flat_map(|(_, table)| &table.source_configurations).filter(|row| row.id == *id).count() == 1;
    let contributor = |id: &JointProducerTupleRef| tables.iter().flat_map(|(_, table)| &table.source_contributors).filter(|row| row.id == *id).count() == 1;
    let view = |id: &SourceValueViewRef| tables.iter().flat_map(|(_, table)| &table.source_views).filter(|row| row.id == *id).count() == 1;
    let instance = |id: &SourceInstanceRef| match id {
        SourceInstanceRef::Supported(id) => tables.iter().flat_map(|(_, table)| &table.source_instances).filter(|row| row.id == *id).count() == 1,
        SourceInstanceRef::Residual(id) => tables.iter().flat_map(|(_, table)| &table.source_populations).filter(|row| row.id == *id).count() == 1,
    };
    let slot = |id: &SourceSlotRef| tables.iter().flat_map(|(_, table)| &table.source_slots).filter(|row| row.id == *id).count() == 1;
    let formal = |id: &BoundaryFormalPortRef| tables.iter().flat_map(|(_, table)| &table.boundary_signatures).filter(|row| row.owner == id.owner && row.ports.get(id.port).is_some()).count() == 1;
    let monitor = |id: &JointPortMonitorRef| tables.iter().flat_map(|(_, table)| &table.source_port_monitors).filter(|row| row.id == *id).count() == 1;
    let moved = |id: &SourceMoveWitnessRef| tables.iter().flat_map(|(_, table)| &table.source_move_witnesses).filter(|row| row.id == *id).count() == 1;
    let value = |id: &EvaluatedValueId| tables.iter().flat_map(|(_, table)| &table.values).filter(|row| row.id == *id).count() == 1;
    let case_view = |row: &SourceCaseViewFact| {
        slot(&row.port) && view(&row.view) && configuration(&row.configuration) && contributor(&row.contributor)
            && tables.iter().flat_map(|(_, table)| &table.source_paths).filter(|path| path.id == row.path && path.configuration == row.configuration && path.contributor == row.contributor).count() == 1
            && tables.iter().flat_map(|(_, table)| &table.values).flat_map(|value| &value.cases).filter(|case| case.id == row.case && case.path == row.path).count() == 1
    };
    let selected = |row: &JointSelectedPortFact| slot(&row.coordinate) && case_view(&row.observation) && row.sources.iter().all(&case_view) && row.moves.iter().all(&moved);
    for (owner, table) in tables {
        for row in &table.source_name_reads {
            if row.event.owner != **owner || row.value.resolved.producer != **owner || row.value.point != row.event.point
                || row.value.resolved.output != 0 || !value(&row.value)
                || table.source_name_reads.iter().filter(|other| other.event == row.event || other.value == row.value).count() != 1 {
                return Err(ValueSchemaRemapError::MissingPoint { producer: row.event.owner.clone(), point: row.event.point });
            }
        }
        for value in &table.values {
            for case in &value.cases {
                if let EvaluatedPayloadFact::Formal { port, view } = &case.payload {
                    let rows = tables.iter().flat_map(|(_, table)| &table.source_formal_values).filter(|row| row.id == *view).collect::<Vec<_>>();
                    if !matches!(rows.as_slice(), [row] if row.port == *port && row.shape == value.shape) || case.id.value != value.id {
                        return Err(ValueSchemaRemapError::MissingBoundaryFormal(port.clone()));
                    }
                }
            }
        }
        for row in &table.source_formal_reads {
            let sources = tables.iter().flat_map(|(_, table)| &table.source_formal_values).filter(|source| source.id == row.source).collect::<Vec<_>>();
            let rules = tables.iter().flat_map(|(_, table)| &table.source_contributors).filter(|rule| rule.id == row.contributor).collect::<Vec<_>>();
            if !matches!((sources.as_slice(), rules.as_slice()), ([source], [rule]) if source.id.owner == row.event.owner && rule.event == row.event && rule.input == row.input && rule.output == row.output && rule.outputs.contains(&row.value))
                || !configuration(&row.input) || !configuration(&row.output) || !monitor(&row.monitor) || !selected(&row.selected)
                || row.selected.observation.case.value != row.value || row.selected.observation.configuration != row.output
                || row.selected.observation.contributor != row.contributor
                || !tables.iter().flat_map(|(_, table)| &table.source_views).any(|view| view.id == row.selected.observation.view && view.payload == SourcePayloadView::Formal(row.source.clone())) {
                return Err(ValueSchemaRemapError::MissingProducer(row.event.owner.clone()));
            }
        }
        for row in &table.source_case_views {
            if !case_view(row) { return Err(ValueSchemaRemapError::MissingBoundarySlot(row.port.clone())); }
        }
        for row in &table.source_paths {
            if !configuration(&row.configuration) || !contributor(&row.contributor) || table.source_paths.iter().filter(|other| other.id == row.id).count() != 1 {
                return Err(ValueSchemaRemapError::MissingProducer(row.id.producer.clone()));
            }
        }
        for row in &table.source_configurations {
            if !configuration(&row.id) || row.instances.iter().any(|id| !instance(id)) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); }
            for selected in &row.slots { if !slot(&selected.coordinate) || !view(&selected.view) { return Err(ValueSchemaRemapError::MissingBoundarySlot(selected.coordinate.clone())); } }
            for input in &row.formal_inputs {
                if !formal(&input.port) || !slot(&input.coordinate) { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.port.clone())); }
                let signatures = tables.iter().flat_map(|(_, table)| &table.boundary_signatures).filter(|row| row.owner == input.port.owner).collect::<Vec<_>>();
                let [signature] = signatures.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySignature(input.port.owner.clone())); };
                let port = &signature.ports[input.port.port];
                if port.coordinate != input.coordinate || port.shape != input.shape { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.port.clone())); }
            }
            for saved in &row.saved { if !slot(&saved.coordinate) || !view(&saved.view) { return Err(ValueSchemaRemapError::MissingBoundarySlot(saved.coordinate.clone())); } }
            for grant in &row.grants { if !slot(&grant.coordinate) || !instance(&grant.referent) || row.grants.iter().filter(|other| other.class == grant.class).count() != 1 { return Err(ValueSchemaRemapError::MissingBoundarySlot(grant.coordinate.clone())); } }
            for owner_view in &row.owner_views { if !slot(&owner_view.coordinate) || !instance(&owner_view.payload) || !instance(&owner_view.owner_instance) { return Err(ValueSchemaRemapError::MissingBoundarySlot(owner_view.coordinate.clone())); } }
            for creation in &row.created { if !instance(&creation.instance) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for owns in &row.owns { if !instance(&owns.owner_instance) || !instance(&owns.payload) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for depends in &row.depends { if !instance(&depends.source) || !instance(&depends.target) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for target in &row.has_target { if !instance(&target.source) || !instance(&SourceInstanceRef::Residual(target.target.clone())) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for functional in &row.functional { if !instance(&functional.source) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for relation in row.same_target.iter().chain(&row.different_target) { if !instance(&relation.left) || !instance(&relation.right) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for pending in &row.pending { if pending.outputs.iter().any(|id| !value(id)) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
        }
        for row in &table.source_instances {
            if !instance(&SourceInstanceRef::Supported(row.id.clone())) || !slot(&row.support) { return Err(ValueSchemaRemapError::MissingBoundarySlot(row.support.clone())); }
            if let SourceInstanceOrigin::Formal(id) = &row.origin { if !formal(id) { return Err(ValueSchemaRemapError::MissingBoundaryFormal(id.clone())); } }
        }
        for row in &table.source_views {
            if !view(&row.id) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); }
            let valid = match &row.payload {
                SourcePayloadView::Formal(id) => tables.iter().flat_map(|(_, table)| &table.source_formal_values).filter(|row| row.id == *id).count() == 1,
                SourcePayloadView::Aggregate(members) => members.iter().all(|member| view(&member.view) && value(&member.source)),
                SourcePayloadView::Checked { referents, .. } => referents.iter().all(|referent| instance(&referent.instance) && slot(&referent.coordinate)),
                SourcePayloadView::Raw { referent, .. } => instance(referent),
                SourcePayloadView::Backing { instance: id, .. } => instance(id),
                SourcePayloadView::Plain | SourcePayloadView::Unwritten | SourcePayloadView::StaticRaw(_) => true,
            };
            if !valid { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); }
        }
        for row in &table.source_contributors {
            if !contributor(&row.id) || !configuration(&row.input) || !configuration(&row.output) || row.preserved.iter().chain(&row.fresh).any(|id| !instance(id)) || row.outputs.iter().any(|id| !value(id)) {
                return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone()));
            }
            for child in &row.children { if !configuration(&child.configuration) || child.ports.iter().any(|id| !slot(id)) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
            for transport in &row.transports { if !slot(&transport.source) || !slot(&transport.destination) { return Err(ValueSchemaRemapError::MissingBoundarySlot(transport.source.clone())); } }
            for production in &row.productions { if !slot(&production.destination) || production.inputs.iter().any(|id| !slot(id)) { return Err(ValueSchemaRemapError::MissingBoundarySlot(production.destination.clone())); } }
        }
        for row in &table.source_port_monitors { if !monitor(&row.id) || !contributor(&row.contributor) || row.ports.iter().any(|port| !selected(port)) || row.pending.iter().any(|id| !slot(id)) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
        for row in &table.source_port_equations {
            if !contributor(&row.contributor) || !monitor(&row.input) || !monitor(&row.output) || row.moves.iter().any(|id| !moved(id)) { return Err(ValueSchemaRemapError::MissingProducer(row.contributor.owner.clone())); }
            for child in &row.children { if !monitor(&child.monitor) || child.ports.iter().any(|id| !slot(id)) { return Err(ValueSchemaRemapError::MissingProducer(child.monitor.owner.clone())); } }
            for production in &row.productions { if !case_view(&production.observation) || production.inputs.iter().any(|id| !slot(id)) { return Err(ValueSchemaRemapError::MissingBoundarySlot(production.observation.port.clone())); } }
        }
        for row in &table.source_move_witnesses { if !moved(&row.id) || !configuration(&row.input) || !configuration(&row.output) || !contributor(&row.contributor) || !value(&row.source) || !selected(&row.selection) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
        for row in &table.source_saved_ports { if !monitor(&row.monitor) || !selected(&row.selected) { return Err(ValueSchemaRemapError::MissingBoundarySlot(row.selected.coordinate.clone())); } }
        for row in &table.source_grant_creations {
            if !configuration(&row.input) || !configuration(&row.output) || !contributor(&row.contributor) { return Err(ValueSchemaRemapError::MissingProducer(row.event.owner.clone())); }
            if let SourceGrantParent::Selected { source, selection, .. } = &row.parent { if !value(source) || !selected(selection) { return Err(ValueSchemaRemapError::MissingBoundarySlot(selection.coordinate.clone())); } }
        }
        for row in &table.owner_receiving_transitions { if !configuration(&row.input) || !configuration(&row.output) || !instance(&row.receiving_instance) { return Err(ValueSchemaRemapError::MissingProducer(row.event.owner.clone())); } }
        for row in &table.source_populations { if !instance(&SourceInstanceRef::Residual(row.id.clone())) { return Err(ValueSchemaRemapError::MissingProducer(row.id.owner.clone())); } }
        for row in &table.source_backing_observations { if !view(&row.view) { return Err(ValueSchemaRemapError::MissingProducer(row.view.owner.clone())); } }
        for row in &table.source_literal_members { if !value(&row.value) || row.members.iter().any(|member| !value(&member.value)) { return Err(ValueSchemaRemapError::MissingProducer((*owner).clone())); } }
        for graph in &table.source_use_graphs {
            if table.source_use_graphs.iter().filter(|row| row.id == graph.id).count() != 1 || table.common_loans.iter().filter(|row| row.input == graph.epoch).count() != 1 {
                return Err(ValueSchemaRemapError::MissingProducer(graph.id.owner.clone()));
            }
            let support = |support: &SourceUseSupport| {
                graph.nodes.iter().filter(|node| node.id == support.node).count() == 1 && slot(&support.coordinate) && match &support.subject {
                    SourceUseSubject::Payload { instance: id, .. } | SourceUseSubject::Owner { instance: id, .. } => instance(id),
                    SourceUseSubject::Grant { referent, .. } => instance(referent),
                }
            };
            for node in &graph.nodes {
                if !contributor(&node.id.contributor) || !monitor(&node.id.monitor) || !configuration(&node.configuration) || node.supports.iter().any(|row| row.node != node.id || !support(row)) {
                    return Err(ValueSchemaRemapError::MissingProducer(graph.id.owner.clone()));
                }
            }
            for usage in &graph.uses {
                if !contributor(&usage.contributor) || !configuration(&usage.configuration) || !support(&usage.support) || table.source_port_equations.iter().filter(|row| **row == usage.equation).count() != 1 {
                    return Err(ValueSchemaRemapError::MissingProducer(graph.id.owner.clone()));
                }
                let valid = match &usage.input { SourceUseInput::Selected(port) => selected(port), SourceUseInput::Storage(storage) => slot(&storage.coordinate) && view(&storage.view) };
                if !valid { return Err(ValueSchemaRemapError::MissingBoundarySlot(usage.support.coordinate.clone())); }
            }
            for edge in &graph.edges {
                if !support(&edge.source) || !support(&edge.destination) || table.source_port_equations.iter().filter(|row| **row == edge.equation).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(graph.id.owner.clone())); }
            }
            for reachable in &graph.reachable {
                if !support(&reachable.support) || reachable.direct_use >= graph.uses.len() || reachable.predecessors.iter().any(|predecessor| predecessor.edge >= graph.edges.len() || predecessor.reachable >= graph.reachable.len()) { return Err(ValueSchemaRemapError::MissingProducer(graph.id.owner.clone())); }
            }
            for returned in &graph.returns {
                if returned.input.graph != graph.id || graph.nodes.iter().filter(|node| node.id == returned.input.node).count() != 1 || graph.nodes.iter().filter(|node| node.id == returned.output).count() != 1
                    || returned.input.reachable.iter().chain(&returned.remaining).any(|index| *index >= graph.reachable.len()) || returned.input.parent_support.iter().any(|index| *index >= graph.edges.len()) || returned.input.coverage.iter().any(|index| *index >= graph.coverage.len()) {
                    return Err(ValueSchemaRemapError::MissingProducer(graph.id.owner.clone()));
                }
            }
        }
        for stage in &table.staged_loan_queries {
            if table.staged_loan_queries.iter().filter(|row| row.id == stage.id).count() != 1 || table.common_loans.iter().filter(|row| row.input == stage.epoch).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(stage.id.owner.clone())); }
            if let StagedLoanQueryResult::Candidates { slices, queries, .. } = &stage.result {
                for query in queries {
                    if slices.iter().filter(|slice| slice.candidate == query.candidate).count() != 1 || !configuration(&query.input) || !configuration(&query.output) || !contributor(&query.contributor)
                        || query.contributor != query.equation.contributor || table.source_port_equations.iter().filter(|row| **row == query.equation).count() != 1 {
                        return Err(ValueSchemaRemapError::MissingProducer(stage.id.owner.clone()));
                    }
                    for pair in &query.pairs { if !selected(&pair.support.selected) || !slot(&pair.support.grant.coordinate) || !slot(&pair.protected.coordinate) { return Err(ValueSchemaRemapError::MissingBoundarySlot(pair.support.grant.coordinate.clone())); } }
                }
            }
        }
    }
    Ok(())
}
