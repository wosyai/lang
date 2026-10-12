use super::*;

fn invalid(source: &SourceIdentity, span: ByteSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext { source_span: SourceSpan::new(source.clone(), span), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }
}

fn source(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => source(enclosing),
    }
}

pub(in crate::scalar::reference) fn global_declarations(source: &SourceIdentity, items: &[ScalarItem]) -> Vec<SourceNameGlobalDeclaration> {
    items.iter().filter_map(|item| match item {
        ScalarItem::Binding(binding) => Some(SourceNameGlobalDeclaration { owner: ProducerCfgOwnerId::Module(source.clone()), binding: binding.clone() }),
        _ => None,
    }).collect()
}

pub(in crate::scalar::reference) fn module_root(source: &SourceIdentity, items: &[ScalarItem], top_level: &ScalarTopLevelCorrespondence) -> SourceNameResolutionContextRoot {
    SourceNameResolutionContextRoot {
        owner: ProducerCfgOwnerId::Module(source.clone()),
        root: SourceNameResolutionRoot::Module {
            items: items.iter().map(|item| match item {
                ScalarItem::Binding(binding) => SourceNameModuleItem::Binding(binding.clone()),
                ScalarItem::Executable(item) => SourceNameModuleItem::Executable(item.clone()),
                ScalarItem::Function(function) => SourceNameModuleItem::Function(SourceNameFunctionDeclaration {
                    owner: function.reference_cfg.producer_owner.clone(), name: function.name.clone(), name_span: function.name_span,
                    span: function.span, signature: function.signature.clone(),
                }),
                ScalarItem::Namespace(namespace) => SourceNameModuleItem::Namespace(namespace.clone()),
                ScalarItem::Extern(external) => SourceNameModuleItem::Extern(external.clone()),
            }).collect(),
            top_level: top_level.clone(),
        },
        globals: global_declarations(source, items), parents: Vec::new(),
    }
}

fn function_root(function: &ScalarFunction) -> SourceNameResolutionRoot {
    SourceNameResolutionRoot::Named(SourceNameFunctionRoot {
        name: function.name.clone(), name_span: function.name_span, span: function.span,
        signature: function.signature.clone(), parameters: function.parameters.clone(),
        parameter_spans: function.parameter_spans.clone(), generic_declarations: function.generic_parameters.clone(), body: function.body.clone(),
    })
}

fn original(context: &SourceNameResolutionContext) -> SourceNameResolutionContextRoot {
    SourceNameResolutionContextRoot { owner: context.owner.clone(), root: context.root.clone(), globals: context.globals.clone(), parents: context.parents.clone() }
}

fn globals_match(globals: &[SourceNameGlobalDeclaration], actual: &BTreeMap<String, (ReferenceBindingId, ScalarType)>, span: ByteSpan, identity: &SourceIdentity) -> Result<(), ReferenceAnalysisError> {
    let mut expected = BTreeMap::new();
    for declaration in globals {
        let ProducerCfgOwnerId::Module(source) = &declaration.owner else { return Err(invalid(identity, span)); };
        let binding = ReferenceBindingId { source: source.clone(), function_span: ByteSpan::new(0, 0), declaration_span: declaration.binding.name_span,
            block_span: ByteSpan::new(0, 0), kind: ReferenceBindingKind::Declared };
        if expected.insert(declaration.binding.name.clone(), (binding, declaration.binding.declared_type.clone())).is_some() { return Err(invalid(identity, span)); }
    }
    if expected != *actual { return Err(invalid(identity, span)); }
    Ok(())
}

fn module_names(originals: &[SourceNameResolutionContextRoot]) -> HashSet<String> {
    originals.iter().filter_map(|root| match &root.root { SourceNameResolutionRoot::Module { items, .. } => Some(items), _ => None })
        .flatten().map(|item| match item {
            SourceNameModuleItem::Binding(binding) => Some(binding.name.clone()),
            SourceNameModuleItem::Function(function) => Some(function.name.clone()),
            SourceNameModuleItem::Namespace(namespace) => Some(namespace.binding.clone()),
            SourceNameModuleItem::Extern(external) => Some(external.binding.clone()),
            SourceNameModuleItem::Executable(_) => None,
        }).flatten().collect()
}

