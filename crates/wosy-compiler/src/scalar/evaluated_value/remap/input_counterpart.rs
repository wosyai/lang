use super::*;
mod types;
mod ast;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryInputOriginal {
    owner: ProducerCfgOwnerId,
    construction: BoundaryResourceConstructionEvidence,
    policy_context: SourceCapturePolicyContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BoundaryNominalCounterpart {
    Struct { owner: ProducerCfgOwnerId, source: ScalarStruct, destination: ScalarStruct, fields: Vec<BoundaryFieldCounterpart> },
    Enum { owner: ProducerCfgOwnerId, source: ScalarEnum, destination: ScalarEnum },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryFieldCounterpart {
    owner: ProducerCfgOwnerId,
    source: ScalarStructField,
    destination: ScalarStructField,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryTypeCounterpart {
    owner: ProducerCfgOwnerId,
    source: ScalarType,
    destination: ScalarType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryShapeTypeCounterpart {
    owner: ProducerCfgOwnerId,
    source: ResolvedValueShape,
    destination: ResolvedValueShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryNominalDemand {
    counterpart: BoundaryNominalCounterpart,
    root: ValueShapeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BoundaryInputQualification {
    Mapped { source: ProducerCfgOwnerId, destination: ProducerCfgOwnerId },
    SharedOriginal { owner: ProducerCfgOwnerId },
}

impl BoundaryInputQualification {
    fn mapped(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId) -> Result<Self, ValueSchemaRemapError> {
        Ok(Self::Mapped { source: owner.clone(), destination: remap.boundary_producer(owner)? })
    }

    fn creator(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId) -> Result<Self, ValueSchemaRemapError> {
        let destination = resources::qualified_input_owner(remap, owner)?;
        if remap.producers.iter().any(|row| row.0 == *owner) { Ok(Self::Mapped { source: owner.clone(), destination }) }
        else { Ok(Self::SharedOriginal { owner: destination }) }
    }

    fn destination(&self) -> &ProducerCfgOwnerId {
        match self { Self::Mapped { destination, .. } => destination, Self::SharedOriginal { owner } => owner }
    }

    fn validate(&self, source: &ValueTableSource, destination: &ValueTableSource) -> Result<(), ValueSchemaRemapError> {
        let valid = match self {
            Self::Mapped { source: owner, destination: actual } => source.producer == *owner && destination.producer == *actual,
            Self::SharedOriginal { owner } => source.producer == *owner && destination.producer == *owner
                && source.resource_construction == destination.resource_construction && source.policy_context == destination.policy_context
                && source.tables.shapes == destination.tables.shapes
                && source.tables.shapes == source.resource_construction.original().evaluated_values.shapes,
        };
        if valid { Ok(()) } else { Err(ValueSchemaRemapError::MissingProducer(source.producer.clone())) }
    }

    fn shape(&self, remap: &ValueSchemaRemap, id: &ValueShapeId) -> Result<ValueShapeId, ValueSchemaRemapError> {
        match self {
            Self::Mapped { source, .. } if id.producer == *source => remap.boundary_shape(id),
            Self::SharedOriginal { owner } if id.producer == *owner => Ok(id.clone()),
            _ => Err(ValueSchemaRemapError::MissingShape(id.clone())),
        }
    }

    fn point(&self, remap: &ValueSchemaRemap, event: &SourceEventId) -> Result<SourceEventId, ValueSchemaRemapError> {
        match self {
            Self::Mapped { source, .. } if event.owner == *source => remap.boundary_point(event),
            Self::SharedOriginal { owner } if event.owner == *owner => Ok(event.clone()),
            _ => Err(ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckedBoundaryInputTypes {
    source_forest: SourceCaptureConstructionForest,
    destination_forest: SourceCaptureConstructionForest,
    originals: Vec<BoundaryInputOriginal>,
    nominals: Vec<BoundaryNominalCounterpart>,
    fields: Vec<BoundaryFieldCounterpart>,
    types: Vec<BoundaryTypeCounterpart>,
    shapes: Vec<BoundaryShapeTypeCounterpart>,
    qualifications: Vec<BoundaryInputQualification>,
    nominal_demands: Vec<BoundaryNominalDemand>,
    syntax_outputs: Vec<ast::OriginalSyntaxOutputPair>,
    output_incidences: Vec<ast::outputs::OriginalOutputIncidencePair>,
    call_signatures: Vec<ast::signatures::OriginalCallSignaturePair>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BoundaryInputTypeEvidence {
    Transported,
    Independent(CheckedBoundaryInputTypes),
}

pub(super) enum OriginalInputEndpoint { Source, Actual }

fn carrier<'a>(forest: &'a SourceCaptureConstructionForest, owner: &ProducerCfgOwnerId) -> Result<&'a ValueTableSource, ValueSchemaRemapError> {
    let rows = forest.sources.iter().filter(|row| row.producer == *owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
    let cfg = row.resource_construction.original();
    if cfg.producer_owner != *owner || cfg.initial_points != row.initial_points || cfg.capture_points != row.capture_points
        || cfg.source_capture_inventory != row.capture_inventory || *cfg.top_level != row.top_level
        || cfg.evaluated_values.values != row.tables.values || cfg.evaluated_values.source_capture_productions != row.tables.source_capture_productions {
        return Err(ValueSchemaRemapError::MissingProducer(owner.clone()));
    }
    Ok(row)
}

fn lexical(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => lexical(enclosing),
    }
}

impl CheckedBoundaryInputTypes {
    #[cfg(test)]
    pub(super) fn ast_proof_candidates(&self) -> Vec<Self> { ast::proof_candidates(self) }
    #[cfg(test)]
    pub(super) fn ast_evidence_counts(&self) -> (usize, usize, usize, usize) {
        let (annotations, errors) = ast::annotation_counts(&self.syntax_outputs);
        (annotations, errors, self.output_incidences.len(), self.call_signatures.len())
    }
    pub(super) fn formal_role_pair(&self, remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &BoundaryPortRole, transported: &BoundaryPortRole, actual: &BoundaryPortRole) -> Result<bool, ValueSchemaRemapError> {
        ast::role(remap, self, owner, source, transported, actual)
    }

    pub(super) fn capture_input_pair(&self, remap: &ValueSchemaRemap, source: &SourceCaptureProduction, actual: &SourceCaptureProduction) -> Result<(), ValueSchemaRemapError> {
        ast::captures(remap, self, source, actual)
    }

    pub(super) fn capture_original<'a>(&self, originals: &'a [BoundaryResourceConstructionEvidence], production: &SourceCaptureProduction, endpoint: OriginalInputEndpoint) -> Result<&'a ScalarReferenceCfg, ValueSchemaRemapError> {
        let forest = match endpoint { OriginalInputEndpoint::Source => &self.source_forest, OriginalInputEndpoint::Actual => &self.destination_forest };
        let original = carrier(forest, &production.event.owner)?;
        let rows = originals.iter().filter(|row| row.original().producer_owner == production.event.owner).collect::<Vec<_>>();
        let [snapshot] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(production.event.owner.clone())); };
        if **snapshot != original.resource_construction || !self.originals.iter().any(|row| row.owner == production.event.owner && row.construction == **snapshot && row.policy_context == original.policy_context) { return Err(ValueSchemaRemapError::MissingProducer(production.event.owner.clone())); }
        let cfg = snapshot.original();
        let invalid = || ValueSchemaRemapError::MissingPoint { producer: production.event.owner.clone(), point: production.event.point };
        if cfg.evaluated_values.source_capture_productions.iter().filter(|row| **row == *production).count() != 1 { return Err(invalid()); }
        let points = cfg.initial_points.iter().filter(|point| point.id == production.event.point).collect::<Vec<_>>();
        let [point] = points.as_slice() else { return Err(invalid()); };
        let CfgPointKind::Capture { captures, signature, .. } = &point.kind else { return Err(invalid()); };
        if *signature != production.signature || captures.len() != production.inputs.len() || !captures.iter().zip(&production.inputs).all(|(capture, input)| *capture == input.capture)
            || cfg.source_capture_inventory.required.iter().filter(|row| row.event == production.event && row.output == production.output && row.child == production.child && row.signature == production.signature && row.captures == *captures).count() != 1
            || cfg.capture_points.iter().filter(|row| row.point == production.event.point && row.occurrence.source_span == point.source_span && row.occurrence.output_projection == production.output.resolved.output_projection).count() != 1
            || cfg.literal_cfgs.iter().filter(|child| child.capture_point == production.event.point && child.reference_cfg.producer_owner == production.child && child.literal == point.source_span && child.captures == *captures).count() != 1 { return Err(invalid()); }
        let child = carrier(forest, &production.child)?;
        if !originals.contains(&child.resource_construction) { return Err(ValueSchemaRemapError::MissingProducer(production.child.clone())); }
        Ok(cfg)
    }
    fn qualification(&self, remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId) -> Result<BoundaryInputQualification, ValueSchemaRemapError> {
        let rows = self.qualifications.iter().filter(|row| match row {
            BoundaryInputQualification::Mapped { source, .. } => source == owner,
            BoundaryInputQualification::SharedOriginal { owner: source } => source == owner,
        }).collect::<Vec<_>>();
        let [record] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
        let current = match record {
            BoundaryInputQualification::Mapped { .. } => BoundaryInputQualification::mapped(remap, owner)?,
            BoundaryInputQualification::SharedOriginal { .. } => BoundaryInputQualification::creator(remap, owner)?,
        };
        if current != **record { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); }
        Ok(current)
    }

    pub(super) fn extend_original_types(&self, remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &[ScalarType], destination: &[ScalarType], root: &ValueShapeId) -> Result<Self, ValueSchemaRemapError> {
        types::extend_original_types(remap, self, owner, source, destination, root)
    }

    pub(super) fn type_pair(&self, owner: &ProducerCfgOwnerId, source: &ScalarType, destination: &ScalarType) -> bool {
        self.types.iter().filter(|row| row.owner == *owner && row.source == *source && row.destination == *destination).count() == 1
    }

    fn retain_original(&mut self, row: &ValueTableSource) -> Result<(), ValueSchemaRemapError> {
        let original = BoundaryInputOriginal { owner: row.producer.clone(), construction: row.resource_construction.clone(), policy_context: row.policy_context.clone() };
        if !self.originals.contains(&original) { self.originals.push(original); }
        Ok(())
    }
}

pub(super) fn checked_header_types(remap: &ValueSchemaRemap, source: &[BoundaryTableHeaders], destination: &[BoundaryTableHeaders], input: &header_counterpart::BoundaryHeaderRoleInput<'_>) -> Result<BoundaryInputTypeEvidence, ValueSchemaRemapError> {
    let header_counterpart::BoundaryHeaderRoleInput::Independent { source: supplied, source_forest, destination: actual, destination_forest } = input else { return Ok(BoundaryInputTypeEvidence::Transported); };
    remap.validate_forest_sources(supplied, source_forest)?;
    remap.validate_forest_sources(actual, destination_forest)?;
    let mut checked = CheckedBoundaryInputTypes { source_forest: (*source_forest).clone(), destination_forest: (*destination_forest).clone(), originals: Vec::new(), nominals: Vec::new(), fields: Vec::new(), types: Vec::new(), shapes: Vec::new(), qualifications: Vec::new(), nominal_demands: Vec::new(), syntax_outputs: Vec::new(), output_incidences: Vec::new(), call_signatures: Vec::new() };
    for header in source {
        let original = carrier(source_forest, &header.producer)?;
        let owner = remap.boundary_producer(&header.producer)?;
        let receiving = carrier(destination_forest, &owner)?;
        if !supplied.contains(original) || !actual.contains(receiving) || BoundaryTableHeaders::extract(original) != *header
            || destination.iter().filter(|row| row.producer == owner && **row == BoundaryTableHeaders::extract(receiving)).count() != 1
            || original.tables.shapes != original.resource_construction.original().evaluated_values.shapes
            || receiving.tables.shapes != receiving.resource_construction.original().evaluated_values.shapes {
            return Err(ValueSchemaRemapError::MissingProducer(header.producer.clone()));
        }
        checked.retain_original(original)?;
        checked.retain_original(receiving)?;
        name_resolution::validate_counterpart(remap, original, receiving)?;
        let qualification = BoundaryInputQualification::mapped(remap, &original.producer)?;
        types::roots(remap, &mut checked, original, receiving, &qualification)?;
        checked.qualifications.push(qualification);
    }
    ast::roots(remap, &mut checked, &source.iter().map(|header| header.producer.clone()).collect::<Vec<_>>())?;
    ast::signatures::registered_calls(remap, &mut checked, &source.iter().map(|header| header.producer.clone()).collect::<Vec<_>>())?;
    Ok(BoundaryInputTypeEvidence::Independent(checked))
}

pub(super) fn retain_creator_types(remap: &ValueSchemaRemap, header: &BoundaryInputTypeEvidence, source: &[BoundaryResourceConstructionEvidence], destination: &[BoundaryResourceConstructionEvidence], source_forest: &SourceCaptureConstructionForest, destination_forest: &SourceCaptureConstructionForest) -> Result<BoundaryInputTypeEvidence, ValueSchemaRemapError> {
    let BoundaryInputTypeEvidence::Independent(prefix) = header else { return Ok(BoundaryInputTypeEvidence::Transported); };
    if prefix.source_forest != *source_forest || prefix.destination_forest != *destination_forest { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    let mut checked = prefix.clone();
    for snapshot in source {
        let original = carrier(source_forest, &snapshot.original().producer_owner)?;
        if original.resource_construction != *snapshot { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); }
        let qualification = BoundaryInputQualification::creator(remap, &original.producer)?;
        let actual = carrier(destination_forest, qualification.destination())?;
        if !destination.contains(&actual.resource_construction) { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); }
        qualification.validate(original, actual)?;
        let extension = match &qualification {
            BoundaryInputQualification::Mapped { .. } => {
                let evidence = remap.checked_qualified_header_shapes(&[BoundaryTableHeaders::extract(original)], &[BoundaryTableHeaders::extract(actual)], &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source_forest.sources, source_forest, destination: &destination_forest.sources, destination_forest })?;
                let BoundaryInputTypeEvidence::Independent(extension) = evidence else { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); };
                extension
            }
            BoundaryInputQualification::SharedOriginal { .. } => {
                let mut extension = CheckedBoundaryInputTypes { source_forest: source_forest.clone(), destination_forest: destination_forest.clone(), originals: Vec::new(), nominals: Vec::new(), fields: Vec::new(), types: Vec::new(), shapes: Vec::new(), qualifications: vec![qualification.clone()], nominal_demands: Vec::new(), syntax_outputs: Vec::new(), output_incidences: Vec::new(), call_signatures: Vec::new() };
                extension.retain_original(original)?;
                extension.retain_original(actual)?;
                types::roots(remap, &mut extension, original, actual, &qualification)?;
                ast::roots(remap, &mut extension, &[original.producer.clone()])?;
                ast::signatures::registered_calls(remap, &mut extension, &[original.producer.clone()])?;
                extension
            }
        };
        checked.retain_original(original)?;
        checked.retain_original(actual)?;
        types::merge(&mut checked, &extension.nominal_demands, &extension.fields, &extension.types, &extension.shapes)?;
        if !checked.qualifications.contains(&qualification) { checked.qualifications.push(qualification); }
        for qualification in extension.qualifications { if !checked.qualifications.contains(&qualification) { checked.qualifications.push(qualification); } }
        for original in extension.originals { if !checked.originals.contains(&original) { checked.originals.push(original); } }
        for record in extension.syntax_outputs { if !checked.syntax_outputs.contains(&record) { checked.syntax_outputs.push(record); } }
        for record in extension.output_incidences { if !checked.output_incidences.contains(&record) { checked.output_incidences.push(record); } }
        for record in extension.call_signatures { if !checked.call_signatures.contains(&record) { checked.call_signatures.push(record); } }
    }
    for snapshot in destination {
        let actual = carrier(destination_forest, &snapshot.original().producer_owner)?;
        if actual.resource_construction != *snapshot { return Err(ValueSchemaRemapError::MissingProducer(actual.producer.clone())); }
        checked.retain_original(actual)?;
    }
    Ok(BoundaryInputTypeEvidence::Independent(checked))
}

pub(super) fn postvalidate(remap: &ValueSchemaRemap, source: &[ValueTableSource], correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    match (&correspondence.input_types, resources.input_types(), resources.provenance()) {
        (BoundaryInputTypeEvidence::Transported, BoundaryInputTypeEvidence::Transported, resources::BoundaryResourceProvenance::Copied { .. }) => Ok(()),
        (BoundaryInputTypeEvidence::Independent(header), BoundaryInputTypeEvidence::Independent(_), resources::BoundaryResourceProvenance::Independent { originals }) => {
            remap.validate_forest_sources(source, &header.source_forest)?;
            let destination = source.iter().map(|row| carrier(&header.destination_forest, &remap.boundary_producer(&row.producer)?).cloned()).collect::<Result<Vec<_>, _>>()?;
            let mut selected = Vec::new();
            for row in destination { if !selected.contains(&row) { selected.push(row); } }
            let checked = remap.checked_qualified_header_shapes(&source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &selected.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source, source_forest: &header.source_forest, destination: &selected, destination_forest: &header.destination_forest })?;
            if checked != correspondence.input_types { return Err(ValueSchemaRemapError::EmptyTransportSources); }
            let retained = retain_creator_types(remap, &checked, resources.source_originals(), originals, &header.source_forest, &header.destination_forest)?;
            if retained != *resources.input_types() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
            Ok(())
        }
        _ => Err(ValueSchemaRemapError::EmptyTransportSources),
    }
}

