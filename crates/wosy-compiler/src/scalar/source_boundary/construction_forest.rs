use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCaptureForestRoot {
    pub owner: ProducerCfgOwnerId,
    pub declaration: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCaptureConstructionForest {
    pub phase: SourceCaptureConstructionPhase,
    pub roots: Vec<SourceCaptureForestRoot>,
    pub sources: Vec<ValueTableSource>,
    pub pending: Vec<ProducerCfgOwnerId>,
}

fn validate_expression_originals(source: &SourceIdentity, top_level_cfg: &ScalarReferenceCfg, sources: &[ValueTableSource]) -> Result<(), ValueSchemaRemapError> {
    use reference::name_resolution::{NameResolutionContext, SourceNameResolutionContextRoot, SourceNameResolutionRoot};
    let module_owner = ProducerCfgOwnerId::Module(source.clone());
    let fail = || ValueSchemaRemapError::MissingProducer(module_owner.clone());
    if top_level_cfg.producer_owner != module_owner { return Err(fail()); }
    module_resolver_input::validate_endpoint(top_level_cfg)?;
    let SourceModuleResolverInputState::OriginalModuleInvocation(evidence) = &top_level_cfg.module_resolver_input else { return Err(fail()); };
    let expressions = evidence.expression_epoch(source).map_err(|_| fail())?;
    for row in sources {
        let original = row.resource_construction.original();
        let reject = || ValueSchemaRemapError::MissingProducer(row.producer.clone());
        let NameResolutionContext::Complete(context) = &original.name_resolution_context else { return Err(reject()); };
        if original.producer_owner != row.producer || context.owner != row.producer || context.expressions != expressions
            || row.policy_context != evidence.policy_context(&row.producer).map_err(|_| reject())?
            || sources.iter().filter(|other| other.producer == row.producer).count() != 1 { return Err(reject()); }
        match (&row.producer, &context.root) {
            (ProducerCfgOwnerId::Module(identity), SourceNameResolutionRoot::Module { .. }) => {
                let SourceModuleResolverInputState::OriginalModuleInvocation(retained) = &original.module_resolver_input else { return Err(reject()); };
                if identity != source || row.producer != module_owner || original.top_level.as_ref() != &row.top_level
                    || original.initial_points != row.initial_points || original.capture_points != row.capture_points
                    || original.name_resolution_context != top_level_cfg.name_resolution_context
                    || !evidence.same_original_invocation(retained) { return Err(reject()); }
                module_resolver_input::validate_endpoint(original)?;
            }
            (ProducerCfgOwnerId::Function { source: identity, .. }, SourceNameResolutionRoot::Named(root)) => {
                if identity != source || *root != evidence.named_root(&row.producer).map_err(|_| reject())? { return Err(reject()); }
            }
            (ProducerCfgOwnerId::Literal { .. }, SourceNameResolutionRoot::Literal { parent, occurrence, signature, parameters, parameter_spans, captures, body }) => {
                let parents = sources.iter().filter(|other| other.producer == parent.owner).collect::<Vec<_>>();
                let [parent_source] = parents.as_slice() else { return Err(reject()); };
                if occurrence.source != *source || context.parents != vec![parent.clone()] { return Err(reject()); }
                let NameResolutionContext::Complete(parent_context) = &parent_source.resource_construction.original().name_resolution_context else { return Err(reject()); };
                let mut ancestors = parent_context.ancestors.clone();
                ancestors.push(SourceNameResolutionContextRoot { owner: parent_context.owner.clone(), root: parent_context.root.clone(), globals: parent_context.globals.clone(), parents: parent_context.parents.clone() });
                if context.ancestors != ancestors { return Err(reject()); }
                let points = parent_source.initial_points.iter().enumerate().filter(|(_, point)| point.id == parent.point).collect::<Vec<_>>();
                let [(index, point)] = points.as_slice() else { return Err(reject()); };
                if *index != parent.point.0 { return Err(reject()); }
                let CfgPointKind::Capture { literal, environment, signature: actual_signature, parameters: actual_parameters, parameter_spans: actual_spans, captures: actual_captures, body: actual_body } = &point.kind else { return Err(reject()); };
                let declaration = reference::source_epoch::SourceCaptureDeclaration::new(
                    &parent_source.producer, &point.source_span, point.id, *literal, &environment.literal,
                    actual_signature, actual_captures, actual_parameters, actual_spans, actual_body,
                ).map_err(|_| reject())?;
                if point.source_span != *occurrence || declaration.event != *parent || declaration.literal_owner != row.producer
                    || declaration.literal_span != occurrence || environment.literal != *occurrence
                    || actual_signature != signature || actual_parameters != parameters || actual_spans != parameter_spans
                    || actual_captures != captures || actual_body != body { return Err(reject()); }
                let links = parent_source.children.iter().filter(|link| link.event == *parent || link.child == row.producer).collect::<Vec<_>>();
                let [link] = links.as_slice() else { return Err(reject()); };
                if link.event != *parent || link.child != row.producer { return Err(reject()); }
            }
            _ => return Err(reject()),
        }
        for ancestor in &context.ancestors {
            let ancestors = sources.iter().filter(|other| other.producer == ancestor.owner).collect::<Vec<_>>();
            let [ancestor_source] = ancestors.as_slice() else { return Err(reject()); };
            let NameResolutionContext::Complete(actual) = &ancestor_source.resource_construction.original().name_resolution_context else { return Err(reject()); };
            if actual.owner != ancestor.owner || actual.root != ancestor.root || actual.globals != ancestor.globals
                || actual.parents != ancestor.parents { return Err(reject()); }
        }
    }
    Ok(())
}

pub fn capture_construction_forest(source: &SourceIdentity, items: &[ScalarItem], top_level_cfg: &ScalarReferenceCfg, structs: &[ScalarStruct], enums: &[ScalarEnum], phase: SourceCaptureConstructionPhase, pending: &[ProducerCfgOwnerId]) -> Result<SourceCaptureConstructionForest, ValueSchemaRemapError> {
    fn function(source: &SourceIdentity, declaration: &ScalarFunction, structs: &[ScalarStruct], enums: &[ScalarEnum], forest: &mut SourceCaptureConstructionForest) -> Result<(), ValueSchemaRemapError> {
        let owner = declaration.reference_cfg.producer_owner.clone();
        if !matches!(&owner, ProducerCfgOwnerId::Function { source: actual, declaration: name, .. } if actual == source && *name == declaration.name_span) { return Err(ValueSchemaRemapError::MissingProducer(owner)); }
        forest.roots.push(SourceCaptureForestRoot { owner: owner.clone(), declaration: SourceSpan::new(source.clone(), declaration.span) });
        if forest.pending.contains(&owner) {
            if !declaration.reference_cfg.initial_points.is_empty() { return Err(ValueSchemaRemapError::MissingProducer(owner)); }
        } else {
            let generic_parameters = declaration.generic_parameters.iter().map(|row| row.name.clone()).collect::<BTreeSet<_>>();
            if generic_parameters.len() != declaration.generic_parameters.len() { return Err(ValueSchemaRemapError::MissingProducer(owner)); }
            let context = SourceCapturePolicyContext { structs: structs.to_vec(), enums: enums.to_vec(), generic_scope: owner, generic_declarations: declaration.generic_parameters.clone(), generic_parameters };
            forest.sources.extend(capture_table_sources(&declaration.reference_cfg, &context)?);
        }
        for arm in &declaration.overload_arms { function(source, arm, structs, enums, forest)?; }
        Ok(())
    }
    let owner = ProducerCfgOwnerId::Module(source.clone());
    if top_level_cfg.producer_owner != owner { return Err(ValueSchemaRemapError::MissingProducer(owner)); }
    if phase == SourceCaptureConstructionPhase::Complete { module_source_topology::validate_published(top_level_cfg)?; }
    let mut forest = SourceCaptureConstructionForest { phase, roots: vec![SourceCaptureForestRoot { owner: owner.clone(), declaration: SourceSpan::new(source.clone(), ByteSpan::new(0, 0)) }], sources: Vec::new(), pending: pending.to_vec() };
    if forest.pending.contains(&owner) {
        if !top_level_cfg.initial_points.is_empty() { return Err(ValueSchemaRemapError::MissingProducer(owner)); }
    } else {
        if top_level_cfg.initial_points.iter().filter(|row| matches!(row.kind, CfgPointKind::Entry) && row.source_span == SourceSpan::new(source.clone(), ByteSpan::new(0, 0))).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(owner)); }
        let context = SourceCapturePolicyContext { structs: structs.to_vec(), enums: enums.to_vec(), generic_scope: owner, generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() };
        forest.sources.extend(capture_table_sources(top_level_cfg, &context)?);
    }
    for item in items { if let ScalarItem::Function(declaration) = item { function(source, declaration, structs, enums, &mut forest)?; } }
    validate_expression_originals(source, top_level_cfg, &forest.sources)?;
    Ok(forest)
}