fn validate_roots(context: &SourceNameResolutionContext, span: ByteSpan) -> Result<(), ReferenceAnalysisError> {
    let identity = source(&context.owner);
    if context.ancestors.iter().any(|root| root.owner == context.owner)
        || context.ancestors.iter().enumerate().any(|(index, root)| context.ancestors[..index].iter().any(|other| other.owner == root.owner)) {
        return Err(invalid(identity, span));
    }
    for root in &context.ancestors {
        if root.parents.iter().any(|parent| parent.owner != context.owner && !context.ancestors.iter().any(|other| other.owner == parent.owner)) {
            return Err(invalid(identity, span));
        }
    }
    if context.parents.iter().any(|parent| !context.ancestors.iter().any(|root| root.owner == parent.owner)) { return Err(invalid(identity, span)); }
    Ok(())
}

pub(in crate::scalar::reference) fn named_context(identity: &SourceIdentity, function: &ScalarFunction, names: &HashSet<String>, actual_globals: &BTreeMap<String, (ReferenceBindingId, ScalarType)>, globals: &[SourceNameGlobalDeclaration], originals: &[SourceNameResolutionContextRoot], expressions: &[ResolvedExpressionOutputFact]) -> Result<NameResolutionContext, ReferenceAnalysisError> {
    if source(&function.reference_cfg.producer_owner) != identity || module_names(originals) != *names { return Err(invalid(identity, function.span)); }
    globals_match(globals, actual_globals, function.span, identity)?;
    let context = SourceNameResolutionContext { owner: function.reference_cfg.producer_owner.clone(), expressions: expressions.to_vec(), root: function_root(function), globals: globals.to_vec(), parents: Vec::new(), ancestors: originals.to_vec() };
    validate_roots(&context, function.span)?;
    Ok(NameResolutionContext::Complete(Box::new(context)))
}

pub(in crate::scalar::reference) fn module_context(identity: &SourceIdentity, items: &[ScalarItem], top_level: &ScalarTopLevelCorrespondence, expressions: &[ResolvedExpressionOutputFact]) -> NameResolutionContext {
    let root = module_root(identity, items, top_level);
    NameResolutionContext::Complete(Box::new(SourceNameResolutionContext { owner: root.owner, expressions: expressions.to_vec(), root: root.root, globals: root.globals, parents: root.parents, ancestors: Vec::new() }))
}

pub(in crate::scalar::reference) fn literal_context(parent: &ScalarReferenceCfg, declaration: &source_epoch::SourceCaptureDeclaration<'_>, actual_globals: &BTreeMap<String, (ReferenceBindingId, ScalarType)>, expressions: &[ResolvedExpressionOutputFact]) -> Result<NameResolutionContext, ReferenceAnalysisError> {
    let NameResolutionContext::Complete(context) = &parent.name_resolution_context else { return Err(invalid(&declaration.literal_span.source, declaration.literal_span.range)); };
    if context.owner != parent.producer_owner || declaration.event.owner != parent.producer_owner { return Err(invalid(&declaration.literal_span.source, declaration.literal_span.range)); }
    let mut ancestors = context.ancestors.clone();
    ancestors.push(original(context));
    let globals = context.globals.iter().filter(|global| actual_globals.contains_key(&global.binding.name)).cloned().collect::<Vec<_>>();
    globals_match(&globals, actual_globals, declaration.literal_span.range, &declaration.literal_span.source)?;
    let context = SourceNameResolutionContext {
        owner: declaration.literal_owner.clone(),
        expressions: expressions.to_vec(),
        root: SourceNameResolutionRoot::Literal { parent: declaration.event.clone(), occurrence: declaration.literal_span.clone(), signature: declaration.signature.clone(),
            parameters: declaration.parameters.iter().map(|parameter| parameter.name.clone()).collect(),
            parameter_spans: declaration.parameters.iter().map(|parameter| parameter.binding.declaration_span).collect(),
            captures: declaration.captures.to_vec(), body: declaration.body.clone() },
        globals, parents: vec![declaration.event.clone()], ancestors,
    };
    validate_roots(&context, declaration.literal_span.range)?;
    Ok(NameResolutionContext::Complete(Box::new(context)))
}

