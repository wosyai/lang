use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
enum OriginalSignatureOrigin {
    Module { root: ProducerCfgOwnerId, catalogue: RuntimeCallSignature, maker: ProducerCfgOwnerId },
    Extern { root: ProducerCfgOwnerId, source: SourceIdentity, declaration: ByteSpan },
    Core { root: ProducerCfgOwnerId, operation: CoreOperationId },
    Local { root: ProducerCfgOwnerId, target: ResolvedCallTarget, declared: ScalarType, creators: Vec<SourceEventId>, signatures: Vec<ScalarType>, paths: Vec<resources::callee_constructor::OriginalCalleePath> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar::evaluated_value::remap::input_counterpart) struct OriginalCallSignaturePair {
    source_location: OriginalAstLocation,
    destination_location: OriginalAstLocation,
    source: ScalarCallCfgPoint,
    destination: ScalarCallCfgPoint,
    source_point: ReferenceFlowPoint,
    destination_point: ReferenceFlowPoint,
    source_inventory: Vec<RuntimeCallSignature>,
    destination_inventory: Vec<RuntimeCallSignature>,
    source_origin: OriginalSignatureOrigin,
    destination_origin: OriginalSignatureOrigin,
}

fn lexical_root(forest: &SourceCaptureConstructionForest, owner: &ProducerCfgOwnerId) -> Result<ProducerCfgOwnerId, ValueSchemaRemapError> {
    let mut current = owner.clone();
    loop {
        let row = carrier(forest, &current)?;
        match &current {
            ProducerCfgOwnerId::Function { .. } | ProducerCfgOwnerId::Module(_) => return Ok(current),
            ProducerCfgOwnerId::Literal { enclosing, capture_point, literal } => {
                let parent = carrier(forest, enclosing)?;
                if parent.children.iter().filter(|link| link.child == current && link.event.owner == **enclosing && link.event.point == *capture_point).count() != 1
                    || parent.resource_construction.original().literal_cfgs.iter().filter(|child| child.capture_point == *capture_point && child.literal == *literal && child.reference_cfg.as_ref() == row.resource_construction.original()).count() != 1 {
                    return Err(ValueSchemaRemapError::MissingProducer(current));
                }
                current = (**enclosing).clone();
            }
        }
    }
}

#[cfg(test)]
pub(super) fn proof_candidates(checked: &CheckedBoundaryInputTypes) -> Vec<CheckedBoundaryInputTypes> {
    let mut candidates = Vec::new();
    for (index, row) in checked.call_signatures.iter().enumerate() {
        for defect in 0..8 {
            let mut candidate = checked.clone();
            let actual = &mut candidate.call_signatures[index];
            match defect {
                0 => actual.destination.evaluated_point = CfgPointId(usize::MAX),
                1 => actual.destination_point.kind = CfgPointKind::ScopeExit,
                2 => actual.destination_location.path.push(OriginalAstStep::Argument(usize::MAX)),
                3 => match &mut actual.destination_origin { OriginalSignatureOrigin::Module { root, .. } | OriginalSignatureOrigin::Extern { root, .. } | OriginalSignatureOrigin::Core { root, .. } | OriginalSignatureOrigin::Local { root, .. } => *root = row.destination_location.owner.clone() },
                4 => match &mut actual.destination_origin { OriginalSignatureOrigin::Module { catalogue, .. } => catalogue.declaration_span = ByteSpan::new(0, 0), _ => continue },
                5 => match &mut actual.destination_origin { OriginalSignatureOrigin::Module { catalogue, .. } => catalogue.parameters.push(ScalarType::Error), OriginalSignatureOrigin::Local { signatures, .. } => signatures.push(ScalarType::Error), _ => continue },
                6 => match &mut actual.destination_origin { OriginalSignatureOrigin::Module { catalogue, .. } => catalogue.generic_parameters.push("incorrect".into()), _ => continue },
                7 => match &mut actual.destination_point.kind {
                    CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Overload(selection), .. }, .. } => {
                        let Some(value) = selection.substitutions.values_mut().next() else { continue; };
                        *value = ScalarType::Error;
                    }
                    _ => continue,
                },
                _ => unreachable!(),
            }
            if candidate != *checked { candidates.push(candidate); }
        }
    }
    candidates
}