pub fn capture_root_construction_forest(root: &ScalarReferenceCfg, context: &SourceCapturePolicyContext) -> Result<SourceCaptureConstructionForest, ValueSchemaRemapError> {
    let sources = capture_table_sources(root, context)?;
    capture_source_tree_construction_forest(&root.producer_owner, &sources)
}

pub(in crate::scalar) fn capture_source_tree_construction_forest(owner: &ProducerCfgOwnerId, sources: &[ValueTableSource]) -> Result<SourceCaptureConstructionForest, ValueSchemaRemapError> {
    let roots = sources.iter().filter(|row| &row.producer == owner).collect::<Vec<_>>();
    let [root] = roots.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
    for source in sources {
        for point in &source.initial_points {
            if matches!(point.kind, CfgPointKind::Capture { .. }) && source.top_level.points.iter().any(|row| row.point == point.id && row.execution == ScalarTopLevelPointExecution::SupplementalAnalysisOnly) {
                return Err(ValueSchemaRemapError::MissingPoint { producer: source.producer.clone(), point: point.id });
            }
        }
    }
    let entries = root.initial_points.iter().filter(|row| matches!(row.kind, CfgPointKind::Entry)).collect::<Vec<_>>();
    let declaration = match entries.as_slice() {
        [entry] => entry.source_span.clone(),
        [] if matches!(root.producer, ProducerCfgOwnerId::Module(_)) => {
            let ProducerCfgOwnerId::Module(source) = &root.producer else { unreachable!(); };
            SourceSpan::new(source.clone(), ByteSpan::new(0, 0))
        }
        _ => return Err(ValueSchemaRemapError::MissingProducer(root.producer.clone())),
    };
    let phase = if sources.iter().all(|row| row.capture_inventory.phase == SourceCaptureConstructionPhase::Complete) { SourceCaptureConstructionPhase::Complete } else { SourceCaptureConstructionPhase::Building };
    Ok(SourceCaptureConstructionForest { phase, roots: vec![SourceCaptureForestRoot { owner: root.producer.clone(), declaration }], sources: sources.to_vec(), pending: Vec::new() })
}