pub(super) fn direct_storage_originals(remap: &ValueSchemaRemap, evidence: &BoundaryInputTypeEvidence, source: &BoundaryResourceConstructionEvidence, destination: &BoundaryResourceConstructionEvidence) -> Result<(), ValueSchemaRemapError> {
    let BoundaryInputTypeEvidence::Independent(checked) = evidence else { return Err(ValueSchemaRemapError::EmptyTransportSources); };
    let mut isolated = checked.clone();
    let before = carrier(&isolated.source_forest, &source.original().producer_owner)?;
    let after = carrier(&isolated.destination_forest, &destination.original().producer_owner)?;
    if before.resource_construction != *source || after.resource_construction != *destination { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    ast::direct_addresses(remap, &mut isolated, source, destination)
}

pub(super) fn allocation_identity_originals(remap: &ValueSchemaRemap, evidence: &BoundaryInputTypeEvidence, source: &BoundaryResourceConstructionEvidence, destination: &BoundaryResourceConstructionEvidence) -> Result<(), ValueSchemaRemapError> {
    let BoundaryInputTypeEvidence::Independent(checked) = evidence else { return Err(ValueSchemaRemapError::EmptyTransportSources); };
    let mut isolated = checked.clone();
    let before = carrier(&isolated.source_forest, &source.original().producer_owner)?;
    let after = carrier(&isolated.destination_forest, &destination.original().producer_owner)?;
    if before.resource_construction != *source || after.resource_construction != *destination { return Err(ValueSchemaRemapError::EmptyTransportSources); }
    ast::allocation_addresses(remap, &mut isolated, source, destination)
}
