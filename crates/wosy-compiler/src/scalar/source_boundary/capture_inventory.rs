use super::*;
mod resource_construction;
pub use resource_construction::BoundaryResourceConstructionEvidence;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceCaptureConstructionPhase {
    Building,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCaptureRequirement {
    pub event: SourceEventId,
    pub output: EvaluatedValueId,
    pub child: ProducerCfgOwnerId,
    pub signature: ScalarType,
    pub captures: Vec<closure::ClosureCapture>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCaptureInventory {
    pub phase: SourceCaptureConstructionPhase,
    pub required: Vec<SourceCaptureRequirement>,
}

impl SourceCaptureInventory {
    pub fn building() -> Self {
        Self { phase: SourceCaptureConstructionPhase::Building, required: Vec::new() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCaptureChildLink {
    pub event: SourceEventId,
    pub child: ProducerCfgOwnerId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCapturePolicyContext {
    pub structs: Vec<ScalarStruct>,
    pub enums: Vec<ScalarEnum>,
    pub generic_scope: ProducerCfgOwnerId,
    pub generic_declarations: Vec<ScalarGenericParameter>,
    pub generic_parameters: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCaptureInputPort {
    pub capture: closure::ClosureCapture,
    pub formal: BoundaryFormalPortRef,
    pub coordinate: SourceSlotRef,
    pub shape: ValueShapeId,
    pub policy: CopyPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCaptureProduction {
    pub event: SourceEventId,
    pub output: EvaluatedValueId,
    pub child: ProducerCfgOwnerId,
    pub signature: ScalarType,
    pub inputs: Vec<SourceCaptureInputPort>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCapturePendingInput {
    pub event: SourceEventId,
    pub output: EvaluatedValueId,
    pub child: ProducerCfgOwnerId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCapturePolicyInstance {
    pub event: SourceEventId,
    pub formal: BoundaryFormalPortRef,
    pub substitutions: BTreeMap<String, ScalarType>,
    pub ty: ScalarType,
    pub policy: CopyPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueTableSource {
    pub resource_construction: BoundaryResourceConstructionEvidence,
    pub policy_context: SourceCapturePolicyContext,
    pub producer: ProducerCfgOwnerId,
    pub tables: EvaluatedValueTables,
    pub capture_inventory: SourceCaptureInventory,
    pub initial_points: Vec<ReferenceFlowPoint>,
    pub capture_points: Vec<ScalarCaptureCfgPoint>,
    pub top_level: ScalarTopLevelCorrespondence,
    pub children: Vec<SourceCaptureChildLink>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemappedValueTable {
    pub resources: CheckedBoundaryResources,
    pub policy_context: SourceCapturePolicyContext,
    pub source: ProducerCfgOwnerId,
    pub destination: ProducerCfgOwnerId,
    pub tables: EvaluatedValueTables,
    pub capture_inventory: SourceCaptureInventory,
    pub children: Vec<SourceCaptureChildLink>,
}

pub fn capture_table_sources(root: &ScalarReferenceCfg, context: &SourceCapturePolicyContext) -> Result<Vec<ValueTableSource>, ValueSchemaRemapError> {
    fn validate_seed_refs(cfg: &ScalarReferenceCfg) -> Result<(), ValueSchemaRemapError> {
        let expected = cfg.literal_cfgs.iter().map(|child| child.constructor_seed.id.clone()).collect::<Vec<_>>();
        if cfg.evaluated_values.source_literal_constructor_seeds != expected {
            return Err(ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone()));
        }
        Ok(())
    }
    fn collect(cfg: &ScalarReferenceCfg, context: &SourceCapturePolicyContext, sources: &mut Vec<ValueTableSource>) -> Result<(), ValueSchemaRemapError> {
        module_resolver_input::validate_endpoint(cfg)?;
        reference::name_resolution::validate_null_constructors(cfg, &context.structs)
            .map_err(|_| ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone()))?;
        if sources.iter().any(|known| known.producer == cfg.producer_owner) {
            return Err(ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone()));
        }
        let mut children = Vec::new();
        for literal in &cfg.literal_cfgs {
            let points = cfg.initial_points.iter().filter(|point| point.id == literal.capture_point).collect::<Vec<_>>();
            let [point] = points.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: cfg.producer_owner.clone(), point: literal.capture_point }); };
            let CfgPointKind::Capture { literal: span, environment, signature, captures, parameters, parameter_spans, body } = &point.kind else {
                return Err(ValueSchemaRemapError::MissingProducer(literal.reference_cfg.producer_owner.clone()));
            };
            let declaration = reference::source_epoch::SourceCaptureDeclaration::new(
                &cfg.producer_owner, &point.source_span, point.id, *span, &environment.literal,
                signature, captures, parameters, parameter_spans, body,
            ).map_err(|_| ValueSchemaRemapError::MissingPoint { producer: cfg.producer_owner.clone(), point: point.id })?;
            if declaration.literal_owner != literal.reference_cfg.producer_owner
                || declaration.literal_span != &literal.literal || captures != &literal.captures
            { return Err(ValueSchemaRemapError::MissingProducer(literal.reference_cfg.producer_owner.clone())); }
            let seed = &literal.constructor_seed;
            if seed.id.event != declaration.event || seed.id.child != declaration.literal_owner
                || seed.environment != *environment || seed.captures != *captures {
                return Err(ValueSchemaRemapError::MissingPoint { producer: cfg.producer_owner.clone(), point: point.id });
            }
            parent_seed::validate_original(cfg, literal)?;
            let link = SourceCaptureChildLink { event: declaration.event, child: declaration.literal_owner };
            if children.contains(&link) { return Err(ValueSchemaRemapError::MissingProducer(link.child)); }
            children.push(link);
        }
        validate_seed_refs(cfg)?;
        sources.push(ValueTableSource {
            resource_construction: BoundaryResourceConstructionEvidence::extract(cfg, &context.structs),
            policy_context: context.clone(),
            producer: cfg.producer_owner.clone(), tables: cfg.evaluated_values.clone(),
            capture_inventory: cfg.source_capture_inventory.clone(), initial_points: cfg.initial_points.clone(),
            capture_points: cfg.capture_points.clone(), top_level: *cfg.top_level.clone(), children,
        });
        for literal in &cfg.literal_cfgs { collect(&literal.reference_cfg, context, sources)?; }
        Ok(())
    }
    let mut sources = Vec::new();
    collect(root, context, &mut sources)?;
    Ok(sources)
}
