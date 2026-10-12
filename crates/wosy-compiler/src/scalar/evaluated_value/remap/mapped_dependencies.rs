use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Reference {
    Configuration(SourceConfigurationRef),
    Contributor(JointProducerTupleRef),
    View(SourceValueViewRef),
    Instance(SourceInstanceRef),
    Slot(SourceSlotRef),
    Formal(BoundaryFormalPortRef),
    FormalValue(SourceFormalValueRef),
    FormalValueIncidence(SourceFormalValue),
    FormalReadIncidence(SourceFormalReadWitness),
    NameReadIncidence(SourceNameReadFact),
    Monitor(JointPortMonitorRef),
    Move(SourceMoveWitnessRef),
    Value(EvaluatedValueId),
    Case(ValueCaseId),
    Path(EvaluatedControlPathId),
    Shape(ValueShapeId),
    Node(SourceUseGraphRef, SourceUseNodeRef),
    DirectUse(SourceUseGraphRef, usize),
    Edge(SourceUseGraphRef, usize),
    Reachable(SourceUseGraphRef, usize),
    Coverage(SourceUseGraphRef, usize),
    Equation(ProducerCfgOwnerId, JointPortMonitorEquation),
    Candidate(StagedLoanQueryRef, LocalLoanCandidateRef),
    Epoch(CommonLoanInput),
    PathIncidence(EvaluatedControlPathId, SourceConfigurationRef, JointProducerTupleRef),
    CaseIncidence(ValueCaseId, EvaluatedControlPathId),
    FormalIncidence(BoundaryFormalPortRef, SourceSlotRef, ValueShapeId),
}

fn case_view(row: &SourceCaseViewFact, references: &mut Vec<Reference>) {
    references.extend([Reference::Slot(row.port.clone()), Reference::View(row.view.clone()), Reference::Configuration(row.configuration.clone()), Reference::Contributor(row.contributor.clone()), Reference::Path(row.path.clone()), Reference::Case(row.case.clone())]);
    references.extend([Reference::PathIncidence(row.path.clone(), row.configuration.clone(), row.contributor.clone()), Reference::CaseIncidence(row.case.clone(), row.path.clone())]);
}

fn selected(row: &JointSelectedPortFact, references: &mut Vec<Reference>) {
    references.push(Reference::Slot(row.coordinate.clone()));
    case_view(&row.observation, references);
    for source in &row.sources { case_view(source, references); }
    references.extend(row.moves.iter().cloned().map(Reference::Move));
}

fn equation(row: &JointPortMonitorEquation, references: &mut Vec<Reference>) {
    references.extend([Reference::Contributor(row.contributor.clone()), Reference::Monitor(row.input.clone()), Reference::Monitor(row.output.clone())]);
    references.extend(row.moves.iter().cloned().map(Reference::Move));
    for child in &row.children { references.push(Reference::Monitor(child.monitor.clone())); references.extend(child.ports.iter().cloned().map(Reference::Slot)); }
    for production in &row.productions { case_view(&production.observation, references); references.extend(production.inputs.iter().cloned().map(Reference::Slot)); }
}

fn support(row: &SourceUseSupport, graph: &SourceUseGraphRef, references: &mut Vec<Reference>) {
    references.extend([Reference::Node(graph.clone(), row.node.clone()), Reference::Slot(row.coordinate.clone())]);
    match &row.subject {
        SourceUseSubject::Payload { instance, .. } | SourceUseSubject::Owner { instance, .. } => references.push(Reference::Instance(instance.clone())),
        SourceUseSubject::Grant { referent, .. } => references.push(Reference::Instance(referent.clone())),
    }
}

fn observation(row: &SourceUseObservation, references: &mut Vec<Reference>) {
    references.push(Reference::Node(row.graph.clone(), row.node.clone()));
    references.extend(row.reachable.iter().map(|index| Reference::Reachable(row.graph.clone(), *index)));
    references.extend(row.parent_support.iter().map(|index| Reference::Edge(row.graph.clone(), *index)));
    references.extend(row.coverage.iter().map(|index| Reference::Coverage(row.graph.clone(), *index)));
}

