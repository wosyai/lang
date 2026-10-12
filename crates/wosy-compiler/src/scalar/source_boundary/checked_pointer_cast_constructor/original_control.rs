use super::*;
use module_source_topology::SourceModuleTopologyState;

pub(in crate::scalar) fn own_module_original<'a>(identity: &SourceIdentity, original: &module_resolver_input::SourceModuleResolverInputEvidence, forest: &'a SourceCaptureConstructionForest) -> Result<&'a ScalarReferenceCfg, ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: SourceSpan::new(identity.clone(), ByteSpan::new(0, 0)),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let owner = ProducerCfgOwnerId::Module(identity.clone());
    let rows = forest.sources.iter().filter(|row| row.producer == owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(fail()); };
    let donor = row.resource_construction.original();
    if donor.producer_owner != owner || row.initial_points != donor.initial_points
        || row.capture_points != donor.capture_points || row.top_level != *donor.top_level { return Err(fail()); }
    let SourceModuleResolverInputState::OriginalModuleInvocation(evidence) = &donor.module_resolver_input else { return Err(fail()); };
    module_resolver_input::validate_endpoint(donor).map_err(|_| fail())?;
    if !evidence.same_original_invocation(original) { return Err(fail()); }
    let NameResolutionContext::Complete(context) = &donor.name_resolution_context else { return Err(fail()); };
    if context.owner != owner || !matches!(context.root, reference::name_resolution::SourceNameResolutionRoot::Module { .. }) { return Err(fail()); }
    Ok(donor)
}
pub(in crate::scalar) fn own_function_original<'a>(identity: &SourceIdentity, owner: &ProducerCfgOwnerId, original: &module_resolver_input::SourceModuleResolverInputEvidence, forest: &'a SourceCaptureConstructionForest) -> Result<&'a ScalarReferenceCfg, ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: SourceSpan::new(identity.clone(), ByteSpan::new(0, 0)),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let ProducerCfgOwnerId::Function { source, .. } = owner else { return Err(fail()); };
    if *source != *identity { return Err(fail()); }
    let _root = original.named_root(owner).map_err(|_| fail())?;
    let rows = forest.sources.iter().filter(|row| row.producer == *owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(fail()); };
    let donor = row.resource_construction.original();
    if donor.producer_owner != *owner || row.initial_points != donor.initial_points
        || row.capture_points != donor.capture_points || row.top_level != *donor.top_level { return Err(fail()); }
    module_resolver_input::validate_endpoint(donor).map_err(|_| fail())?;
    let NameResolutionContext::Complete(context) = &donor.name_resolution_context else { return Err(fail()); };
    if context.owner != *owner || !matches!(context.root, reference::name_resolution::SourceNameResolutionRoot::Named(_)) { return Err(fail()); }
    Ok(donor)
}
pub(in crate::scalar) fn own_literal_chain<'a>(cfg: &ScalarReferenceCfg, original: &module_resolver_input::SourceModuleResolverInputEvidence, forest: &'a SourceCaptureConstructionForest) -> Result<&'a ScalarReferenceCfg, ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: cfg.initial_points[0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let ProducerCfgOwnerId::Literal { .. } = &cfg.producer_owner else { return Err(fail()); };
    let rows = forest.sources.iter().filter(|row| row.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(fail()); };
    let donor = row.resource_construction.original();
    if donor.producer_owner != cfg.producer_owner || row.initial_points != donor.initial_points
        || row.capture_points != donor.capture_points || row.top_level != *donor.top_level { return Err(fail()); }
    let mut visited = Vec::new();
    let mut current = cfg.producer_owner.clone();
    while let ProducerCfgOwnerId::Literal { enclosing, capture_point, literal } = current.clone() {
        if visited.contains(&current) { return Err(fail()); }
        visited.push(current.clone());
        if visited.len() > forest.sources.len() + 1 { return Err(fail()); }
        let parent_rows = forest.sources.iter().filter(|row| row.producer == *enclosing).collect::<Vec<_>>();
        let [parent_row] = parent_rows.as_slice() else { return Err(fail()); };
        let parent = parent_row.resource_construction.original();
        let literals = parent.literal_cfgs.iter().filter(|literal| literal.reference_cfg.producer_owner == current).collect::<Vec<_>>();
        let [literal_cfg] = literals.as_slice() else { return Err(fail()); };
        if literal_cfg.capture_point != capture_point { return Err(fail()); }
        let points = parent.initial_points.iter().filter(|point| point.id == capture_point).collect::<Vec<_>>();
        let [point] = points.as_slice() else { return Err(fail()); };
        let CfgPointKind::Capture { literal: span, environment, signature, captures, parameters, parameter_spans, body } = &point.kind else { return Err(fail()); };
        let declaration = reference::source_epoch::SourceCaptureDeclaration::new(
            &parent.producer_owner, &point.source_span, point.id, *span, &environment.literal,
            signature, captures, parameters, parameter_spans, body,
        ).map_err(|_| fail())?;
        if declaration.literal_owner != current || declaration.event.owner != parent.producer_owner
            || declaration.event.point != capture_point { return Err(fail()); }
        if literal != *declaration.literal_span { return Err(fail()); }
        parent_seed::validate_original(parent, literal_cfg).map_err(|_| fail())?;
        current = (*enclosing).clone();
    }
    match &current {
        ProducerCfgOwnerId::Module(source) => {
            own_module_original(source, original, forest)?;
        },
        ProducerCfgOwnerId::Function { .. } => {
            let source = match &current {
                ProducerCfgOwnerId::Function { source, .. } => source.clone(),
                _ => return Err(fail()),
            };
            own_function_original(&source, &current, original, forest)?;
        },
        ProducerCfgOwnerId::Literal { .. } => return Err(fail()),
    }
    Ok(donor)
}
pub(in crate::scalar) fn upstream_closure<'a>(cfg: &ScalarReferenceCfg, _original: &module_resolver_input::SourceModuleResolverInputEvidence, forest: &'a SourceCaptureConstructionForest) -> Result<&'a ScalarReferenceCfg, ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: cfg.initial_points[0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let rows = forest.sources.iter().filter(|row| row.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(fail()); };
    let donor = row.resource_construction.original();
    if donor.producer_owner != cfg.producer_owner { return Err(fail()); }
    if cfg.literal_cfgs != donor.literal_cfgs { return Err(fail()); }
    if cfg.evaluated_values.source_literal_constructor_seeds != donor.evaluated_values.source_literal_constructor_seeds
        || cfg.evaluated_values.source_capture_pending != donor.evaluated_values.source_capture_pending { return Err(fail()); }
    if let ProducerCfgOwnerId::Module(_) = &cfg.producer_owner {
        let SourceModuleResolverInputState::OriginalModuleInvocation(donor_input) = &donor.module_resolver_input else { return Err(fail()); };
        match &donor_input.topology {
            SourceModuleTopologyState::AugmentedOriginal(_) => {
                module_source_topology::validate_published(donor).map_err(|_| fail())?;
            },
            SourceModuleTopologyState::ConnectedConstruction(_) => {},
            SourceModuleTopologyState::AwaitingConnectedConstruction => return Err(fail()),
        }
    }
    Ok(donor)
}
pub(in crate::scalar) fn validate<'a>(cfg: &ScalarReferenceCfg, original: &module_resolver_input::SourceModuleResolverInputEvidence, forest: &'a SourceCaptureConstructionForest) -> Result<&'a [ReferenceFlowPoint], ReferenceAnalysisError> {
    let fail = || ReferenceAnalysisError::MissingContext {
        source_span: cfg.initial_points[0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let rows = forest.sources.iter().filter(|row| row.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(fail()); };
    let donor = row.resource_construction.original();
    if donor.producer_owner != cfg.producer_owner || row.initial_points != donor.initial_points
        || row.capture_points != donor.capture_points || row.top_level != *donor.top_level
        || cfg.name_resolution_context != donor.name_resolution_context { return Err(fail()); }
    if cfg.while_points != donor.while_points || cfg.block_scope_exits != donor.block_scope_exits
        || cfg.conditional_points != donor.conditional_points || cfg.call_points != donor.call_points
        || cfg.capture_points != donor.capture_points || cfg.read_points != donor.read_points
        || cfg.binding_points != donor.binding_points || cfg.assignment_points != donor.assignment_points {
        return Err(fail());
    }
    if cfg.evaluated_values.productions != donor.evaluated_values.productions
        || cfg.evaluated_values.call_argument_inputs != donor.evaluated_values.call_argument_inputs
        || cfg.evaluated_values.raw_address_inputs != donor.evaluated_values.raw_address_inputs
        || cfg.evaluated_values.raw_address_productions != donor.evaluated_values.raw_address_productions
        || row.tables.productions != donor.evaluated_values.productions
        || row.tables.call_argument_inputs != donor.evaluated_values.call_argument_inputs
        || row.tables.raw_address_inputs != donor.evaluated_values.raw_address_inputs
        || row.tables.raw_address_productions != donor.evaluated_values.raw_address_productions {
        return Err(fail());
    }
    match &cfg.producer_owner {
        ProducerCfgOwnerId::Function { .. } | ProducerCfgOwnerId::Literal { .. } => {
            if cfg.initial_points != donor.initial_points { return Err(fail()); }
            Ok(donor.initial_points.as_slice())
        },
        ProducerCfgOwnerId::Module(_) => {
            let SourceModuleResolverInputState::OriginalModuleInvocation(candidate_input) = &cfg.module_resolver_input else { return Err(fail()); };
            let SourceModuleResolverInputState::OriginalModuleInvocation(donor_input) = &donor.module_resolver_input else { return Err(fail()); };
            module_source_topology::validate(cfg).map_err(|_| fail())?;
            module_source_topology::validate(donor).map_err(|_| fail())?;
            if !candidate_input.same_original_invocation(donor_input) || !candidate_input.same_original_invocation(original)
                || !donor_input.same_original_invocation(original) { return Err(fail()); }
            let (candidate_stage, candidate_points) = module_source_topology::constructor_points(cfg).map_err(|_| fail())?;
            let (donor_stage, donor_points) = module_source_topology::constructor_points(donor).map_err(|_| fail())?;
            if candidate_stage != donor_stage || candidate_points != donor_points { return Err(fail()); }
            match (&candidate_input.topology, &donor_input.topology) {
                (SourceModuleTopologyState::ConnectedConstruction(_), SourceModuleTopologyState::ConnectedConstruction(_))
                    | (SourceModuleTopologyState::AugmentedOriginal(_), SourceModuleTopologyState::AugmentedOriginal(_)) => {
                    if cfg.initial_points != donor.initial_points || cfg.top_level != donor.top_level { return Err(fail()); }
                },
                (SourceModuleTopologyState::ConnectedConstruction(_), SourceModuleTopologyState::AugmentedOriginal(_)) => {
                    let (handoff_stage, handoff_points) = module_source_topology::constructor_handoff(donor, cfg).map_err(|_| fail())?;
                    if handoff_stage != donor_stage || handoff_points != donor_points { return Err(fail()); }
                },
                (SourceModuleTopologyState::AugmentedOriginal(_), SourceModuleTopologyState::ConnectedConstruction(_))
                    | (SourceModuleTopologyState::AwaitingConnectedConstruction, _)
                    | (_, SourceModuleTopologyState::AwaitingConnectedConstruction) => return Err(fail()),
            }
            Ok(donor_points)
        },
    }
}