fn origin(forest: &SourceCaptureConstructionForest, cfg: &ScalarReferenceCfg, point: &ReferenceFlowPoint, target: &ResolvedCallTarget) -> Result<OriginalSignatureOrigin, ValueSchemaRemapError> {
    let root = lexical_root(forest, &cfg.producer_owner)?;
    let catalogue = carrier(forest, &root)?.resource_construction.original();
    let invalid = || ValueSchemaRemapError::MissingPoint { producer: cfg.producer_owner.clone(), point: point.id };
    match target {
        ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } => {
            let selection = match concrete { ConcreteCallSelection::Declaration => RuntimeCallSelection::Declaration, ConcreteCallSelection::Overload(selection) => RuntimeCallSelection::Overload(selection.arm_index) };
            let rows = catalogue.call_signatures.iter().filter(|signature| signature.source == *source && signature.declaration_span == *declaration_span && signature.selection == selection).collect::<Vec<_>>();
            let [signature] = rows.as_slice() else { return Err(invalid()); };
            let local = cfg.call_signatures.iter().filter(|row| row.source == *source && row.declaration_span == *declaration_span && row.selection == selection).collect::<Vec<_>>();
            if local.len() > 1 || local.iter().any(|row| **row != **signature) { return Err(invalid()); }
            let makers = forest.sources.iter().filter(|row| matches!(&row.producer, ProducerCfgOwnerId::Function { source: s, declaration, body } if s == source && declaration == declaration_span && match (body, concrete) {
                (ProducerFunctionBodyId::Declaration, ConcreteCallSelection::Declaration) => true,
                (ProducerFunctionBodyId::Overload(a), ConcreteCallSelection::Overload(b)) => *a == b.arm_index,
                _ => false,
            })).collect::<Vec<_>>();
            let [maker] = makers.as_slice() else { return Err(invalid()); };
            Ok(OriginalSignatureOrigin::Module { root, catalogue: (*signature).clone(), maker: maker.producer.clone() })
        }
        ResolvedCallTarget::ExternCallable { source, declaration_span } => Ok(OriginalSignatureOrigin::Extern { root, source: source.clone(), declaration: *declaration_span }),
        ResolvedCallTarget::Core(operation) => Ok(OriginalSignatureOrigin::Core { root, operation: *operation }),
        ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. } => {
            let owner = carrier(forest, &cfg.producer_owner)?;
            let declared = match target {
                ResolvedCallTarget::LocalCallable(binding) => binding_type(cfg, binding)?,
                ResolvedCallTarget::CallableField { receiver_place, field } => {
                    let mut ty = binding_type(cfg, &receiver_place.binding)?;
                    for projection in &receiver_place.projections {
                        ty = match (projection, ty) {
                            (ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)), ScalarType::Struct(structure)) if field.structure == structure => field_type(&owner.policy_context, field)?,
                            (ReferencePlaceProjection::Index(_), ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. }) => *element,
                            (ReferencePlaceProjection::Dereference(_), ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. }) => *inner,
                            _ => return Err(invalid()),
                        };
                    }
                    if ty != ScalarType::Struct(field.structure.clone()) { return Err(invalid()); }
                    field_type(&owner.policy_context, field)?
                }
                _ => unreachable!(),
            };
            if !matches!(declared, ScalarType::Callable { .. }) { return Err(invalid()); }
            let current = cfg.points.iter().filter(|row| row.id == point.id && row.kind == point.kind).collect::<Vec<_>>();
            let [current] = current.as_slice() else { return Err(invalid()); };
            let paths = resources::callee_constructor::paths(forest, cfg, current, target)?;
            let originals = resources::callee_constructor::signatures(&paths);
            let creators = originals.iter().map(|row| row.0.clone()).collect();
            let signatures = originals.into_iter().map(|row| row.1).collect();
            Ok(OriginalSignatureOrigin::Local { root, target: target.clone(), declared, creators, signatures, paths })
        }
    }
}

