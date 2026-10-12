use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RemappedStagedLoanQueries {
    pub(super) queries: Vec<StagedLoanQueryAnalysis>,
    pub(super) correspondence: BoundaryArenaCorrespondence,
    pub(super) resources: CheckedBoundaryResources,
}

impl ValueSchemaRemap {
    pub fn tables(&self, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest) -> Result<Vec<RemappedValueTable>, ValueSchemaRemapError> {
        let destination = self.stage_boundary_sources(source, forest)?;
        self.map_graph(source, forest, &destination)
    }

    pub fn tables_into(&self, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, destination: &[ValueTableSource], destination_forest: &SourceCaptureConstructionForest) -> Result<Vec<RemappedValueTable>, ValueSchemaRemapError> {
        let destination = self.independent_boundary_destination(destination, destination_forest)?;
        self.map_graph(source, forest, &destination)
    }

    fn map_graph(&self, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, witness: &BoundaryGraphDestination) -> Result<Vec<RemappedValueTable>, ValueSchemaRemapError> {
        self.validate_forest_sources(source, forest)?;
        let demand = resources::demand::resource_demand_full(source);
        let checked = witness.checked(self, source, forest, &demand)?;
        let destination = &checked.headers;
        let correspondence = checked.correspondence;
        let resources = checked.resources;
        graph_dependencies::validate_tables(&source.iter().map(|row| (&row.producer, &row.tables)).collect::<Vec<_>>())?;
        let mut result = Vec::new();
        for original in source {
            let owner = self.producer(&original.producer)?;
            let matches = destination.iter().filter(|row| row.producer == owner).collect::<Vec<_>>();
            let [target] = matches.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner)); };
            let mapped = RemappedValueTable {
                resources: resources.clone(),
                source: original.producer.clone(), destination: owner,
                policy_context: original.policy_context.clone(),
                tables: self.map_table(&original.producer, &original.tables, &correspondence, &target.tables)?,
                capture_inventory: self.capture_inventory(&original.capture_inventory)?,
                children: original.children.iter().map(|row| Ok(SourceCaptureChildLink { event: self.source_event(&row.event)?, child: self.producer(&row.child)? })).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            };
            Self::validate_header(&mapped.tables, &mapped.destination)?;
            result.push(mapped);
        }
        for (original, row) in source.iter().zip(&result) {
            for child in &row.children {
                if destination.iter().filter(|row| row.producer == child.child).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(child.child.clone())); }
                if !matches!(&child.child, ProducerCfgOwnerId::Literal { enclosing, capture_point, .. } if enclosing.as_ref() == &child.event.owner && *capture_point == child.event.point) {
                    return Err(ValueSchemaRemapError::MissingProducer(child.child.clone()));
                }
            }
            for (before, production) in original.tables.source_capture_productions.iter().zip(&row.tables.source_capture_productions) {
                let mapped_children = result.iter().filter(|row| row.source == before.child && row.destination == production.child).collect::<Vec<_>>();
                let [mapped_child] = mapped_children.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(before.child.clone())); };
                let requirements = row.capture_inventory.required.iter().filter(|requirement| requirement.event == production.event && requirement.output == production.output && requirement.child == production.child && requirement.signature == production.signature).collect::<Vec<_>>();
                let [requirement] = requirements.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: production.event.owner.clone(), point: production.event.point }); };
                for (ordinal, input) in production.inputs.iter().enumerate() {
                    let children = destination.iter().filter(|row| row.producer == input.formal.owner).collect::<Vec<_>>();
                    let [child] = children.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone())); };
                    let signatures = child.tables.boundary_signatures.iter().filter(|row| row.owner == input.formal.owner).collect::<Vec<_>>();
                    let [signature] = signatures.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySignature(input.formal.owner.clone())); };
                    let Some(formal) = signature.ports.get(input.formal.port) else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone())); };
                    if input.formal.owner != production.child || formal.role != (BoundaryPortRole::Capture { ordinal, binding: input.capture.binding.clone() }) || requirement.captures.get(ordinal) != Some(&input.capture) {
                        return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone()));
                    }
                    Self::capture_shape(&mapped_child.tables, &formal.shape, &input.capture.ty)?;
                    Self::capture_shape(&row.tables, &input.shape, &input.capture.ty)?;
                    let coordinates = row.tables.source_slots.iter().filter(|slot| slot.id == input.coordinate && slot.role == SourceCoordinateRole::Binding(input.capture.binding.clone()) && slot.shape == input.shape).collect::<Vec<_>>();
                    let [_] = coordinates.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySlot(input.coordinate.clone())); };
                }
            }
        }
        mapped_dependencies::validate_full(self, source, &result, &correspondence, &resources)?;
        Ok(result)
    }

    fn selective_correspondence(&self, root: &ProducerCfgOwnerId, tables: &EvaluatedValueTables, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, destination: &BoundaryGraphDestination, demand: &resources::BoundaryResourceDemand) -> Result<destination_evidence::CheckedBoundaryDestination, ValueSchemaRemapError> {
        let roots = source.iter().filter(|row| &row.producer == root).collect::<Vec<_>>();
        let [carrier] = roots.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); };
        if &carrier.tables != tables { return Err(ValueSchemaRemapError::MissingProducer(root.clone())); }
        for owner in tables.source_use_graphs.iter().map(|row| &row.id.owner).chain(tables.staged_loan_queries.iter().map(|row| &row.id.owner)) {
            if source.iter().filter(|row| &row.producer == owner && &row.tables == tables).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
        }
        let checked = destination.checked(self, source, forest, demand)?;
        graph_dependencies::validate_tables(&source.iter().map(|row| (&row.producer, &row.tables)).collect::<Vec<_>>())?;
        Ok(checked)
    }

    pub fn source_use_graphs_into(&self, root: &ProducerCfgOwnerId, tables: &EvaluatedValueTables, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, destination: &BoundaryGraphDestination) -> Result<RemappedSourceUseGraphs, ValueSchemaRemapError> {
        let demand = resources::demand::resource_demand_uses(root, tables, source);
        let checked = self.selective_correspondence(root, tables, source, forest, destination, &demand)?;
        let graphs = self.source_use_graphs(tables, &checked.correspondence)?;
        mapped_dependencies::validate_uses(root, self, &tables.source_use_graphs, &graphs, source, &checked.headers, &checked.correspondence, &checked.resources)?;
        Ok(RemappedSourceUseGraphs { graphs, resources: checked.resources })
    }

    pub(super) fn staged_loan_queries_into(&self, root: &ProducerCfgOwnerId, tables: &EvaluatedValueTables, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, destination: &BoundaryGraphDestination) -> Result<RemappedStagedLoanQueries, ValueSchemaRemapError> {
        let demand = resources::demand::resource_demand_stages(root, tables, source);
        let checked = self.selective_correspondence(root, tables, source, forest, destination, &demand)?;
        let queries = self.staged_loan_queries(tables, &checked.correspondence)?;
        mapped_dependencies::validate_stages(root, self, &tables.staged_loan_queries, &queries, source, &checked.headers, &checked.correspondence, &checked.resources)?;
        Ok(RemappedStagedLoanQueries { queries, correspondence: checked.correspondence, resources: checked.resources })
    }
}
