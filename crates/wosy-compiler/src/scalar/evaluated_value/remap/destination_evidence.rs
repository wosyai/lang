use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryTableHeaders {
    pub(super) producer: ProducerCfgOwnerId,
    pub(super) policy_context: SourceCapturePolicyContext,
    pub(super) tables: EvaluatedValueTables,
}

impl BoundaryTableHeaders {
    pub(super) fn extract(source: &ValueTableSource) -> Self {
        Self::from_tables(&source.producer, &source.policy_context, &source.tables)
    }

    pub(super) fn from_tables(producer: &ProducerCfgOwnerId, policy_context: &SourceCapturePolicyContext, source: &EvaluatedValueTables) -> Self {
        let mut tables = EvaluatedValueTables::new();
        tables.shapes = source.shapes.clone();
        tables.source_slots = source.source_slots.clone();
        tables.boundary_signatures = source.boundary_signatures.clone();
        Self { producer: producer.clone(), policy_context: policy_context.clone(), tables }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundarySlotAllocation {
    source: SourceSlotRef,
    destination: SourceSlotRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BoundaryDestinationEvidence {
    Copied {
        remap: ValueSchemaRemap,
        original: Vec<ValueTableSource>,
        forest: SourceCaptureConstructionForest,
        headers: Vec<BoundaryTableHeaders>,
        correspondence: BoundaryArenaCorrespondence,
        allocations: Vec<BoundarySlotAllocation>,
    },
    Independent {
        original: Vec<ValueTableSource>,
        forest: SourceCaptureConstructionForest,
        headers: Vec<BoundaryTableHeaders>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryGraphDestination {
    evidence: BoundaryDestinationEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckedBoundaryDestination {
    pub(super) headers: Vec<BoundaryTableHeaders>,
    pub(super) correspondence: BoundaryArenaCorrespondence,
    pub(super) resources: CheckedBoundaryResources,
}

impl BoundaryGraphDestination {
    pub(super) fn checked(&self, remap: &ValueSchemaRemap, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, demand: &resources::BoundaryResourceDemand) -> Result<CheckedBoundaryDestination, ValueSchemaRemapError> {
        let (headers, correspondence) = self.checked_headers(remap, source, forest)?;
        let resources = match &self.evidence {
            BoundaryDestinationEvidence::Copied { .. } => resources::allocation::allocate_copied_resources(remap, source, forest, demand, &correspondence)?,
            BoundaryDestinationEvidence::Independent { original, forest: destination_forest, .. } => resources::independent::validate_independent_resources(remap, source, forest, original, destination_forest, demand, &correspondence)?,
        };
        Ok(CheckedBoundaryDestination { headers: headers.to_vec(), correspondence, resources })
    }

    pub(super) fn checked_headers(&self, remap: &ValueSchemaRemap, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest) -> Result<(&[BoundaryTableHeaders], BoundaryArenaCorrespondence), ValueSchemaRemapError> {
        remap.validate_forest_sources(source, forest)?;
        match &self.evidence {
            BoundaryDestinationEvidence::Copied { remap: original_remap, original, forest: original_forest, headers, correspondence, allocations } => {
                for original_source in source.iter().chain(original) {
                    if original_remap != remap || original != source || original_forest != forest { return Err(ValueSchemaRemapError::MissingProducer(original_source.producer.clone())); }
                }
                for root in forest.roots.iter().chain(&original_forest.roots) {
                    if original_remap != remap || original != source || original_forest != forest { return Err(ValueSchemaRemapError::MissingProducer(root.owner.clone())); }
                }
                for source in source {
                    for slot in &source.tables.source_slots {
                        let rows = allocations.iter().filter(|row| row.source == slot.id).collect::<Vec<_>>();
                        let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySlot(slot.id.clone())); };
                        if correspondence.slot(&slot.id)? != row.destination || headers.iter().flat_map(|row| &row.tables.source_slots).filter(|slot| slot.id == row.destination).count() != 1 { return Err(ValueSchemaRemapError::MissingBoundarySlot(slot.id.clone())); }
                    }
                }
                Ok((headers, correspondence.clone()))
            }
            BoundaryDestinationEvidence::Independent { original, forest: destination_forest, headers } => {
                remap.validate_forest_sources(original, destination_forest)?;
                Ok((headers, remap.boundary_arena_correspondence(source, forest, original, destination_forest)?))
            }
        }
    }
}

pub(super) struct StagedBoundaryHeaders {
    pub(super) headers: Vec<BoundaryTableHeaders>,
    pub(super) correspondence: BoundaryArenaCorrespondence,
    allocations: Vec<BoundarySlotAllocation>,
}

impl ValueSchemaRemap {
    pub(super) fn stage_boundary_headers(&self, source: &[BoundaryTableHeaders]) -> Result<StagedBoundaryHeaders, ValueSchemaRemapError> {
        let mut headers = Vec::new();
        let mut allocations = Vec::new();
        for original in source {
            let owner = self.boundary_producer(&original.producer)?;
            if headers.iter().any(|row: &BoundaryTableHeaders| row.producer == owner) { return Err(ValueSchemaRemapError::AmbiguousBoundarySignature(owner)); }
            let mut tables = EvaluatedValueTables::new();
            tables.shapes = original.tables.shapes.iter().map(|row| self.resolved_shape(row)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
            for slot in &original.tables.source_slots {
                let mapped = SourceSlotFact { id: SourceSlotRef { owner: owner.clone(), index: tables.source_slots.len() }, role: self.source_role(&original.producer, &slot.role)?, shape: self.boundary_shape(&slot.shape)? };
                allocations.push(BoundarySlotAllocation { source: slot.id.clone(), destination: mapped.id.clone() });
                tables.source_slots.push(mapped);
            }
            for signature in &original.tables.boundary_signatures {
                let mut ports = Vec::new();
                for port in &signature.ports {
                    let rows = allocations.iter().filter(|row| row.source == port.coordinate).collect::<Vec<_>>();
                    let [allocation] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySlot(port.coordinate.clone())); };
                    ports.push(BoundaryFormalPort { role: self.boundary_role(&original.producer, &port.role)?, shape: self.boundary_shape(&port.shape)?, coordinate: allocation.destination.clone() });
                }
                tables.boundary_signatures.push(BoundarySignatureFact { owner: owner.clone(), entry: self.boundary_point(&signature.entry)?, frame: FrameVariableRef { owner: owner.clone(), variable: signature.frame.variable }, ports });
            }
            Self::validate_header(&tables, &owner)?;
            headers.push(BoundaryTableHeaders { producer: owner, policy_context: original.policy_context.clone(), tables });
        }
        let correspondence = self.boundary_header_correspondence(source, &headers)?;
        Ok(StagedBoundaryHeaders { headers, correspondence, allocations })
    }
}

impl ValueSchemaRemap {
    pub fn stage_boundary_sources(&self, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest) -> Result<crate::scalar::BoundaryGraphDestination, ValueSchemaRemapError> {
        self.validate_forest_sources(source, forest)?;
        let originals = source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>();
        let StagedBoundaryHeaders { headers, correspondence, allocations } = self.stage_boundary_headers(&originals)?;
        Ok(BoundaryGraphDestination { evidence: BoundaryDestinationEvidence::Copied { remap: self.clone(), original: source.to_vec(), forest: forest.clone(), headers, correspondence, allocations } })
    }

    pub fn independent_boundary_destination(&self, destination: &[ValueTableSource], forest: &SourceCaptureConstructionForest) -> Result<BoundaryGraphDestination, ValueSchemaRemapError> {
        self.validate_forest_sources(destination, forest)?;
        let headers = destination.iter().map(BoundaryTableHeaders::extract).collect();
        Ok(BoundaryGraphDestination { evidence: BoundaryDestinationEvidence::Independent { original: destination.to_vec(), forest: forest.clone(), headers } })
    }
}