fn use_references(graphs: &[SourceUseGraph]) -> Vec<Reference> {
    let mut references = Vec::new();
    for graph in graphs {
        references.push(Reference::Epoch(graph.epoch.clone()));
        for node in &graph.nodes {
            references.extend([Reference::Node(graph.id.clone(), node.id.clone()), Reference::Contributor(node.id.contributor.clone()), Reference::Monitor(node.id.monitor.clone()), Reference::Configuration(node.configuration.clone())]);
            for row in &node.supports { support(row, &graph.id, &mut references); }
        }
        for usage in &graph.uses {
            references.extend([Reference::Contributor(usage.contributor.clone()), Reference::Configuration(usage.configuration.clone()), Reference::Equation(graph.id.owner.clone(), usage.equation.clone())]);
            equation(&usage.equation, &mut references);
            support(&usage.support, &graph.id, &mut references);
            match &usage.input { SourceUseInput::Selected(row) => selected(row, &mut references), SourceUseInput::Storage(row) => references.extend([Reference::Slot(row.coordinate.clone()), Reference::View(row.view.clone())]) }
        }
        for edge in &graph.edges {
            references.push(Reference::Equation(graph.id.owner.clone(), edge.equation.clone()));
            equation(&edge.equation, &mut references);
            support(&edge.source, &graph.id, &mut references);
            support(&edge.destination, &graph.id, &mut references);
        }
        for reachable in &graph.reachable {
            support(&reachable.support, &graph.id, &mut references);
            references.push(Reference::DirectUse(graph.id.clone(), reachable.direct_use));
            for predecessor in &reachable.predecessors { references.extend([Reference::Edge(graph.id.clone(), predecessor.edge), Reference::Reachable(graph.id.clone(), predecessor.reachable)]); }
        }
        for returned in &graph.returns {
            observation(&returned.input, &mut references);
            references.push(Reference::Node(graph.id.clone(), returned.output.clone()));
            references.extend(returned.remaining.iter().map(|index| Reference::Reachable(graph.id.clone(), *index)));
        }
    }
    references
}

fn staged_references(stages: &[StagedLoanQueryAnalysis]) -> Vec<Reference> {
    let mut references = Vec::new();
    for stage in stages {
        references.push(Reference::Epoch(stage.epoch.clone()));
        if let StagedLoanQueryResult::Candidates { slices, queries, .. } = &stage.result {
            for slice in slices { references.push(Reference::Candidate(stage.id.clone(), slice.candidate.clone())); }
            for query in queries {
                references.extend([Reference::Candidate(stage.id.clone(), query.candidate.clone()), Reference::Configuration(query.input.clone()), Reference::Configuration(query.output.clone()), Reference::Contributor(query.contributor.clone()), Reference::Equation(stage.id.owner.clone(), query.equation.clone())]);
                equation(&query.equation, &mut references);
                observation(&query.remaining_uses, &mut references);
                match &query.access {
                    LocalLoanAccess::DirectStorage { referent, coordinate, .. } => references.extend([Reference::Instance(referent.clone()), Reference::Slot(coordinate.clone())]),
                    LocalLoanAccess::Through(row) => { selected(&row.selected, &mut references); references.extend([Reference::Slot(row.grant.coordinate.clone()), Reference::Instance(row.grant.referent.clone())]); }
                    LocalLoanAccess::Pending => {}
                }
                for pair in &query.pairs { selected(&pair.support.selected, &mut references); references.extend([Reference::Slot(pair.support.grant.coordinate.clone()), Reference::Instance(pair.support.grant.referent.clone()), Reference::Slot(pair.protected.coordinate.clone()), Reference::Instance(pair.protected.referent.clone())]); }
            }
        }
    }
    references
}

