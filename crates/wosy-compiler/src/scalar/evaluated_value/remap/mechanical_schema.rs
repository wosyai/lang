use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar::evaluated_value) struct MechanicalSchemaTableSource {
    pub(in crate::scalar::evaluated_value) policy_context: SourceCapturePolicyContext,
    pub(in crate::scalar::evaluated_value) producer: ProducerCfgOwnerId,
    pub(in crate::scalar::evaluated_value) tables: EvaluatedValueTables,
    pub(in crate::scalar::evaluated_value) capture_inventory: SourceCaptureInventory,
    pub(in crate::scalar::evaluated_value) initial_points: Vec<ReferenceFlowPoint>,
    pub(in crate::scalar::evaluated_value) capture_points: Vec<ScalarCaptureCfgPoint>,
    pub(in crate::scalar::evaluated_value) top_level: ScalarTopLevelCorrespondence,
    pub(in crate::scalar::evaluated_value) children: Vec<SourceCaptureChildLink>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar::evaluated_value) struct MechanicalSchemaTableResult {
    pub(in crate::scalar::evaluated_value) policy_context: SourceCapturePolicyContext,
    pub(in crate::scalar::evaluated_value) source: ProducerCfgOwnerId,
    pub(in crate::scalar::evaluated_value) destination: ProducerCfgOwnerId,
    pub(in crate::scalar::evaluated_value) tables: EvaluatedValueTables,
    pub(in crate::scalar::evaluated_value) capture_inventory: SourceCaptureInventory,
    pub(in crate::scalar::evaluated_value) children: Vec<SourceCaptureChildLink>,
}

impl ValueSchemaRemap {
    pub(in crate::scalar::evaluated_value) fn mechanical_schema_tables(&self, source: &[MechanicalSchemaTableSource]) -> Result<Vec<MechanicalSchemaTableResult>, ValueSchemaRemapError> {
        let headers = source.iter().map(|row| BoundaryTableHeaders::from_tables(&row.producer, &row.policy_context, &row.tables)).collect::<Vec<_>>();
        let destination_evidence::StagedBoundaryHeaders { headers, correspondence, .. } = self.stage_boundary_headers(&headers)?;
        let mut result = Vec::new();
        for original in source {
            let owner = self.producer(&original.producer)?;
            let matches = headers.iter().filter(|row| row.producer == owner).collect::<Vec<_>>();
            let [target] = matches.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner)); };
            result.push(MechanicalSchemaTableResult {
                policy_context: original.policy_context.clone(),
                source: original.producer.clone(),
                destination: owner,
                tables: self.map_table(&original.producer, &original.tables, &correspondence, &target.tables)?,
                capture_inventory: self.capture_inventory(&original.capture_inventory)?,
                children: original.children.iter().map(|row| Ok(SourceCaptureChildLink { event: self.source_event(&row.event)?, child: self.producer(&row.child)? })).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            });
        }
        Ok(result)
    }
}
