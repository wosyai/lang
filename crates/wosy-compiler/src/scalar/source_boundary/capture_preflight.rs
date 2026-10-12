use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar) struct SourceCapturePreflightInput {
    pub policy_context: SourceCapturePolicyContext,
    pub construction_forest: SourceCaptureConstructionForest,
    pub substitutions: BTreeMap<String, ScalarType>,
}

pub(in crate::scalar) fn preflight_input(
    owner: &ProducerCfgOwnerId,
    forest: &SourceCaptureConstructionForest,
    substitutions: BTreeMap<String, ScalarType>,
) -> Result<SourceCapturePreflightInput, ReferenceAnalysisError> {
    let rows = forest.sources.iter().filter(|row| row.producer == *owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        let roots = forest.roots.iter().filter(|row| row.owner == *owner).collect::<Vec<_>>();
        let [root] = roots.as_slice() else { panic!("canonical preflight owner must be an actual required constructor"); };
        return Err(invalid(&root.declaration));
    };
    Ok(SourceCapturePreflightInput { policy_context: row.policy_context.clone(), construction_forest: forest.clone(), substitutions })
}

pub(in crate::scalar) fn local_preflight_input(
    cfg: &ScalarReferenceCfg,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    generic_parameters: &BTreeSet<String>,
    generic_declarations: &[ScalarGenericParameter],
) -> Result<SourceCapturePreflightInput, ReferenceAnalysisError> {
    let mut scope = &cfg.producer_owner;
    while let ProducerCfgOwnerId::Literal { enclosing, .. } = scope { scope = enclosing; }
    let context = SourceCapturePolicyContext { structs: structs.to_vec(), enums: enums.to_vec(), generic_scope: scope.clone(), generic_declarations: generic_declarations.to_vec(), generic_parameters: generic_parameters.clone() };
    let forest = capture_root_construction_forest(cfg, &context).map_err(|_| invalid(&cfg.initial_points[0].source_span))?;
    preflight_input(&cfg.producer_owner, &forest, BTreeMap::new())
}

pub(in crate::scalar) fn local_preflight_input_with_ancestor(
    cfg: &ScalarReferenceCfg,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    generic_parameters: &BTreeSet<String>,
    generic_declarations: &[ScalarGenericParameter],
    ancestor: &SourceCapturePreflightInput,
) -> Result<SourceCapturePreflightInput, ReferenceAnalysisError> {
    let input = local_preflight_input(cfg, structs, enums, generic_parameters, generic_declarations)?;
    let ProducerCfgOwnerId::Literal { enclosing, .. } = &cfg.producer_owner else {
        return Err(invalid(&cfg.initial_points[0].source_span));
    };
    let rows = ancestor.construction_forest.sources.iter().filter(|row| row.producer == **enclosing).collect::<Vec<_>>();
    let [_] = rows.as_slice() else { return Err(invalid(&cfg.initial_points[0].source_span)); };
    if input.policy_context != ancestor.policy_context { return Err(invalid(&cfg.initial_points[0].source_span)); }
    Ok(input)
}

pub(in crate::scalar) fn module_preflight_forest(
    source: &SourceIdentity,
    items: &[ScalarItem],
    cfg: &ScalarReferenceCfg,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    phase: SourceCaptureConstructionPhase,
) -> Result<SourceCaptureConstructionForest, ReferenceAnalysisError> {
    fn pending(function: &ScalarFunction, owners: &mut Vec<ProducerCfgOwnerId>) {
        if function.reference_cfg.initial_points.is_empty() { owners.push(function.reference_cfg.producer_owner.clone()); }
        for arm in &function.overload_arms { pending(arm, owners); }
    }
    let mut owners = Vec::new();
    if phase == SourceCaptureConstructionPhase::Building {
        for item in items { if let ScalarItem::Function(function) = item { pending(function, &mut owners); } }
    }
    capture_construction_forest(source, items, cfg, structs, enums, phase, &owners).map_err(|_| invalid(&cfg.initial_points[0].source_span))
}

fn capture_points(points: &[ReferenceFlowPoint]) -> Vec<(CfgPointId, SourceSpan, CfgPointKind)> {
    points.iter().filter(|row| matches!(row.kind, CfgPointKind::Capture { .. } | CfgPointKind::Entry))
        .map(|row| (row.id, row.source_span.clone(), row.kind.clone())).collect()
}

fn generic_type(ty: &ScalarType, names: &BTreeSet<String>) -> bool {
    match ty {
        ScalarType::Named { name, .. } => names.contains(name),
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => generic_type(inner, names),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => generic_type(element, names),
        ScalarType::Callable { parameters, outputs } => parameters.iter().any(|ty| generic_type(ty, names)) || outputs.outputs.iter().any(|row| generic_type(&row.ty, names)),
        _ => false,
    }
}