fn table_references(table: &EvaluatedValueTables) -> Vec<Reference> {
    let mut references = Vec::new();
    for row in &table.source_formal_values {
        references.extend([Reference::FormalValue(row.id.clone()), Reference::FormalValueIncidence(row.clone()), Reference::Formal(row.port.clone()), Reference::Shape(row.shape.clone()), Reference::Instance(row.value_instance.clone())]);
        for edge in &row.members { references.extend([Reference::Shape(edge.member.shape.clone()), Reference::FormalValue(edge.value.clone())]); }
    }
    for row in &table.source_name_reads { references.extend([Reference::NameReadIncidence(row.clone()), Reference::Value(row.value.clone())]); }
    for row in &table.source_formal_reads {
        references.extend([Reference::FormalReadIncidence(row.clone()), Reference::FormalValue(row.source.clone()), Reference::Value(row.value.clone()), Reference::Configuration(row.input.clone()), Reference::Configuration(row.output.clone()), Reference::Contributor(row.contributor.clone()), Reference::Monitor(row.monitor.clone())]);
        selected(&row.selected, &mut references);
    }
    for row in &table.source_case_views { case_view(row, &mut references); }
    for row in &table.source_paths { references.extend([Reference::Path(row.id.clone()), Reference::Configuration(row.configuration.clone()), Reference::Contributor(row.contributor.clone())]); }
    for row in &table.source_configurations {
        references.push(Reference::Configuration(row.id.clone()));
        references.extend(row.instances.iter().cloned().map(Reference::Instance));
        for slot in &row.slots { references.extend([Reference::Slot(slot.coordinate.clone()), Reference::View(slot.view.clone())]); }
        for input in &row.formal_inputs { references.extend([Reference::Formal(input.port.clone()), Reference::Slot(input.coordinate.clone()), Reference::Shape(input.shape.clone()), Reference::FormalIncidence(input.port.clone(), input.coordinate.clone(), input.shape.clone())]); }
        for saved in &row.saved { references.extend([Reference::Slot(saved.coordinate.clone()), Reference::View(saved.view.clone())]); }
        for grant in &row.grants { references.extend([Reference::Slot(grant.coordinate.clone()), Reference::Instance(grant.referent.clone())]); }
        for owner in &row.owner_views { references.extend([Reference::Slot(owner.coordinate.clone()), Reference::Instance(owner.payload.clone()), Reference::Instance(owner.owner_instance.clone())]); }
        for created in &row.created { references.extend([Reference::Instance(created.instance.clone()), Reference::Shape(created.shape.clone())]); }
        for owns in &row.owns { references.extend([Reference::Instance(owns.owner_instance.clone()), Reference::Instance(owns.payload.clone())]); }
        for depends in &row.depends { references.extend([Reference::Instance(depends.source.clone()), Reference::Instance(depends.target.clone())]); }
        for target in &row.has_target { references.extend([Reference::Instance(target.source.clone()), Reference::Instance(SourceInstanceRef::Residual(target.target.clone()))]); }
        for functional in &row.functional { references.push(Reference::Instance(functional.source.clone())); }
        for relation in row.same_target.iter().chain(&row.different_target) { references.extend([Reference::Instance(relation.left.clone()), Reference::Instance(relation.right.clone())]); }
        for pending in &row.pending { references.extend(pending.outputs.iter().cloned().map(Reference::Value)); }
    }
    for row in &table.source_instances { references.extend([Reference::Instance(SourceInstanceRef::Supported(row.id.clone())), Reference::Slot(row.support.clone()), Reference::Shape(row.shape.clone())]); if let SourceInstanceOrigin::Formal(id) = &row.origin { references.push(Reference::Formal(id.clone())); } }
    for row in &table.source_views {
        references.extend([Reference::View(row.id.clone()), Reference::Shape(row.shape.clone())]);
        match &row.payload {
            SourcePayloadView::Formal(id) => references.push(Reference::FormalValue(id.clone())),
            SourcePayloadView::Aggregate(members) => { for member in members { references.extend([Reference::View(member.view.clone()), Reference::Value(member.source.clone())]); } }
            SourcePayloadView::Checked { referents, .. } => { for referent in referents { references.extend([Reference::Instance(referent.instance.clone()), Reference::Slot(referent.coordinate.clone())]); } }
            SourcePayloadView::Raw { referent, .. } => references.push(Reference::Instance(referent.clone())),
            SourcePayloadView::Backing { instance, .. } => references.push(Reference::Instance(instance.clone())),
            SourcePayloadView::Plain | SourcePayloadView::Unwritten | SourcePayloadView::StaticRaw(_) => {}
        }
    }
    for row in &table.source_contributors {
        references.extend([Reference::Contributor(row.id.clone()), Reference::Configuration(row.input.clone()), Reference::Configuration(row.output.clone())]);
        references.extend(row.preserved.iter().chain(&row.fresh).cloned().map(Reference::Instance));
        references.extend(row.outputs.iter().cloned().map(Reference::Value));
        for child in &row.children { references.push(Reference::Configuration(child.configuration.clone())); references.extend(child.ports.iter().cloned().map(Reference::Slot)); }
        for transport in &row.transports { references.extend([Reference::Slot(transport.source.clone()), Reference::Slot(transport.destination.clone())]); }
        for production in &row.productions { references.push(Reference::Slot(production.destination.clone())); references.extend(production.inputs.iter().cloned().map(Reference::Slot)); }
    }
    for row in &table.source_port_monitors { references.extend([Reference::Monitor(row.id.clone()), Reference::Contributor(row.contributor.clone())]); for port in &row.ports { selected(port, &mut references); } references.extend(row.pending.iter().cloned().map(Reference::Slot)); }
    for row in &table.source_port_equations { equation(row, &mut references); }
    for row in &table.source_move_witnesses { references.extend([Reference::Move(row.id.clone()), Reference::Configuration(row.input.clone()), Reference::Configuration(row.output.clone()), Reference::Contributor(row.contributor.clone()), Reference::Value(row.source.clone())]); selected(&row.selection, &mut references); }
    for row in &table.source_saved_ports { references.push(Reference::Monitor(row.monitor.clone())); selected(&row.selected, &mut references); }
    for row in &table.source_grant_creations { references.extend([Reference::Configuration(row.input.clone()), Reference::Configuration(row.output.clone()), Reference::Contributor(row.contributor.clone())]); if let SourceGrantParent::Selected { source, selection, .. } = &row.parent { references.push(Reference::Value(source.clone())); selected(selection, &mut references); } }
    for row in &table.owner_receiving_transitions { references.extend([Reference::Configuration(row.input.clone()), Reference::Configuration(row.output.clone()), Reference::Instance(row.receiving_instance.clone())]); }
    for row in &table.source_populations { references.extend([Reference::Instance(SourceInstanceRef::Residual(row.id.clone())), Reference::Shape(row.shape.clone())]); }
    for row in &table.source_backing_observations { references.push(Reference::View(row.view.clone())); }
    for row in &table.source_literal_members { references.push(Reference::Value(row.value.clone())); references.extend(row.members.iter().map(|member| Reference::Value(member.value.clone()))); }
    references.extend(use_references(&table.source_use_graphs));
    references.extend(staged_references(&table.staged_loan_queries));
    references
}

