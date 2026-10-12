use super::*;

pub(in crate::scalar::reference) fn freeze(
    parent: &ScalarReferenceCfg,
    source: &source_boundary::SourceCapturePreflightInput,
    transfer: &initial_transfer::InitialTransferReceipt,
    point: &ReferenceFlowPoint,
    child: &ProducerCfgOwnerId,
) -> Result<source_boundary::SourceLiteralConstructorSeed, ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: point.source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let CfgPointKind::Capture { environment, captures, .. } = &point.kind else { return Err(fail()); };
    let endpoints = source.construction_forest.sources.iter().filter(|row| row.producer == parent.producer_owner).collect::<Vec<_>>();
    let [endpoint] = endpoints.as_slice() else { return Err(fail()); };
    let original = endpoint.resource_construction.original();
    let parent_source = if matches!(parent.producer_owner, ProducerCfgOwnerId::Module(_)) {
        let (stage, _) = source_boundary::module_source_topology::constructor_handoff(original, parent)?;
        let event = SourceEventId { owner: parent.producer_owner.clone(), point: point.id };
        source_boundary::module_source_topology::constructor_capture(original, &event)?;
        source_boundary::SourceLiteralSeedParentSource::Module(stage)
    } else {
        if original.initial_points.get(point.id.0) != Some(point) { return Err(fail()); }
        source_boundary::SourceLiteralSeedParentSource::Callable(original.producer_owner.clone())
    };
    let initial_facts = initial_transfer::original_arena(parent, source, transfer)?;
    let captured = parent.environments.get(environment.index).ok_or_else(fail)?;
    if captured.literal != environment.literal { return Err(fail()); }
    for input in &captured.inputs {
        for origin in &input.origins {
            if parent.facts.get(origin.0).is_none() { return Err(fail()); }
        }
    }
    let id = source_boundary::SourceLiteralConstructorSeedRef {
        event: SourceEventId { owner: parent.producer_owner.clone(), point: point.id }, child: child.clone(),
    };
    let inputs = captured.inputs.clone();
    let inherited_facts = parent.facts.clone();
    let parent_seed_origins = source_boundary::parent_seed::derive(&id, &parent_source, environment, captures, &inputs, &inherited_facts)?;
    Ok(source_boundary::SourceLiteralConstructorSeed {
        id, parent_source, environment: environment.clone(), captures: captures.clone(), inputs,
        initial_facts, inherited_facts, parent_seed_origins,
        borrow_creations: transfer.borrows.iter().map(|receipt| source_boundary::SourceLiteralSeedBorrowCreation {
            event: receipt.event.clone(), source_span: receipt.source_span.clone(), scope: receipt.scope,
            request: receipt.request.clone(), input: receipt.input.clone(),
            definitions: receipt.definitions.iter().map(|row| (row.id.origin, row.definition.clone())).collect(),
            realized: receipt.realized.clone(), applied: receipt.applied.clone(),
        }).collect(),
    })
}

pub(in crate::scalar::reference) fn publish(
    parent: &mut ScalarReferenceCfg,
    children: Vec<ScalarLiteralReferenceCfg>,
    errors: Vec<ReferenceAnalysisError>,
) -> Vec<ReferenceAnalysisError> {
    if errors.is_empty() {
        let seeds = children.iter().map(|child| child.constructor_seed.id.clone()).collect();
        parent.evaluated_values.source_literal_constructor_seeds = seeds;
        parent.literal_cfgs = children;
    }
    errors
}

#[cfg(test)]
mod atomic_tests;

#[cfg(test)]
mod tests;