fn binding_type(cfg: &ScalarReferenceCfg, binding: &ReferenceBindingId) -> Result<ScalarType, ValueSchemaRemapError> {
    let mut types = Vec::new();
    for slot in &cfg.evaluated_values.source_slots {
        let selected = match &slot.role {
            SourceCoordinateRole::Binding(actual) | SourceCoordinateRole::Boundary(BoundaryPortRole::Parameter { binding: actual, .. } | BoundaryPortRole::Capture { binding: actual, .. } | BoundaryPortRole::Global(actual)) => actual == binding,
            _ => false,
        };
        if selected {
            let rows = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == slot.shape).collect::<Vec<_>>();
            let [shape] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(slot.shape.clone())); };
            if !types.contains(&shape.ty) { types.push(shape.ty.clone()); }
        }
    }
    let [ty] = types.as_slice() else { return Err(ValueSchemaRemapError::MissingBinding(binding.clone())); };
    Ok(ty.clone())
}

fn field_type(context: &SourceCapturePolicyContext, field: &ScalarStructFieldId) -> Result<ScalarType, ValueSchemaRemapError> {
    let fields = context.structs.iter().filter(|structure| structure.id == field.structure).flat_map(|structure| &structure.fields).filter(|row| row.id == *field).collect::<Vec<_>>();
    let [field] = fields.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone())); };
    Ok(field.ty.clone())
}