impl Reference {
    fn owner(&self) -> &ProducerCfgOwnerId {
        match self {
            Self::FormalValue(id) => &id.owner,
            Self::FormalValueIncidence(row) => &row.id.owner,
            Self::FormalReadIncidence(row) => &row.event.owner,
            Self::NameReadIncidence(row) => &row.event.owner,
            Self::Configuration(id) => &id.owner, Self::Contributor(id) => &id.owner, Self::View(id) => &id.owner,
            Self::Instance(SourceInstanceRef::Supported(id)) => &id.owner, Self::Instance(SourceInstanceRef::Residual(id)) => &id.owner,
            Self::Slot(id) => &id.owner, Self::Formal(id) => &id.owner, Self::Monitor(id) => &id.owner, Self::Move(id) => &id.owner,
            Self::Value(id) => &id.resolved.producer, Self::Case(id) => &id.value.resolved.producer, Self::Path(id) => &id.producer, Self::Shape(id) => &id.producer,
            Self::Node(id, _) | Self::DirectUse(id, _) | Self::Edge(id, _) | Self::Reachable(id, _) | Self::Coverage(id, _) => &id.owner,
            Self::Equation(owner, _) => owner, Self::Candidate(stage, _) => &stage.owner,
            Self::Epoch(epoch) => &epoch.owner,
            Self::PathIncidence(path, _, _) => &path.producer,
            Self::CaseIncidence(case, _) => &case.value.resolved.producer,
            Self::FormalIncidence(port, _, _) => &port.owner,
        }
    }

