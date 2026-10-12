use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceConstructorOriginRef {
    pub owner: ProducerCfgOwnerId,
    pub origin: ReferenceOriginId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceParentSeedCaptureUse {
    pub capture_ordinal: usize,
    pub binding: ReferenceBindingId,
    pub input_ordinal: usize,
    pub origin: SourceConstructorOriginRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceParentSeedOrigin {
    pub seed: SourceLiteralConstructorSeedRef,
    pub parent_source: SourceLiteralSeedParentSource,
    pub parent: SourceConstructorOriginRef,
    pub child: SourceConstructorOriginRef,
    pub definition: ReferenceOrigin,
    pub capture_uses: Vec<SourceParentSeedCaptureUse>,
}

pub(in crate::scalar) fn derive(
    seed: &SourceLiteralConstructorSeedRef,
    parent_source: &SourceLiteralSeedParentSource,
    environment: &CallableEnvironmentId,
    captures: &[closure::ClosureCapture],
    inputs: &[CallableCaptureInput],
    facts: &[ReferenceOrigin],
) -> Result<Vec<SourceParentSeedOrigin>, ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: environment.literal.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let ProducerCfgOwnerId::Literal { enclosing, .. } = &seed.child else { return Err(fail()); };
    if enclosing.as_ref() != &seed.event.owner { return Err(fail()); }
    match (&seed.event.owner, parent_source) {
        (ProducerCfgOwnerId::Module(_), SourceLiteralSeedParentSource::Module(stage))
            if stage.invocation.owner == seed.event.owner && stage.stage == module_source_topology::SourceModuleOriginalStage::ConnectedConstruction => {},
        (ProducerCfgOwnerId::Function { .. } | ProducerCfgOwnerId::Literal { .. }, SourceLiteralSeedParentSource::Callable(owner))
            if owner == &seed.event.owner => {},
        _ => return Err(fail()),
    }
    for input in inputs {
        if !captures.iter().any(|capture| capture.binding == input.binding)
            || input.origins.iter().any(|origin| origin.0 >= facts.len()) { return Err(fail()); }
    }
    Ok(facts.iter().enumerate().map(|(index, definition)| {
        let parent = SourceConstructorOriginRef { owner: seed.event.owner.clone(), origin: ReferenceOriginId(index) };
        let mut capture_uses = Vec::new();
        for (capture_ordinal, capture) in captures.iter().enumerate() {
            for (input_ordinal, input) in inputs.iter().enumerate() {
                if input.binding == capture.binding && input.origins.contains(&parent.origin) {
                    capture_uses.push(SourceParentSeedCaptureUse {
                        capture_ordinal, binding: capture.binding.clone(), input_ordinal, origin: parent.clone(),
                    });
                }
            }
        }
        SourceParentSeedOrigin {
            seed: seed.clone(), parent_source: parent_source.clone(), parent,
            child: SourceConstructorOriginRef { owner: seed.child.clone(), origin: ReferenceOriginId(index) },
            definition: definition.clone(), capture_uses,
        }
    }).collect())
}

pub(in crate::scalar) fn validate_original(parent: &ScalarReferenceCfg, literal: &ScalarLiteralReferenceCfg) -> Result<(), ValueSchemaRemapError> {
    let fail = || ValueSchemaRemapError::MissingPoint { producer: parent.producer_owner.clone(), point: literal.capture_point };
    let seed = &literal.constructor_seed;
    if seed.id.event.owner != parent.producer_owner || seed.id.event.point != literal.capture_point
        || seed.id.child != literal.reference_cfg.producer_owner { return Err(fail()); }
    let point = parent.initial_points.get(literal.capture_point.0).ok_or_else(fail)?;
    let CfgPointKind::Capture { environment, captures, .. } = &point.kind else { return Err(fail()); };
    let source = if matches!(parent.producer_owner, ProducerCfgOwnerId::Module(_)) {
        let (stage, _) = module_source_topology::constructor_capture(parent, &seed.id.event).map_err(|_| fail())?;
        SourceLiteralSeedParentSource::Module(stage)
    } else { SourceLiteralSeedParentSource::Callable(parent.producer_owner.clone()) };
    if seed.parent_source != source { return Err(fail()); }
    let expected = derive(&seed.id, &source, environment, captures, &seed.inputs, &seed.inherited_facts).map_err(|_| fail())?;
    if seed.parent_seed_origins != expected || !literal.reference_cfg.facts.starts_with(&seed.inherited_facts) { return Err(fail()); }
    Ok(())
}

#[cfg(test)]
mod tests;
