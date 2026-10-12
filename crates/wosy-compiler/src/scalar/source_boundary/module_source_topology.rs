use super::*;
use module_resolver_input::{SourceModuleResolverInputEvidence, SourceModuleResolverInputRef};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceModuleTopologyState {
    AwaitingConnectedConstruction,
    ConnectedConstruction(Box<SourceModuleConnectedConstruction>),
    AugmentedOriginal(Box<SourceModuleTopologyAugmentation>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceModuleConnectedConstruction {
    invocation: SourceModuleResolverInputRef,
    canonical_root: NameResolutionContext,
    points: Vec<ReferenceFlowPoint>,
    top_level: ScalarTopLevelCorrespondence,
    capture_points: Vec<ScalarCaptureCfgPoint>,
    calls: Vec<ResolvedCallSite>,
    captures: Vec<(ScalarCaptureOccurrence, CfgPointKind)>,
    placed: BTreeSet<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceModuleTopologyAugmentation {
    connected: SourceModuleConnectedConstruction,
    original_environment_prefix: Vec<CallableEnvironmentFact>,
    suffix: Vec<ReferenceFlowPoint>,
    suffix_environments: Vec<CallableEnvironmentFact>,
    final_points: Vec<ReferenceFlowPoint>,
    final_top_level: ScalarTopLevelCorrespondence,
}

pub(in crate::scalar) struct SourceModuleAugmentationInput {
    connected: SourceModuleConnectedConstruction,
    environments: Vec<CallableEnvironmentFact>,
    facts: Vec<ReferenceOrigin>,
    holder_edges: Vec<CallableHolderEdgeFact>,
    holder_exits: Vec<CallableHolderExitFact>,
    children: Vec<ProducerCfgOwnerId>,
}

fn fail(owner: &ProducerCfgOwnerId) -> ReferenceAnalysisError {
    let ProducerCfgOwnerId::Module(source) = owner else { unreachable!("module topology endpoint") };
    ReferenceAnalysisError::MissingContext { source_span: SourceSpan::new(source.clone(), ByteSpan::new(0, 0)),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }
}

fn evidence(cfg: &ScalarReferenceCfg) -> Result<&SourceModuleResolverInputEvidence, ReferenceAnalysisError> {
    let SourceModuleResolverInputState::OriginalModuleInvocation(evidence) = &cfg.module_resolver_input else {
        return Err(fail(&cfg.producer_owner));
    };
    Ok(evidence)
}

fn point(id: CfgPointId, kind: CfgPointKind, source_span: SourceSpan) -> ReferenceFlowPoint {
    ReferenceFlowPoint {
        id, kind, source_span, scope: ReferenceScopeId(0), possible_origins: Some(BTreeSet::new()),
        possible_environments: BTreeSet::new(), possible_projected_environments: Vec::new(),
        possible_callee_environments: BTreeSet::new(), possible_backings: Vec::new(),
        possible_projected_backings: Vec::new(), backing_argument_paths: Vec::new(),
        global_callable_call_paths: Vec::new(), global_callable_output_effects: Vec::new(),
        call_import_paths: Vec::new(), successors: Vec::new(),
    }
}

impl SourceModuleConnectedConstruction {
    fn validate(&self, original: &SourceModuleResolverInputEvidence) -> Result<Vec<ScalarItem>, ReferenceAnalysisError> {
        let (invocation, items, calls, captures) = original.inventory()?;
        let owner = &invocation.owner;
        if self.invocation != invocation || self.calls != calls || self.captures != captures {
            return Err(fail(owner));
        }
        let NameResolutionContext::Complete(root) = &self.canonical_root else { return Err(fail(owner)); };
        let SourceNameResolutionRoot::Module { top_level, .. } = &root.root else { return Err(fail(owner)); };
        if root.owner != *owner || *top_level != self.top_level { return Err(fail(owner)); }
        for (index, event) in self.points.iter().enumerate() {
            if event.id != CfgPointId(index) { return Err(fail(owner)); }
        }
        for row in &self.top_level.points {
            let ScalarTopLevelPointExecution::Connected { outgoing, .. } = &row.execution else { return Err(fail(owner)); };
            let event = self.points.get(row.point.0).ok_or_else(|| fail(owner))?;
            if event.successors != *outgoing { return Err(fail(owner)); }
        }
        let mut placed = BTreeSet::new();
        for (ordinal, (occurrence, kind)) in captures.iter().enumerate() {
            let span = &occurrence.source_span;
            let outermost = !captures.iter().any(|(parent, _)| parent.source_span != *span
                && parent.source_span.range.start <= span.range.start && span.range.end <= parent.source_span.range.end);
            let binding = items.iter().any(|item| matches!(item, ScalarItem::Binding(binding)
                if binding.span.start <= span.range.start && span.range.end <= binding.span.end
                && !(matches!(binding.declared_type, ScalarType::Array { .. })
                    && matches!(binding.value, ScalarExpression::ArrayLiteral { .. }))));
            if outermost && binding {
                let incidences = self.capture_points.iter().filter(|row| row.occurrence == *occurrence
                    && row.role == ScalarCapturePointRole::Capture).collect::<Vec<_>>();
                let [incidence] = incidences.as_slice() else { return Err(fail(owner)); };
                let event = self.points.get(incidence.point.0).ok_or_else(|| fail(owner))?;
                let CfgPointKind::Capture { environment, .. } = &event.kind else { return Err(fail(owner)); };
                let mut expected = kind.clone();
                let CfgPointKind::Capture { environment: target, .. } = &mut expected else { return Err(fail(owner)); };
                *target = environment.clone();
                if event.source_span != *span || event.kind != expected || environment.literal != *span { return Err(fail(owner)); }
                placed.insert(ordinal);
            }
        }
        if placed != self.placed { return Err(fail(owner)); }
        Ok(items)
    }

    fn expected_suffix(&self, original: &SourceModuleResolverInputEvidence, environment_start: usize)
        -> Result<(Vec<ReferenceFlowPoint>, Vec<CallableEnvironmentFact>), ReferenceAnalysisError> {
        let items = self.validate(original)?;
        let mut suffix = Vec::new();
        let mut environments = Vec::new();
        for site in &self.calls {
            if items.iter().any(|item| matches!(item, ScalarItem::Function(function)
                if function.span.start <= site.source_span.range.start && site.source_span.range.end <= function.span.end)) {
                continue;
            }
            if self.points.iter().chain(&suffix).any(|event| matches!(event.kind, CfgPointKind::Call { output: 0, .. })
                && event.source_span == site.source_span) { continue; }
            suffix.push(point(CfgPointId(self.points.len() + suffix.len()), CfgPointKind::Call {
                expression: site.expression.clone(), output: 0, target: site.target.clone(), argument_points: Vec::new(),
            }, site.source_span.clone()));
        }
        for (ordinal, (occurrence, kind)) in self.captures.iter().enumerate() {
            if self.placed.contains(&ordinal) { continue; }
            let mut kind = kind.clone();
            let CfgPointKind::Capture { environment, .. } = &mut kind else { return Err(fail(&self.invocation.owner)); };
            *environment = CallableEnvironmentId { literal: occurrence.source_span.clone(), index: environment_start + environments.len() };
            environments.push(CallableEnvironmentFact {
                literal: occurrence.source_span.clone(), instance: CallableEnvironmentInstance::Capture,
                origins: BTreeSet::new(), owners: BTreeSet::new(), loans: BTreeSet::new(), backings: Vec::new(),
                inputs: Vec::new(), capture_paths: Vec::new(),
            });
            suffix.push(point(CfgPointId(self.points.len() + suffix.len()), kind, occurrence.source_span.clone()));
        }
        Ok((suffix, environments))
    }
}

pub(in crate::scalar) fn freeze_connected(cfg: &mut ScalarReferenceCfg, placed: &BTreeSet<usize>) -> Result<(), ReferenceAnalysisError> {
    let original = evidence(cfg)?;
    if original.topology != SourceModuleTopologyState::AwaitingConnectedConstruction { return Err(fail(&cfg.producer_owner)); }
    let (invocation, _, calls, captures) = original.inventory()?;
    let connected = SourceModuleConnectedConstruction { invocation, canonical_root: cfg.name_resolution_context.clone(),
        points: cfg.initial_points.clone(), top_level: *cfg.top_level.clone(), capture_points: cfg.capture_points.clone(),
        calls, captures, placed: placed.clone() };
    connected.validate(original)?;
    let SourceModuleResolverInputState::OriginalModuleInvocation(original) = &mut cfg.module_resolver_input else { unreachable!(); };
    original.topology = SourceModuleTopologyState::ConnectedConstruction(Box::new(connected));
    Ok(())
}

pub(in crate::scalar) fn prepare_augmentation(cfg: &ScalarReferenceCfg) -> Result<SourceModuleAugmentationInput, ReferenceAnalysisError> {
    validate(cfg)?;
    let SourceModuleTopologyState::ConnectedConstruction(connected) = &evidence(cfg)?.topology else { return Err(fail(&cfg.producer_owner)); };
    Ok(SourceModuleAugmentationInput { connected: *connected.clone(), environments: cfg.environments.clone(),
        facts: cfg.facts.clone(), holder_edges: cfg.holder_edges.clone(), holder_exits: cfg.holder_exits.clone(),
        children: cfg.literal_cfgs.iter().map(|child| child.reference_cfg.producer_owner.clone()).collect() })
}

pub(in crate::scalar) fn finish_augmentation(cfg: &mut ScalarReferenceCfg, input: SourceModuleAugmentationInput) -> Result<(), ReferenceAnalysisError> {
    let original = evidence(cfg)?;
    let SourceModuleTopologyState::ConnectedConstruction(connected) = &original.topology else { return Err(fail(&cfg.producer_owner)); };
    let (suffix, suffix_environments) = input.connected.expected_suffix(original, input.environments.len())?;
    let mut final_points = input.connected.points.clone();
    final_points.extend(suffix.clone());
    let mut final_top_level = input.connected.top_level.clone();
    final_top_level.points.extend(suffix.iter().map(|event| ScalarTopLevelPoint {
        point: event.id, execution: ScalarTopLevelPointExecution::SupplementalAnalysisOnly,
    }));
    let mut environments = input.environments.clone();
    environments.extend(suffix_environments.clone());
    if **connected != input.connected || cfg.name_resolution_context != input.connected.canonical_root
        || cfg.initial_points != final_points || *cfg.top_level != final_top_level || cfg.environments != environments
        || cfg.facts != input.facts || cfg.holder_edges != input.holder_edges || cfg.holder_exits != input.holder_exits
        || cfg.literal_cfgs.iter().map(|child| child.reference_cfg.producer_owner.clone()).collect::<Vec<_>>() != input.children {
        return Err(fail(&cfg.producer_owner));
    }
    let augmented = SourceModuleTopologyAugmentation { connected: input.connected, original_environment_prefix: input.environments,
        suffix, suffix_environments, final_points, final_top_level };
    let SourceModuleResolverInputState::OriginalModuleInvocation(original) = &mut cfg.module_resolver_input else { unreachable!(); };
    original.topology = SourceModuleTopologyState::AugmentedOriginal(Box::new(augmented));
    Ok(())
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    let original = evidence(cfg)?;
    match &original.topology {
        SourceModuleTopologyState::AwaitingConnectedConstruction => {
            if !cfg.initial_points.is_empty() || cfg.name_resolution_context != NameResolutionContext::Building { return Err(fail(&cfg.producer_owner)); }
        },
        SourceModuleTopologyState::ConnectedConstruction(connected) => {
            connected.validate(original)?;
            if cfg.initial_points != connected.points || *cfg.top_level != connected.top_level
                || cfg.name_resolution_context != connected.canonical_root { return Err(fail(&cfg.producer_owner)); }
        },
        SourceModuleTopologyState::AugmentedOriginal(augmented) => {
            let (suffix, environments) = augmented.connected.expected_suffix(original, augmented.original_environment_prefix.len())?;
            let mut points = augmented.connected.points.clone();
            points.extend(suffix.clone());
            let mut top_level = augmented.connected.top_level.clone();
            top_level.points.extend(suffix.iter().map(|event| ScalarTopLevelPoint { point: event.id,
                execution: ScalarTopLevelPointExecution::SupplementalAnalysisOnly }));
            if augmented.suffix != suffix || augmented.suffix_environments != environments || augmented.final_points != points
                || augmented.final_top_level != top_level || cfg.initial_points != points || *cfg.top_level != top_level
                || cfg.name_resolution_context != augmented.connected.canonical_root { return Err(fail(&cfg.producer_owner)); }
            for (index, environment) in augmented.original_environment_prefix.iter().chain(&augmented.suffix_environments).enumerate() {
                let actual = cfg.environments.get(index).ok_or_else(|| fail(&cfg.producer_owner))?;
                if actual.literal != environment.literal || actual.instance != environment.instance { return Err(fail(&cfg.producer_owner)); }
            }
        },
    }
    Ok(())
}

pub(in crate::scalar) fn validate_published(cfg: &ScalarReferenceCfg) -> Result<(), ValueSchemaRemapError> {
    let error = || ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone());
    validate(cfg).map_err(|_| error())?;
    if !matches!(evidence(cfg).map_err(|_| error())?.topology, SourceModuleTopologyState::AugmentedOriginal(_)) { return Err(error()); }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceModuleOriginalStage {
    ConnectedConstruction,
    AugmentedOriginal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceModuleConstructorStageRef {
    pub invocation: SourceModuleResolverInputRef,
    pub stage: SourceModuleOriginalStage,
}

fn constructor_source(original: &ScalarReferenceCfg) -> Result<(SourceModuleConstructorStageRef, &SourceModuleConnectedConstruction), ReferenceAnalysisError> {
    validate(original)?;
    let input = evidence(original)?;
    let connected = match &input.topology {
        SourceModuleTopologyState::ConnectedConstruction(connected) => connected.as_ref(),
        SourceModuleTopologyState::AugmentedOriginal(augmented) => &augmented.connected,
        SourceModuleTopologyState::AwaitingConnectedConstruction => return Err(fail(&original.producer_owner)),
    };
    Ok((SourceModuleConstructorStageRef { invocation: connected.invocation.clone(), stage: SourceModuleOriginalStage::ConnectedConstruction }, connected))
}

pub(in crate::scalar) fn constructor_points(original: &ScalarReferenceCfg)
    -> Result<(SourceModuleConstructorStageRef, &[ReferenceFlowPoint]), ReferenceAnalysisError> {
    let (stage, connected) = constructor_source(original)?;
    Ok((stage, &connected.points))
}

pub(in crate::scalar) fn constructor_handoff<'a>(original: &'a ScalarReferenceCfg, parent: &ScalarReferenceCfg)
    -> Result<(SourceModuleConstructorStageRef, &'a [ReferenceFlowPoint]), ReferenceAnalysisError> {
    let (stage, connected) = constructor_source(original)?;
    if parent.producer_owner != original.producer_owner { return Err(fail(&original.producer_owner)); }
    validate(parent)?;
    let actual = evidence(parent)?;
    let SourceModuleTopologyState::ConnectedConstruction(parent_source) = &actual.topology else { return Err(fail(&original.producer_owner)); };
    if !actual.same_original_invocation(evidence(original)?) || parent_source.as_ref() != connected
        || parent.name_resolution_context != connected.canonical_root || parent.initial_points != connected.points
        || *parent.top_level != connected.top_level { return Err(fail(&original.producer_owner)); }
    Ok((stage, &connected.points))
}

pub(in crate::scalar) fn constructor_capture<'a>(original: &'a ScalarReferenceCfg, event: &SourceEventId)
    -> Result<(SourceModuleConstructorStageRef, &'a ReferenceFlowPoint), ReferenceAnalysisError> {
    let (stage, connected) = constructor_source(original)?;
    if event.owner != original.producer_owner { return Err(fail(&original.producer_owner)); }
    let point = connected.points.get(event.point.0).ok_or_else(|| fail(&original.producer_owner))?;
    if point.id != event.point || !matches!(point.kind, CfgPointKind::Capture { .. })
        || connected.capture_points.iter().filter(|row| row.point == event.point && row.role == ScalarCapturePointRole::Capture).count() != 1 {
        return Err(fail(&original.producer_owner));
    }
    Ok((stage, point))
}

#[cfg(test)]
mod stage_handoff_tests;

#[cfg(test)]
mod tests;