    fn exists(&self, tables: &EvaluatedValueTables) -> bool {
        match self {
            Self::FormalValue(id) => tables.source_formal_values.iter().filter(|row| row.id == *id).count() == 1,
            Self::FormalValueIncidence(original) => tables.source_formal_values.iter().filter(|row| *row == original).count() == 1,
            Self::FormalReadIncidence(original) => tables.source_formal_reads.iter().filter(|row| *row == original).count() == 1,
            Self::NameReadIncidence(original) => tables.source_name_reads.iter().filter(|row| *row == original).count() == 1,
            Self::Configuration(id) => tables.source_configurations.iter().filter(|row| row.id == *id).count() == 1,
            Self::Contributor(id) => tables.source_contributors.iter().filter(|row| row.id == *id).count() == 1,
            Self::View(id) => tables.source_views.iter().filter(|row| row.id == *id).count() == 1,
            Self::Instance(SourceInstanceRef::Supported(id)) => tables.source_instances.iter().filter(|row| row.id == *id).count() == 1,
            Self::Instance(SourceInstanceRef::Residual(id)) => tables.source_populations.iter().filter(|row| row.id == *id).count() == 1,
            Self::Slot(id) => tables.source_slots.iter().filter(|row| row.id == *id).count() == 1,
            Self::Formal(id) => tables.boundary_signatures.iter().filter(|row| row.owner == id.owner && row.ports.get(id.port).is_some()).count() == 1,
            Self::Monitor(id) => tables.source_port_monitors.iter().filter(|row| row.id == *id).count() == 1,
            Self::Move(id) => tables.source_move_witnesses.iter().filter(|row| row.id == *id).count() == 1,
            Self::Value(id) => tables.values.iter().filter(|row| row.id == *id).count() == 1,
            Self::Case(id) => tables.values.iter().flat_map(|row| &row.cases).filter(|row| row.id == *id).count() == 1,
            Self::Path(id) => tables.source_paths.iter().filter(|row| row.id == *id).count() == 1,
            Self::Shape(id) => tables.shapes.iter().filter(|row| row.id == *id).count() == 1,
            Self::Equation(_, equation) => tables.source_port_equations.iter().filter(|row| *row == equation).count() == 1,
            Self::Node(id, node) => tables.source_use_graphs.iter().filter(|graph| graph.id == *id && graph.nodes.iter().filter(|row| row.id == *node).count() == 1).count() == 1,
            Self::DirectUse(id, index) => tables.source_use_graphs.iter().filter(|graph| graph.id == *id && *index < graph.uses.len()).count() == 1,
            Self::Edge(id, index) => tables.source_use_graphs.iter().filter(|graph| graph.id == *id && *index < graph.edges.len()).count() == 1,
            Self::Reachable(id, index) => tables.source_use_graphs.iter().filter(|graph| graph.id == *id && *index < graph.reachable.len()).count() == 1,
            Self::Coverage(id, index) => tables.source_use_graphs.iter().filter(|graph| graph.id == *id && *index < graph.coverage.len()).count() == 1,
            Self::Candidate(id, candidate) => tables.staged_loan_queries.iter().filter(|row| row.id == *id && matches!(&row.result, StagedLoanQueryResult::Candidates { slices, .. } if slices.iter().filter(|slice| slice.candidate == *candidate).count() == 1)).count() == 1,
            Self::Epoch(epoch) => tables.common_loans.iter().filter(|row| row.input == *epoch).count() == 1,
            Self::PathIncidence(id, configuration, contributor) => tables.source_paths.iter().filter(|row| row.id == *id && row.configuration == *configuration && row.contributor == *contributor).count() == 1,
            Self::CaseIncidence(id, path) => tables.values.iter().flat_map(|row| &row.cases).filter(|row| row.id == *id && row.path == *path).count() == 1,
            Self::FormalIncidence(port, coordinate, shape) => tables.boundary_signatures.iter().filter(|row| row.owner == port.owner && row.ports.get(port.port).is_some_and(|row| row.coordinate == *coordinate && row.shape == *shape)).count() == 1,
        }
    }