pub(super) fn call(visitor: &mut InputVisitor<'_>, source: &ScalarExpression, destination: &ScalarExpression, occurrence: &ScalarCallOccurrence, actual_occurrence: &ScalarCallOccurrence) -> Result<(), ValueSchemaRemapError> {
    let sources = visitor.source.call_points.iter().filter(|row| row.occurrence == *occurrence).collect::<Vec<_>>();
    let destinations = visitor.destination.call_points.iter().filter(|row| row.occurrence == *actual_occurrence).collect::<Vec<_>>();
    let ([registered], [actual_registered]) = (sources.as_slice(), destinations.as_slice()) else { return Err(visitor.invalid()); };
    visitor.require(visitor.point(registered.evaluated_point)? == actual_registered.evaluated_point && registered.projections.len() == actual_registered.projections.len())?;
    for (a, b) in registered.projections.iter().zip(&actual_registered.projections) { visitor.require(a.output == b.output && visitor.point(a.point)? == b.point)?; }
    let source_points = visitor.source.initial_points.iter().filter(|point| point.id == registered.evaluated_point).collect::<Vec<_>>();
    let actual_points = visitor.destination.initial_points.iter().filter(|point| point.id == actual_registered.evaluated_point).collect::<Vec<_>>();
    let ([a], [b]) = (source_points.as_slice(), actual_points.as_slice()) else { return Err(visitor.invalid()); };
    let (CfgPointKind::Call { expression: x, output: m, target: t, argument_points: p }, CfgPointKind::Call { expression: y, output: n, target: u, argument_points: q }) = (&a.kind, &b.kind) else { return Err(visitor.invalid()); };
    visitor.require(x == source && y == destination && m == n && p.len() == q.len())?;
    for (p, q) in p.iter().zip(q) { visitor.require(visitor.point(*p)? == *q)?; outputs::point_values(visitor, *p, *q)?; }
    for (a, b) in registered.projections.iter().zip(&actual_registered.projections) { outputs::point_values(visitor, a.point, b.point)?; }
    if registered.projections.is_empty() { outputs::point_values(visitor, registered.evaluated_point, actual_registered.evaluated_point)?; }
    let source_origin = origin(&visitor.checked.source_forest, visitor.source, a, t)?;
    let destination_origin = origin(&visitor.checked.destination_forest, visitor.destination, b, u)?;
    let source_root = lexical_root(&visitor.checked.source_forest, &visitor.source.producer_owner)?;
    let actual_root = lexical_root(&visitor.checked.destination_forest, &visitor.destination.producer_owner)?;
    visitor.require(resources::qualified_input_owner(visitor.remap, &source_root)? == actual_root)?;
    match (t, u) {
        (ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }, ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }) => {},
        (ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Overload(a), .. }, ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Overload(b), .. }) => visitor.selection(&Some(a.clone()), &Some(b.clone()))?,
        (ResolvedCallTarget::ModuleCallable { .. }, _) | (_, ResolvedCallTarget::ModuleCallable { .. }) => return Err(visitor.invalid()),
        _ => {},
    }
    match (&source_origin, &destination_origin) {
        (OriginalSignatureOrigin::Module { catalogue: a, maker: x, .. }, OriginalSignatureOrigin::Module { catalogue: b, maker: y, .. }) => {
            visitor.require(resources::qualified_input_owner(visitor.remap, x)? == *y && a.selection == b.selection && a.declaration_span == b.declaration_span
                && a.generic_parameters == b.generic_parameters && a.parameters.len() == b.parameters.len() && a.outputs.len() == b.outputs.len() && a.bindings.len() == b.bindings.len())?;
            let source = carrier(&visitor.checked.source_forest, x)?.clone();
            let destination = carrier(&visitor.checked.destination_forest, y)?.clone();
            let qualification = BoundaryInputQualification::creator(visitor.remap, x)?;
            qualification.validate(&source, &destination)?;
            visitor.checked.retain_original(&source)?;
            visitor.checked.retain_original(&destination)?;
            if !visitor.checked.qualifications.contains(&qualification) { types::roots(visitor.remap, visitor.checked, &source, &destination, &qualification)?; visitor.checked.qualifications.push(qualification); }
            let qualification = visitor.checked.qualification(visitor.remap, x)?;
            for (source_binding, actual_binding) in a.bindings.iter().zip(&b.bindings) {
                let mapped = match qualification { BoundaryInputQualification::Mapped { .. } => visitor.remap.binding(x, source_binding)?, BoundaryInputQualification::SharedOriginal { .. } => source_binding.clone() };
                visitor.require(mapped == *actual_binding)?;
            }
            for (a, b) in a.parameters.iter().chain(&a.outputs).zip(b.parameters.iter().chain(&b.outputs)) { visitor.require(visitor.checked.type_pair(x, a, b))?; }
        }
        (OriginalSignatureOrigin::Extern { source: a, declaration: x, .. }, OriginalSignatureOrigin::Extern { source: b, declaration: y, .. }) => visitor.require(a == b && x == y)?,
        (OriginalSignatureOrigin::Core { operation: a, .. }, OriginalSignatureOrigin::Core { operation: b, .. }) => visitor.require(a == b)?,
        (OriginalSignatureOrigin::Local { target: a, declared: declared_a, paths: source_paths, .. }, OriginalSignatureOrigin::Local { target: b, declared: declared_b, paths: actual_paths, .. }) => {
            visitor.ty(declared_a, declared_b)?;
            match (a, b) {
                (ResolvedCallTarget::LocalCallable(a), ResolvedCallTarget::LocalCallable(b)) => visitor.require(visitor.binding_reference(a)? == *b)?,
                (ResolvedCallTarget::CallableField { receiver_place: a, field: x }, ResolvedCallTarget::CallableField { receiver_place: b, field: y }) => {
                    visitor.require(visitor.remap.place(&visitor.source.producer_owner, a)? == *b)?;
                    visitor.field(&ScalarFieldReference::Resolved(x.clone()), &ScalarFieldReference::Resolved(y.clone()))?;
                }
                _ => return Err(visitor.invalid()),
            }
            visitor.require(resources::callee_constructor::paired(visitor.remap, &visitor.source.producer_owner, source_paths, actual_paths)?)?;
            callee_types(visitor, source_paths, actual_paths)?;
        }
        _ => return Err(visitor.invalid()),
    }
    let record = OriginalCallSignaturePair { source_location: visitor.source_location.clone(), destination_location: visitor.destination_location.clone(), source: (*registered).clone(), destination: (*actual_registered).clone(), source_point: (*a).clone(), destination_point: (*b).clone(), source_inventory: visitor.source.call_signatures.clone(), destination_inventory: visitor.destination.call_signatures.clone(), source_origin, destination_origin };
    if !visitor.checked.call_signatures.contains(&record) { visitor.checked.call_signatures.push(record); }
    Ok(())
}