pub(in crate::scalar::reference) fn validate_named_source(cfg: &ScalarReferenceCfg, identity: &SourceIdentity, function: &ScalarFunction, names: &HashSet<String>, actual_globals: &BTreeMap<String, (ReferenceBindingId, ScalarType)>, globals: &[SourceNameGlobalDeclaration], originals: &[SourceNameResolutionContextRoot], expressions: &[ResolvedExpressionOutputFact]) -> Result<(), ReferenceAnalysisError> {
    let expected = named_context(identity, function, names, actual_globals, globals, originals, expressions)?;
    if cfg.name_resolution_context != expected || cfg.producer_owner != function.reference_cfg.producer_owner { return Err(invalid(identity, function.span)); }
    Ok(())
}

pub(in crate::scalar::reference) fn complete_module_source(cfg: &mut ScalarReferenceCfg, identity: &SourceIdentity, items: &[ScalarItem]) -> Result<(), ReferenceAnalysisError> {
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(invalid(identity, ByteSpan::new(0, 0))); };
    let SourceNameResolutionRoot::Module { items: retained, .. } = &context.root else { return Err(invalid(identity, ByteSpan::new(0, 0))); };
    let source_boundary::SourceModuleResolverInputState::OriginalModuleInvocation(evidence) = &cfg.module_resolver_input else { return Err(invalid(identity, ByteSpan::new(0, 0))); };
    let expressions = evidence.expression_epoch(identity)?;
    let expected = module_context(identity, items, &cfg.top_level, expressions);
    let NameResolutionContext::Complete(expected_context) = &expected else { unreachable!(); };
    let SourceNameResolutionRoot::Module { items: original_items, .. } = &expected_context.root else { unreachable!(); };
    if retained != original_items || context.owner != cfg.producer_owner || context.globals != expected_context.globals
        || context.parents != expected_context.parents || context.ancestors != expected_context.ancestors
        || context.expressions != expected_context.expressions { return Err(invalid(identity, ByteSpan::new(0, 0))); }
    cfg.name_resolution_context = expected;
    Ok(())
}

pub(in crate::scalar::reference) fn canonical_module_root(
    cfg: &ScalarReferenceCfg,
    identity: &SourceIdentity,
    items: &[ScalarItem],
) -> Result<SourceNameResolutionContextRoot, ReferenceAnalysisError> {
    let fail = || invalid(identity, ByteSpan::new(0, 0));
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(fail()); };
    let SourceNameResolutionRoot::Module { top_level: connected, .. } = &context.root else { return Err(fail()); };
    let source_boundary::SourceModuleResolverInputState::OriginalModuleInvocation(evidence) = &cfg.module_resolver_input else { return Err(fail()); };
    let expressions = evidence.expression_epoch(identity)?;
    if cfg.producer_owner != ProducerCfgOwnerId::Module(identity.clone())
        || cfg.name_resolution_context != module_context(identity, items, connected, expressions)
        || cfg.top_level.bindings != connected.bindings || cfg.top_level.assignments != connected.assignments
        || cfg.top_level.points.get(..connected.points.len()) != Some(connected.points.as_slice()) {
        return Err(fail());
    }
    for event in &connected.points {
        let ScalarTopLevelPointExecution::Connected { outgoing, .. } = &event.execution else { return Err(fail()); };
        let point = cfg.initial_points.get(event.point.0).ok_or_else(fail)?;
        if point.id != event.point || point.source_span.source != *identity || point.successors != *outgoing { return Err(fail()); }
    }
    Ok(original(context))
}

pub(in crate::scalar::reference) fn validate_literal_source(cfg: &ScalarReferenceCfg, parent: &ScalarReferenceCfg, declaration: &source_epoch::SourceCaptureDeclaration<'_>, actual_globals: &BTreeMap<String, (ReferenceBindingId, ScalarType)>, expressions: &[ResolvedExpressionOutputFact]) -> Result<(), ReferenceAnalysisError> {
    if cfg.producer_owner != declaration.literal_owner || cfg.name_resolution_context != literal_context(parent, declaration, actual_globals, expressions)? {
        return Err(invalid(&declaration.literal_span.source, declaration.literal_span.range));
    }
    Ok(())
}
