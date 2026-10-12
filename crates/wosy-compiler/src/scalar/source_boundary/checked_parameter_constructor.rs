use super::*;
use super::null_constructor::SourceConstructorOriginDerivation;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceCheckedParameterConstructorRef {
    pub origin: SourceConstructorOriginRef,
    pub event: SourceEventId,
    pub declaration: SourceNameDeclarationRef,
    pub binding: ReferenceBindingId,
}

pub(in crate::scalar) fn derivation(owner: &ProducerCfgOwnerId, point: CfgPointId, origin: ReferenceOriginId, ordinal: usize, binding: ReferenceBindingId) -> SourceConstructorOriginDerivation {
    SourceConstructorOriginDerivation::CheckedParameter(SourceCheckedParameterConstructorRef {
        origin: SourceConstructorOriginRef { owner: owner.clone(), origin },
        event: SourceEventId { owner: owner.clone(), point },
        declaration: SourceNameDeclarationRef { owner: owner.clone(), site: SourceNameDeclarationSite::Parameter { ordinal } }, binding,
    })
}

fn identity(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => identity(enclosing),
    }
}

fn invalid(cfg: &ScalarReferenceCfg) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext { source_span: cfg.initial_points[0].source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }
}

pub(in crate::scalar) fn expected(cfg: &ScalarReferenceCfg) -> Result<Vec<SourceCheckedParameterConstructorRef>, ReferenceAnalysisError> {
    let fail = || invalid(cfg);
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(fail()); };
    if context.owner != cfg.producer_owner { return Err(fail()); }
    let (signature, names, spans, function_span, block_span) = match &context.root {
        SourceNameResolutionRoot::Named(root) => (&root.signature, &root.parameters, &root.parameter_spans, root.span, root.body.span),
        SourceNameResolutionRoot::Literal { signature, parameters, parameter_spans, occurrence, body, .. } => (signature, parameters, parameter_spans, occurrence.range, body.span),
        SourceNameResolutionRoot::Module { .. } => {
            if !cfg.reference_parameters.is_empty() || cfg.checked_installs.iter().any(|row| matches!(row.input, ReferenceCheckedInstallInput::Parameter { .. })) { return Err(fail()); }
            return Ok(Vec::new());
        },
    };
    let ScalarType::Callable { parameters, .. } = signature else { return Err(fail()); };
    if parameters.len() != names.len() || parameters.len() != spans.len() { return Err(fail()); }
    let source = identity(&context.owner);
    let bindings = spans.iter().map(|span| ReferenceBindingId { source: source.clone(), function_span, declaration_span: *span, block_span, kind: ReferenceBindingKind::Declared }).collect::<Vec<_>>();
    if cfg.reference_parameters != bindings { return Err(fail()); }
    let entries = cfg.initial_points.iter().filter(|row| matches!(row.kind, CfgPointKind::Entry)).collect::<Vec<_>>();
    let [entry] = entries.as_slice() else { return Err(fail()); };
    if entry.scope != ReferenceScopeId(0) || entry.source_span != SourceSpan::new(source.clone(), function_span) { return Err(fail()); }
    let mut expected = Vec::new();
    let mut installs = Vec::new();
    for (ordinal, ((binding, name), ty)) in bindings.iter().zip(names).zip(parameters).enumerate() {
        let ScalarType::CheckedReference { mutability, .. } = ty else { continue; };
        let mut referent = binding.clone();
        referent.kind = ReferenceBindingKind::BorrowedReferent { parameter: ordinal };
        let definition = ReferenceOrigin::BorrowedFrom {
            parameter: ordinal,
            loan: ReferenceLoanId {
                mode: *mutability,
                origin_place: ReferencePlaceId { source: source.clone(), function_span, declaration_span: binding.declaration_span, block_span,
                    binding: referent, projections: Vec::new(), place: ScalarPlace::Name { name: name.clone(), span: binding.declaration_span } },
                creation_source: source.clone(), creation_span: binding.declaration_span, parent: None,
            },
        };
        let Some(index) = cfg.facts.iter().position(|row| *row == definition) else { return Err(fail()); };
        let origin = ReferenceOriginId(index);
        let points = cfg.initial_points.iter().filter(|row| matches!(&row.kind, CfgPointKind::Bind { binding: actual } if actual == binding)).collect::<Vec<_>>();
        let [point] = points.as_slice() else { return Err(fail()); };
        if cfg.initial_points.get(point.id.0) != Some(*point) || point.id.0 <= entry.id.0 || point.scope != entry.scope
            || point.source_span != SourceSpan::new(source.clone(), binding.declaration_span)
            || point.possible_origins != Some(BTreeSet::from([origin]))
            || cfg.initial_points.iter().any(|row| row.id.0 < point.id.0 && matches!(row.kind, CfgPointKind::ScopeEnter)) { return Err(fail()); }
        installs.push(ReferenceCheckedInstall { point: point.id, input: ReferenceCheckedInstallInput::Parameter { binding: binding.clone(), origin } });
        expected.push(SourceCheckedParameterConstructorRef { origin: SourceConstructorOriginRef { owner: context.owner.clone(), origin },
            event: SourceEventId { owner: context.owner.clone(), point: point.id },
            declaration: SourceNameDeclarationRef { owner: context.owner.clone(), site: SourceNameDeclarationSite::Parameter { ordinal } }, binding: binding.clone() });
    }
    if !expected.windows(2).all(|pair| pair[0].event.point < pair[1].event.point)
        || installs != cfg.checked_installs.iter().filter(|row| matches!(row.input, ReferenceCheckedInstallInput::Parameter { .. })).cloned().collect::<Vec<_>>() { return Err(fail()); }
    Ok(expected)
}

pub(in crate::scalar) fn required(cfg: &ScalarReferenceCfg, nulls: &[SourceNullConstructor], structs: &[ScalarStruct]) -> Result<Vec<SourceConstructorOriginDerivation>, ReferenceAnalysisError> {
    let mut required = nulls.iter().map(SourceNullConstructor::derivation)
        .chain(expected(cfg)?.into_iter().map(SourceConstructorOriginDerivation::CheckedParameter))
        .chain(super::direct_storage_constructor::expected(cfg, structs)?.into_iter().map(SourceConstructorOriginDerivation::DirectStorageAddress))
        .chain(super::allocation_identity_constructor::expected(cfg, structs)?.into_iter().map(SourceConstructorOriginDerivation::AllocationIdentityAddress)).collect::<Vec<_>>();
    required.sort_by_key(|row| row.event().point);
    Ok(required)
}

pub(in crate::scalar) fn validate_mixed(cfg: &ScalarReferenceCfg, nulls: &[SourceNullConstructor], structs: &[ScalarStruct]) -> Result<(), ReferenceAnalysisError> {
    let required = required(cfg, nulls, structs)?;
    for kind in 0..4 {
        let family = |row: &&SourceConstructorOriginDerivation| match row {
            SourceConstructorOriginDerivation::NullExpression(_) => kind == 0,
            SourceConstructorOriginDerivation::CheckedParameter(_) => kind == 1,
            SourceConstructorOriginDerivation::DirectStorageAddress(_) => kind == 2,
            SourceConstructorOriginDerivation::AllocationIdentityAddress(_) => kind == 3,
        };
        if required.iter().filter(family).collect::<Vec<_>>() != cfg.evaluated_values.source_origin_derivations.iter().filter(family).collect::<Vec<_>>() { return Err(invalid(cfg)); }
    }
    if required != cfg.evaluated_values.source_origin_derivations { return Err(invalid(cfg)); }
    Ok(())
}

#[cfg(test)]
pub(in crate::scalar) mod tests;