    fn substitute(&self, remap: &ValueSchemaRemap, source: &ValueTableSource, correspondence: &BoundaryArenaCorrespondence) -> Result<Self, ValueSchemaRemapError> {
        Ok(match self {
            Self::FormalValue(id) => Self::FormalValue(remap.source_formal_value(id)?),
            Self::NameReadIncidence(row) => Self::NameReadIncidence(remap.name_read(row)?),
            Self::FormalValueIncidence(row) => Self::FormalValueIncidence(remap.source_formal_row(row, correspondence)?),
            Self::FormalReadIncidence(original) => {
                let indices = source.tables.source_formal_reads.iter().enumerate().filter(|(_, row)| *row == original).collect::<Vec<_>>();
                let [(index, _)] = indices.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(original.event.owner.clone())); };
                Self::FormalReadIncidence(remap.source_formal_reads(&source.tables, correspondence)?[*index].clone())
            },
            Self::Configuration(id) => Self::Configuration(remap.source_configuration(id)?), Self::Contributor(id) => Self::Contributor(remap.source_tuple(id)?),
            Self::View(id) => Self::View(remap.source_view(id)?), Self::Instance(id) => Self::Instance(remap.source_instance(id)?),
            Self::Slot(id) => Self::Slot(correspondence.slot(id)?), Self::Formal(id) => Self::Formal(correspondence.formal(id)?),
            Self::Monitor(id) => Self::Monitor(remap.source_monitor(id)?), Self::Move(id) => Self::Move(SourceMoveWitnessRef { owner: remap.producer(&id.owner)?, index: id.index }),
            Self::Value(id) => Self::Value(remap.value(id)?), Self::Case(id) => Self::Case(remap.case(id)?), Self::Path(id) => Self::Path(remap.path(id)?), Self::Shape(id) => Self::Shape(remap.shape(id)?),
            Self::Node(graph, node) => Self::Node(SourceUseGraphRef { owner: remap.producer(&graph.owner)? }, SourceUseNodeRef { contributor: remap.source_tuple(&node.contributor)?, monitor: remap.source_monitor(&node.monitor)? }),
            Self::DirectUse(graph, index) => Self::DirectUse(SourceUseGraphRef { owner: remap.producer(&graph.owner)? }, *index),
            Self::Edge(graph, index) => Self::Edge(SourceUseGraphRef { owner: remap.producer(&graph.owner)? }, *index),
            Self::Reachable(graph, index) => Self::Reachable(SourceUseGraphRef { owner: remap.producer(&graph.owner)? }, *index),
            Self::Coverage(graph, index) => Self::Coverage(SourceUseGraphRef { owner: remap.producer(&graph.owner)? }, *index),
            Self::Equation(owner, original) => {
                let indices = source.tables.source_port_equations.iter().enumerate().filter(|(_, row)| *row == original).collect::<Vec<_>>();
                let [(index, _)] = indices.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
                Self::Equation(remap.producer(owner)?, remap.source_port_equations(&source.tables, correspondence)?[*index].clone())
            }
            Self::Candidate(stage, candidate) => Self::Candidate(StagedLoanQueryRef { owner: remap.producer(&stage.owner)? }, LocalLoanCandidateRef { owner: remap.producer(&candidate.owner)?, ordinal: candidate.ordinal }),
            Self::Epoch(epoch) => {
                let indices = source.tables.common_loans.iter().enumerate().filter(|(_, row)| row.input == *epoch).collect::<Vec<_>>();
                let [(index, _)] = indices.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(epoch.owner.clone())); };
                Self::Epoch(remap.common_loans(&source.tables)?[*index].input.clone())
            }
            Self::PathIncidence(path, configuration, contributor) => Self::PathIncidence(remap.path(path)?, remap.source_configuration(configuration)?, remap.source_tuple(contributor)?),
            Self::CaseIncidence(case, path) => Self::CaseIncidence(remap.case(case)?, remap.path(path)?),
            Self::FormalIncidence(port, coordinate, shape) => Self::FormalIncidence(correspondence.formal(port)?, correspondence.slot(coordinate)?, remap.shape(shape)?),
        })
    }
}

fn validate_edges(root: &ProducerCfgOwnerId, remap: &ValueSchemaRemap, original: &[Reference], mapped: &[Reference], source: &[ValueTableSource], correspondence: &BoundaryArenaCorrespondence) -> Result<(), ValueSchemaRemapError> {
    if original.len() != mapped.len() { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); }
    for (before, after) in original.iter().zip(mapped) {
        let carriers = source.iter().filter(|row| &row.producer == before.owner()).collect::<Vec<_>>();
        let [carrier] = carriers.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(before.owner().clone())); };
        if !before.exists(&carrier.tables) || before.substitute(remap, carrier, correspondence)? != *after { return Err(ValueSchemaRemapError::MissingProducer(before.owner().clone())); }
    }
    Ok(())
}