fn callee_types(visitor: &mut InputVisitor<'_>, source: &[resources::callee_constructor::OriginalCalleePath], destination: &[resources::callee_constructor::OriginalCalleePath]) -> Result<(), ValueSchemaRemapError> {
    let actual = resources::callee_constructor::signatures(destination);
    for (event, signature) in resources::callee_constructor::signatures(source) {
        let qualification = BoundaryInputQualification::creator(visitor.remap, &event.owner)?;
        let mapped = qualification.point(visitor.remap, &event)?;
        let rows = actual.iter().filter(|row| row.0 == mapped).collect::<Vec<_>>();
        let [target] = rows.as_slice() else { return Err(visitor.invalid()); };
        let before = carrier(&visitor.checked.source_forest, &event.owner)?.clone();
        let after = carrier(&visitor.checked.destination_forest, &mapped.owner)?.clone();
        qualification.validate(&before, &after)?;
        visitor.checked.retain_original(&before)?; visitor.checked.retain_original(&after)?;
        types::roots(visitor.remap, visitor.checked, &before, &after, &qualification)?;
        if !visitor.checked.qualifications.contains(&qualification) { visitor.checked.qualifications.push(qualification); }
        visitor.require(visitor.checked.type_pair(&event.owner, &signature, &target.1))?;
    }
    Ok(())
}

pub(in crate::scalar::evaluated_value::remap::input_counterpart) fn registered_calls(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, owners: &[ProducerCfgOwnerId]) -> Result<(), ValueSchemaRemapError> {
    for owner in owners {
        let source = carrier(&checked.source_forest, owner)?.clone();
        let qualification = checked.qualification(remap, owner)?;
        let actual = carrier(&checked.destination_forest, qualification.destination())?.clone();
        let before = source.resource_construction.original(); let after = actual.resource_construction.original();
        for registration in &before.call_points {
            let point = before.initial_points.iter().filter(|point| point.id == registration.evaluated_point).collect::<Vec<_>>();
            let [point] = point.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
            let CfgPointKind::Call { expression, .. } = &point.kind else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let event = qualification.point(remap, &SourceEventId { owner: owner.clone(), point: point.id })?;
            let targets = after.initial_points.iter().filter(|point| point.id == event.point).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let CfgPointKind::Call { expression: actual_expression, .. } = &target.kind else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let facts = before.evaluated_values.values.iter().filter(|fact| fact.id.point == point.id && fact.id.resolved.output == 0).collect::<Vec<_>>();
            let [fact] = facts.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let mut visitor = InputVisitor { remap, checked, source: before, destination: after, root: fact.shape.clone(), event: SourceEventId { owner: owner.clone(), point: point.id },
                source_location: OriginalAstLocation { owner: owner.clone(), path: vec![OriginalAstStep::Point(point.id)] }, destination_location: OriginalAstLocation { owner: actual.producer.clone(), path: vec![OriginalAstStep::Point(target.id)] } };
            visitor.expression(expression, actual_expression)?;
        }
    }
    Ok(())
}