pub(in crate::scalar) fn validate_capture_preflight(cfg: &ScalarReferenceCfg, input: &SourceCapturePreflightInput) -> Result<(), ReferenceAnalysisError> {
    let originals = input.construction_forest.sources.iter().filter(|row| row.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [original] = originals.as_slice() else { return Err(invalid(&cfg.initial_points[0].source_span)); };
    let source = &original.initial_points[0].source_span;
    if original.policy_context != input.policy_context { return Err(invalid(source)); }
    let current = capture_table_sources(cfg, &input.policy_context).map_err(|_| invalid(source))?;
    let mut demanded = vec![cfg.producer_owner.clone()];
    let mut index = 0;
    while index < demanded.len() {
        let rows = input.construction_forest.sources.iter().filter(|row| row.producer == demanded[index]).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { return Err(invalid(source)); };
        for link in &row.children {
            if demanded.contains(&link.child) { return Err(invalid(source)); }
            demanded.push(link.child.clone());
        }
        index += 1;
    }
    if current.len() != demanded.len() { return Err(invalid(source)); }
    let mut forest = input.construction_forest.clone();
    let mut actual_cfgs = vec![cfg];
    let mut cursor = 0;
    while cursor < actual_cfgs.len() {
        let children = actual_cfgs[cursor].literal_cfgs.iter().map(|child| child.reference_cfg.as_ref()).collect::<Vec<_>>();
        actual_cfgs.extend(children);
        cursor += 1;
    }
    for candidate in &current {
        let rows = forest.sources.iter_mut().filter(|row| row.producer == candidate.producer).collect::<Vec<_>>();
        let [expected] = rows.as_slice() else { return Err(invalid(source)); };
        let actual = actual_cfgs.iter().filter(|row| row.producer_owner == candidate.producer).collect::<Vec<_>>();
        let [actual] = actual.as_slice() else { return Err(invalid(source)); };
        if actual.name_resolution_context != expected.resource_construction.original().name_resolution_context
            || actual.evaluated_values.source_name_reads != expected.resource_construction.original().evaluated_values.source_name_reads {
            return Err(invalid(&expected.initial_points[0].source_span));
        }
        reference::name_resolution::validate(actual, &input.policy_context.structs)?;
        if !demanded.contains(&candidate.producer) || candidate.policy_context != expected.policy_context
            || candidate.capture_inventory != expected.capture_inventory || candidate.capture_points != expected.capture_points
            || candidate.top_level != expected.top_level || candidate.children != expected.children
            || capture_points(&candidate.initial_points) != capture_points(&expected.initial_points)
            || capture_points(&actual.points) != capture_points(&expected.initial_points) {
            return Err(invalid(&expected.initial_points[0].source_span));
        }
        let mut rows = rows;
        rows[0].tables = candidate.tables.clone();
    }
    if !input.substitutions.is_empty() {
        if input.substitutions.keys().cloned().collect::<BTreeSet<_>>() != input.policy_context.generic_parameters { return Err(invalid(source)); }
        for candidate in &current {
            for requirement in &candidate.capture_inventory.required {
                let productions = candidate.tables.source_capture_productions.iter().filter(|row| row.event == requirement.event).collect::<Vec<_>>();
                let [production] = productions.as_slice() else { return Err(invalid(&requirement.output.resolved.occurrence)); };
                for port in &production.inputs {
                    if !generic_type(&port.capture.ty, &input.policy_context.generic_parameters) { continue; }
                    let instances = candidate.tables.source_capture_policy_instances.iter().filter(|row| row.event == requirement.event && row.formal == port.formal && row.substitutions == input.substitutions).collect::<Vec<_>>();
                    let [instance] = instances.as_slice() else { return Err(invalid(&requirement.output.resolved.occurrence)); };
                    let ty = substitute_type(&port.capture.ty, &input.substitutions);
                    let policy = copy_policy(&ty, &input.policy_context.structs, &input.policy_context.enums, &BTreeSet::new()).map_err(|_| invalid(&requirement.output.resolved.occurrence))?;
                    if instance.ty != ty || instance.policy != policy { return Err(invalid(&requirement.output.resolved.occurrence)); }
                }
            }
        }
    }
    ValueSchemaRemap::validate_construction_forest(&forest).map_err(|error| {
        let anchor = match error {
            ValueSchemaRemapError::MissingPoint { producer, point } => forest.sources.iter().find(|row| row.producer == producer).and_then(|row| row.initial_points.iter().find(|row| row.id == point)).map(|row| &row.source_span),
            _ => None,
        };
        match anchor { Some(anchor) => invalid(anchor), None => invalid(source) }
    })?;
    Ok(())
}

pub(in crate::scalar) fn install_capture_policy_instances(cfg: &mut ScalarReferenceCfg, input: &SourceCapturePreflightInput) -> Result<(), ReferenceAnalysisError> {
    let mut rows = Vec::new();
    for production in &cfg.evaluated_values.source_capture_productions {
        for port in &production.inputs {
            if !generic_type(&port.capture.ty, &input.policy_context.generic_parameters) { continue; }
            let ty = substitute_type(&port.capture.ty, &input.substitutions);
            let policy = copy_policy(&ty, &input.policy_context.structs, &input.policy_context.enums, &BTreeSet::new()).map_err(|_| invalid(&production.output.resolved.occurrence))?;
            rows.push(SourceCapturePolicyInstance { event: production.event.clone(), formal: port.formal.clone(), substitutions: input.substitutions.clone(), ty, policy });
        }
    }
    cfg.evaluated_values.source_capture_policy_instances.extend(rows);
    for child in &mut cfg.literal_cfgs { install_capture_policy_instances(&mut child.reference_cfg, input)?; }
    Ok(())
}

#[cfg(test)]
pub(in crate::scalar) fn evaluated_borrow_fixture_input(identity: SourceIdentity, text: &str, layout: ScalarTargetLayout, project: bool) -> (ScalarReferenceCfg, Vec<Diagnostic>, SourceCapturePreflightInput) {
    let parsed = crate::parse_source(identity.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{text}\n{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let (module, diagnostics) = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(single.program, Vec::new())], vec![identity]));
        (validated.project.modules[0].clone(), validated.diagnostics)
    } else { (ScalarModule::from_program(single.program, Vec::new()), single.diagnostics) };
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("run function");
    let forest = module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("original actual evaluated-borrow fixture forest");
    let input = preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("original actual evaluated-borrow fixture context");
    (function.reference_cfg.clone(), diagnostics, input)
}