pub(super) fn validate_full(remap: &ValueSchemaRemap, source: &[ValueTableSource], mapped: &[RemappedValueTable], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    formal_graph::validate_tables(&source.iter().map(|row| (&row.producer, &row.tables)).collect::<Vec<_>>())?;
    formal_graph::validate_tables(&mapped.iter().map(|row| (&row.destination, &row.tables)).collect::<Vec<_>>())?;
    for original in source {
        let counterparts = mapped.iter().filter(|row| row.source == original.producer).collect::<Vec<_>>();
        let [counterpart] = counterparts.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); };
        let before = table_references(&original.tables);
        let after = table_references(&counterpart.tables);
        validate_edges(&original.producer, remap, &before, &after, source, correspondence)?;
        for (original, actual) in before.iter().zip(&after) {
            let targets = mapped.iter().filter(|row| &row.source == original.owner()).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(original.owner().clone())); };
            if !actual.exists(&target.tables) { return Err(ValueSchemaRemapError::MissingProducer(original.owner().clone())); }
        }
    }
    resources::postvalidation::full(remap, source, mapped, correspondence, resources)
}

fn validate_selective_headers(original: &[Reference], mapped: &[Reference], destination: &[BoundaryTableHeaders]) -> Result<(), ValueSchemaRemapError> {
    for (before, after) in original.iter().zip(mapped) {
        if matches!(after, Reference::Slot(..) | Reference::Formal(..) | Reference::Shape(..) | Reference::FormalIncidence(..)) {
            let carriers = destination.iter().filter(|row| &row.producer == after.owner()).collect::<Vec<_>>();
            let [carrier] = carriers.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(before.owner().clone())); };
            if !after.exists(&carrier.tables) { return Err(ValueSchemaRemapError::MissingProducer(before.owner().clone())); }
        }
    }
    Ok(())
}

pub(super) fn validate_uses(root: &ProducerCfgOwnerId, remap: &ValueSchemaRemap, original: &[SourceUseGraph], mapped: &[SourceUseGraph], source: &[ValueTableSource], destination: &[BoundaryTableHeaders], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    if original.len() != mapped.len() { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); }
    let before = use_references(original);
    let after = use_references(mapped);
    validate_edges(root, remap, &before, &after, source, correspondence)?;
    validate_selective_headers(&before, &after, destination)?;
    let mut topology = EvaluatedValueTables::new();
    topology.source_use_graphs = mapped.to_vec();
    for reference in after {
        if matches!(reference, Reference::Node(..) | Reference::DirectUse(..) | Reference::Edge(..) | Reference::Reachable(..) | Reference::Coverage(..)) && !reference.exists(&topology) { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); }
    }
    let carriers = source.iter().filter(|row| row.producer == *root).collect::<Vec<_>>();
    let [carrier] = carriers.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); };
    topology.common_loans = remap.common_loans(&carrier.tables)?;
    topology.owner_receiving_transitions = remap.owner_receiving_transitions(&carrier.tables)?;
    topology.source_origin_derivations = carrier.tables.source_origin_derivations.iter().map(|row| remap.origin_derivation(row)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
    resources::postvalidation::selective(remap, root, source, &topology, destination, correspondence, &resources::BoundaryResourceDemandCut::SourceUses { root: root.clone() }, resources)
}

pub(super) fn validate_stages(root: &ProducerCfgOwnerId, remap: &ValueSchemaRemap, original: &[StagedLoanQueryAnalysis], mapped: &[StagedLoanQueryAnalysis], source: &[ValueTableSource], destination: &[BoundaryTableHeaders], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    if original.len() != mapped.len() { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); }
    let before = staged_references(original);
    let after = staged_references(mapped);
    validate_edges(root, remap, &before, &after, source, correspondence)?;
    validate_selective_headers(&before, &after, destination)?;
    let mut topology = EvaluatedValueTables::new();
    topology.staged_loan_queries = mapped.to_vec();
    for reference in after { if matches!(reference, Reference::Candidate(..)) && !reference.exists(&topology) { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); } }
    let carriers = source.iter().filter(|row| row.producer == *root).collect::<Vec<_>>();
    let [carrier] = carriers.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); };
    topology.common_loans = remap.common_loans(&carrier.tables)?;
    topology.source_origin_derivations = carrier.tables.source_origin_derivations.iter().map(|row| remap.origin_derivation(row)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
    resources::postvalidation::selective(remap, root, source, &topology, destination, correspondence, &resources::BoundaryResourceDemandCut::StagedLoans { root: root.clone() }, resources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_tagged_demands_preserve_independent_destination_aliases() {
        let left = correction_tests::source("r1");
        let right = correction_tests::source("r2");
        let destination = correction_tests::source("r3");
        let sources = [left, right];
        let mapping = ValueSchemaRemap {
            producers: sources.iter().map(|source| (source.producer.clone(), destination.producer.clone())).collect(), points: Vec::new(),
            shapes: sources.iter().flat_map(|source| source.tables.shapes.iter().zip(&destination.tables.shapes).map(|(a, b)| (a.id.clone(), b.id.clone()))).collect(), imports: Vec::new(),
        };
        let destinations = [destination];
        let mut forest = capture_source_tree_construction_forest(&sources[0].producer, &sources[..1]).expect("left actual constructor forest");
        let right_forest = capture_source_tree_construction_forest(&sources[1].producer, &sources[1..]).expect("right actual constructor forest");
        forest.roots.extend(right_forest.roots);
        forest.sources.extend(right_forest.sources);
        let destination_forest = capture_source_tree_construction_forest(&destinations[0].producer, &destinations).expect("actual independent constructor forest");
        let witness = mapping.independent_boundary_destination(&destinations, &destination_forest).expect("actual independent headers");
        let checked = witness.checked(&mapping, &sources, &forest, &resources::demand::resource_demand_full(&sources)).expect("explicit independently aliased shape closures");
        let headers = &checked.headers;
        let correspondence = checked.correspondence;
        let original = sources.iter().map(|source| Reference::Shape(source.tables.shapes.iter().find(|row| row.ty == ScalarType::I32).expect("actual field type").id.clone())).collect::<Vec<_>>();
        let target = destinations[0].tables.shapes.iter().find(|row| row.ty == ScalarType::I32).expect("actual destination field type").id.clone();
        let mapped = [Reference::Shape(target.clone()), Reference::Shape(target)];
        validate_edges(&sources[0].producer, &mapping, &original, &mapped, &sources, &correspondence).expect("each original source demand retains its own association");
        validate_selective_headers(&original, &mapped, headers).expect("one actual destination header serves both independent demands");
    }

    #[test]
    fn crossed_mapped_edge_rejects_existing_wrong_counterpart() {
        let original = correction_tests::source("r1");
        let destination = correction_tests::source("r2");
        let mapping = ValueSchemaRemap {
            producers: vec![(original.producer.clone(), destination.producer.clone())], points: Vec::new(),
            shapes: original.tables.shapes.iter().zip(&destination.tables.shapes).map(|(a, b)| (a.id.clone(), b.id.clone())).collect(), imports: Vec::new(),
        };
        let root = original.producer.clone();
        let before = Reference::Shape(original.tables.shapes.iter().find(|row| row.ty == ScalarType::I32).expect("actual field type").id.clone());
        let wrong = Reference::Shape(destination.tables.shapes.iter().find(|row| row.ty == ScalarType::U8).expect("actual different field type").id.clone());
        let sources = [original];
        let destinations = [destination];
        let forest = capture_source_tree_construction_forest(&root, &sources).expect("actual original constructor forest");
        let destination_forest = capture_source_tree_construction_forest(&destinations[0].producer, &destinations).expect("actual independent constructor forest");
        let witness = mapping.independent_boundary_destination(&destinations, &destination_forest).expect("actual independent headers");
        let correspondence = witness.checked(&mapping, &sources, &forest, &resources::demand::resource_demand_full(&sources)).expect("actual qualified shape closures").correspondence;
        assert_eq!(validate_edges(&root, &mapping, &[before], &[wrong], &sources, &correspondence), Err(ValueSchemaRemapError::MissingProducer(root)));
    }
}
