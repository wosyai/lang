use super::*;

pub(super) fn assert_shape(before: &ResolvedValueShape, after: &ResolvedValueShape, remap: &ValueSchemaRemap, types: &dyn Fn(&ScalarType, &ScalarType) -> bool, source: &[ValueTableSource], destination: &[ValueTableSource]) {
    remap.validate_qualified_shapes(source, destination).expect("complete explicit qualified declaration/field/type shape graph");
    assert!(types(&before.ty, &after.ty));
    match (&before.payload, &after.payload) {
        (ValuePayloadShape::TemplateParameter { scope: a, declaration: x }, ValuePayloadShape::TemplateParameter { scope: b, declaration: y }) => { assert_eq!(before.ty, after.ty); assert_eq!(a, b); assert_eq!(x, y); }
        (ValuePayloadShape::Plain(a), ValuePayloadShape::Plain(b)) => assert_eq!(a, b),
        (ValuePayloadShape::Checked { mutability: a, referent: x }, ValuePayloadShape::Checked { mutability: b, referent: y }) => { assert_eq!(a, b); assert_eq!(remap.shape(x).unwrap(), *y); }
        (ValuePayloadShape::Raw { pointee: a }, ValuePayloadShape::Raw { pointee: b }) => assert_eq!(remap.shape(a).unwrap(), *b),
        (ValuePayloadShape::Callable { signature: a }, ValuePayloadShape::Callable { signature: b }) => assert!(types(a, b)),
        (ValuePayloadShape::Struct { structure: a, fields: x }, ValuePayloadShape::Struct { structure: b, fields: y }) => {
            assert!(types(&ScalarType::Struct(a.clone()), &ScalarType::Struct(b.clone())));
            assert_eq!(x.len(), y.len());
            assert_eq!(before.ty, ScalarType::Struct(a.clone()));
            assert_eq!(after.ty, ScalarType::Struct(b.clone()));
            assert!(x.iter().all(|field| field.field.structure == *a));
            assert!(y.iter().all(|field| field.field.structure == *b));
        }
        (ValuePayloadShape::Enum { enumeration: a, variants: x }, ValuePayloadShape::Enum { enumeration: b, variants: y }) => {
            assert!(types(&ScalarType::Enum(a.clone()), &ScalarType::Enum(b.clone())));
            assert_eq!(x.len(), y.len());
            for (a, b) in x.iter().zip(y) { assert_eq!(a.ordinal, b.ordinal); assert_eq!(a.fields.len(), b.fields.len()); for (x, y) in a.fields.iter().zip(&b.fields) { assert_eq!(remap.shape(x).unwrap(), *y); } }
        }
        (ValuePayloadShape::FixedArray { element: a, length: x }, ValuePayloadShape::FixedArray { element: b, length: y }) => { assert_eq!(x, y); assert_eq!(remap.shape(a).unwrap(), *b); }
        (ValuePayloadShape::RuntimeArray { element: a }, ValuePayloadShape::RuntimeArray { element: b }) => assert_eq!(remap.shape(a).unwrap(), *b),
        _ => panic!("crossed actual typed shape payload"),
    }
}

fn source_pair(a: &SourceIdentity, b: &SourceIdentity, old: &ScalarModule, new: &ScalarModule) -> bool {
    (a == &old.source && b == &new.source) || (a == b && a != &old.source)
}

fn binding_pair(a: &ReferenceBindingId, b: &ReferenceBindingId, old: &ScalarModule, new: &ScalarModule, owner: &ProducerCfgOwnerId, remap: &ValueSchemaRemap) -> bool {
    source_pair(&a.source, &b.source, old, new) && a.function_span == b.function_span && a.declaration_span == b.declaration_span && a.block_span == b.block_span && match (&a.kind, &b.kind) {
        (ReferenceBindingKind::CallOutput { point: a, output: x }, ReferenceBindingKind::CallOutput { point: b, output: y }) => x == y && remap.point(owner, *a).unwrap() == *b,
        (ReferenceBindingKind::AssignmentPrevious { snapshot: a, output: x }, ReferenceBindingKind::AssignmentPrevious { snapshot: b, output: y }) => x == y && remap.point(owner, *a).unwrap() == *b,
        (ReferenceBindingKind::ImportedCapture { call_start: a, call_end: b, output: c, producer: x }, ReferenceBindingKind::ImportedCapture { call_start: d, call_end: e, output: f, producer: y }) => a == d && b == e && c == f && binding_pair(x, y, old, new, owner, remap),
        _ => a.kind == b.kind,
    }
}

fn field_pair(a: &ScalarFieldReference, b: &ScalarFieldReference, old: &ScalarModule, new: &ScalarModule) -> bool {
    match (a, b) {
        (ScalarFieldReference::Resolved(a), ScalarFieldReference::Resolved(b)) => {
            let originals = old.structs.iter().filter(|row| row.id == a.structure).collect::<Vec<_>>();
            let targets = new.structs.iter().filter(|row| row.id == b.structure).collect::<Vec<_>>();
            let ([x], [y]) = (originals.as_slice(), targets.as_slice()) else { return false; };
            source_pair(&a.structure.source, &b.structure.source, old, new) && x.span == y.span && x.name_span == y.name_span && x.copy_policy == y.copy_policy && a.index == b.index
                && x.fields[a.index].name == y.fields[b.index].name && x.fields[a.index].declaration_index == y.fields[b.index].declaration_index
        }
        _ => a == b,
    }
}

fn scalar_place_pair(a: &ScalarPlace, b: &ScalarPlace, old: &ScalarModule, new: &ScalarModule) -> bool {
    match (a, b) {
        (ScalarPlace::Name { .. }, ScalarPlace::Name { .. }) => a == b,
        (ScalarPlace::Field { base: a, field: x, span: p }, ScalarPlace::Field { base: b, field: y, span: q }) => p == q && field_pair(x, y, old, new) && scalar_place_pair(a, b, old, new),
        (ScalarPlace::Dereference { pointer: a, span: p }, ScalarPlace::Dereference { pointer: b, span: q }) => p == q && a == b,
        (ScalarPlace::Index { base: a, index: x, span: p, index_span: r }, ScalarPlace::Index { base: b, index: y, span: q, index_span: s }) => p == q && r == s && x == y && scalar_place_pair(a, b, old, new),
        _ => false,
    }
}

fn place_pair(a: &ReferencePlaceId, b: &ReferencePlaceId, old: &ScalarModule, new: &ScalarModule, owner: &ProducerCfgOwnerId, remap: &ValueSchemaRemap) -> bool {
    source_pair(&a.source, &b.source, old, new) && a.function_span == b.function_span && a.declaration_span == b.declaration_span && a.block_span == b.block_span
        && binding_pair(&a.binding, &b.binding, old, new, owner, remap) && scalar_place_pair(&a.place, &b.place, old, new) && a.projections.len() == b.projections.len()
        && a.projections.iter().zip(&b.projections).all(|(a, b)| match (a, b) {
            (ReferencePlaceProjection::Field(a), ReferencePlaceProjection::Field(b)) => field_pair(a, b, old, new),
            (ReferencePlaceProjection::Dereference(a), ReferencePlaceProjection::Dereference(b)) => loan_pair(a, b, old, new, owner, remap),
            _ => a == b,
        })
}

fn loan_pair(a: &ReferenceLoanId, b: &ReferenceLoanId, old: &ScalarModule, new: &ScalarModule, owner: &ProducerCfgOwnerId, remap: &ValueSchemaRemap) -> bool {
    a.mode == b.mode && source_pair(&a.creation_source, &b.creation_source, old, new) && a.creation_span == b.creation_span && place_pair(&a.origin_place, &b.origin_place, old, new, owner, remap)
        && match (&a.parent, &b.parent) { (Some(a), Some(b)) => loan_pair(a, b, old, new, owner, remap), (None, None) => true, _ => false }
}

fn places(cfg: &ScalarReferenceCfg) -> Vec<ReferencePlaceId> {
    let mut places = cfg.checked_bindings.values().cloned().collect::<Vec<_>>();
    for origin in &cfg.facts { match origin { ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => places.push(loan.origin_place.clone()), _ => {} } }
    places.extend(cfg.evaluated_values.source_move_inputs.iter().map(|row| row.place.clone()));
    places.extend(cfg.evaluated_values.raw_address_inputs.iter().map(|row| row.place.clone()));
    for destination in &cfg.evaluated_values.destinations { if let EvaluatedDestinationStorage::Direct(place) = &destination.storage { places.push(place.clone()); } }
    for value in &cfg.evaluated_values.values { for case in &value.cases { if let ValueAvailability::Moved { place, .. } = &case.availability { places.push(place.clone()); } if let EvaluatedPayloadFact::Raw { provenance: RawValueProvenance::CompilerStorage { source_place, .. } } = &case.payload { places.push(source_place.clone()); } } }
    let mut unique = Vec::new();
    for place in places { if !unique.contains(&place) { unique.push(place); } }
    unique
}

pub(super) fn imports(old: &ScalarModule, new: &ScalarModule, before: &ScalarReferenceCfg, after: &ScalarReferenceCfg, remap: &ValueSchemaRemap) -> ValueImportCorrespondence {
    let owner = &before.producer_owner;
    let old_places = places(before);
    let new_places = places(after);
    let bindings = |cfg: &ScalarReferenceCfg, places: &[ReferencePlaceId]| {
        let mut bindings = cfg.reference_parameters.iter().chain(&cfg.allocations).chain(cfg.checked_bindings.keys()).cloned().collect::<Vec<_>>();
        bindings.extend(places.iter().map(|row| row.binding.clone()));
        bindings.extend(cfg.evaluated_values.source_binding_shapes.iter().map(|row| row.binding.clone()));
        for row in &cfg.evaluated_values.disposition_inputs { if let ValueConsumerKind::Binding { binding, .. } = &row.consumer.kind { bindings.push(binding.clone()); } }
        let mut unique = Vec::new(); for binding in bindings { if !unique.contains(&binding) { unique.push(binding); } } unique
    };
    let original_bindings = bindings(before, &old_places);
    let target_bindings = bindings(after, &new_places);
    let bindings = original_bindings.iter().map(|binding| {
        let candidates = target_bindings.iter().filter(|candidate| binding_pair(binding, candidate, old, new, owner, remap)).collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else { panic!("unique actual binding constructor counterpart"); };
        (binding.clone(), (*candidate).clone())
    }).collect::<Vec<_>>();
    for binding in &target_bindings { assert_eq!(bindings.iter().filter(|(_, target)| target == binding).count(), 1); }
    let places = old_places.iter().map(|place| {
        let candidates = new_places.iter().filter(|candidate| place_pair(place, candidate, old, new, owner, remap)).collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else { panic!("unique full original place counterpart"); };
        (place.clone(), (*candidate).clone())
    }).collect::<Vec<_>>();
    for place in &new_places { assert_eq!(places.iter().filter(|(_, target)| target == place).count(), 1); }
    let origins = before.facts.iter().map(|origin| {
        let candidates = after.facts.iter().enumerate().filter(|(_, candidate)| match (origin, candidate) {
            (ReferenceOrigin::Fresh { allocation: a, loan: x }, ReferenceOrigin::Fresh { allocation: b, loan: y }) => source_pair(&a.source, &b.source, old, new) && a.function_span == b.function_span && a.declaration_span == b.declaration_span && a.creation_span == b.creation_span && loan_pair(x, y, old, new, owner, remap),
            (ReferenceOrigin::BorrowedFrom { parameter: a, loan: x }, ReferenceOrigin::BorrowedFrom { parameter: b, loan: y }) => a == b && loan_pair(x, y, old, new, owner, remap),
            _ => origin == *candidate,
        }).collect::<Vec<_>>();
        let [(index, _)] = candidates.as_slice() else { panic!("unique dense original origin fact counterpart"); };
        ReferenceOriginId(*index)
    }).collect::<Vec<_>>();
    assert_eq!(before.facts.len(), after.facts.len());
    for index in 0..after.facts.len() { assert_eq!(origins.iter().filter(|row| row.0 == index).count(), 1); }
    let mut loans = Vec::new();
    fn collect_loan(loan: &ReferenceLoanId, loans: &mut Vec<ReferenceLoanId>) { if !loans.contains(loan) { loans.push(loan.clone()); } if let Some(parent) = &loan.parent { collect_loan(parent, loans); } }
    for origin in &before.facts { if let ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } = origin { collect_loan(loan, &mut loans); } }
    let mut target_loans = Vec::new();
    for origin in &after.facts { if let ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } = origin { collect_loan(loan, &mut target_loans); } }
    let loans = loans.into_iter().map(|loan| {
        let candidates = target_loans.iter().filter(|candidate| loan_pair(&loan, candidate, old, new, owner, remap)).collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else { panic!("unique checked loan creation and ancestry counterpart"); };
        (loan, (*candidate).clone())
    }).collect::<Vec<_>>();
    for cfg in [before, after] {
        assert!(cfg.backing_parameters.is_empty());
        assert!(cfg.backing_length_sites.is_empty());
        assert!(cfg.backing_input_paths.iter().all(Vec::is_empty));
        assert!(cfg.projected_backing_input_paths.iter().all(Vec::is_empty));
        assert!(cfg.points.iter().all(|row| row.possible_backings.is_empty() && row.possible_projected_backings.is_empty()));
        assert!(cfg.evaluated_values.values.iter().flat_map(|row| &row.cases).all(|row| !matches!(row.payload, EvaluatedPayloadFact::RuntimeBacking { .. }) && row.dependencies.iter().all(|dependency| !matches!(dependency.source, DependencySource::Region { .. }))));
    }
    let environments = before.environments.iter().enumerate().map(|(index, environment)| {
        let candidates = after.environments.iter().enumerate().filter(|(_, candidate)| source_pair(&environment.literal.source, &candidate.literal.source, old, new) && environment.literal.range == candidate.literal.range && match (&environment.instance, &candidate.instance) {
            (CallableEnvironmentInstance::Capture, CallableEnvironmentInstance::Capture) => true,
            (CallableEnvironmentInstance::CallOutput { call: a, output: x, function_span: p }, CallableEnvironmentInstance::CallOutput { call: b, output: y, function_span: q }) => source_pair(&a.source, &b.source, old, new) && a.range == b.range && x == y && p == q,
            _ => false,
        }).collect::<Vec<_>>();
        let [(target_index, candidate)] = candidates.as_slice() else { panic!("unique actual environment creation instance counterpart"); };
        assert!(environment.backings.is_empty() && candidate.backings.is_empty());
        assert!(environment.inputs.is_empty() && candidate.inputs.is_empty());
        assert!(environment.capture_paths.iter().all(Vec::is_empty) && candidate.capture_paths.iter().all(Vec::is_empty));
        assert_eq!(environment.origins.iter().map(|row| origins[row.0]).collect::<BTreeSet<_>>(), candidate.origins);
        assert_eq!(environment.owners.iter().map(|owner| { let matches = bindings.iter().filter(|(old, _)| old == owner).collect::<Vec<_>>(); let [(_, new)] = matches.as_slice() else { panic!("actual environment owner counterpart"); }; new.clone() }).collect::<BTreeSet<_>>(), candidate.owners);
        assert_eq!(environment.loans.iter().map(|loan| CallableLoanId { origin: origins[loan.origin.0] }).collect::<BTreeSet<_>>(), candidate.loans);
        (CallableEnvironmentId { literal: environment.literal.clone(), index }, CallableEnvironmentId { literal: candidate.literal.clone(), index: *target_index })
    }).collect::<Vec<_>>();
    assert_eq!(environments.len(), after.environments.len());
    let holders = holder_pairs(before, after, &bindings, &places, &environments, remap, old, new);
    ValueImportCorrespondence { producer: owner.clone(), bindings, places, origins, loans, runtime_values: Vec::new(), environments, backings: Vec::new(), holders }
}

fn holder_pairs(before: &ScalarReferenceCfg, after: &ScalarReferenceCfg, bindings: &[(ReferenceBindingId, ReferenceBindingId)], places: &[(ReferencePlaceId, ReferencePlaceId)], environments: &[(CallableEnvironmentId, CallableEnvironmentId)], remap: &ValueSchemaRemap, old: &ScalarModule, new: &ScalarModule) -> Vec<(ConnectedCaptureHolderFact, ConnectedCaptureHolderFact)> {
    let holder = |a: &CallableHolderPlace, b: &CallableHolderPlace| match (a, b) {
        (CallableHolderPlace::Binding(a), CallableHolderPlace::Binding(b)) => bindings.contains(&(a.clone(), b.clone())),
        (CallableHolderPlace::Projection(a), CallableHolderPlace::Projection(b)) => places.contains(&(a.clone(), b.clone())),
        (CallableHolderPlace::Temporary(a), CallableHolderPlace::Temporary(b)) => remap.point(&before.producer_owner, *a).unwrap() == *b,
        _ => a == b,
    };
    let environment = |a: &CallableEnvironmentId, b: &CallableEnvironmentId| environments.contains(&(a.clone(), b.clone()));
    let action = |a: &CallableHolderAction, b: &CallableHolderAction| match (a, b) {
        (CallableHolderAction::Retain { environment: a, from: x, to: p, span: s }, CallableHolderAction::Retain { environment: b, from: y, to: q, span: t })
        | (CallableHolderAction::Move { environment: a, from: x, to: p, span: s }, CallableHolderAction::Move { environment: b, from: y, to: q, span: t })
        | (CallableHolderAction::ReturnTransfer { environment: a, from: x, to: p, span: s }, CallableHolderAction::ReturnTransfer { environment: b, from: y, to: q, span: t }) => environment(a, b) && holder(x, y) && holder(p, q) && s.range == t.range && source_pair(&s.source, &t.source, old, new),
        (CallableHolderAction::Drop { environment: a, holder: x, span: s }, CallableHolderAction::Drop { environment: b, holder: y, span: t }) => environment(a, b) && holder(x, y) && s.range == t.range && source_pair(&s.source, &t.source, old, new),
        _ => a == b,
    };
    fn producer(a: &HolderProducerRef, b: &HolderProducerRef, owner: &ProducerCfgOwnerId, remap: &ValueSchemaRemap, environment: &dyn Fn(&CallableEnvironmentId, &CallableEnvironmentId) -> bool, action: &dyn Fn(&CallableHolderAction, &CallableHolderAction) -> bool) -> bool {
        match (a, b) {
            (HolderProducerRef::CaptureCreation { point: a, environment: x }, HolderProducerRef::CaptureCreation { point: b, environment: y }) => remap.point(owner, *a).unwrap() == *b && environment(x, y),
            (HolderProducerRef::Forward { point: a, environment: x, action: p, previous: s }, HolderProducerRef::Forward { point: b, environment: y, action: q, previous: t }) => remap.point(owner, *a).unwrap() == *b && environment(x, y) && action(p, q) && producer(s, t, owner, remap, environment, action),
            (HolderProducerRef::InstalledRead { point: a, environment: x, action: p }, HolderProducerRef::InstalledRead { point: b, environment: y, action: q }) => remap.point(owner, *a).unwrap() == *b && environment(x, y) && action(p, q),
            _ => a == b,
        }
    }
    let collect = |cfg: &ScalarReferenceCfg| { let mut holders = Vec::new(); for case in cfg.evaluated_values.values.iter().flat_map(|row| &row.cases) { if let EvaluatedPayloadFact::Callable { producers } = &case.payload { for holder in producers { if !holders.contains(holder) { holders.push(holder.clone()); } } } } holders };
    let originals = collect(before);
    let targets = collect(after);
    let pairs = originals.into_iter().map(|row| {
        let candidates = targets.iter().filter(|candidate| holder(&row.holder, &candidate.holder) && environment(&row.environment, &candidate.environment) && producer(&row.producer, &candidate.producer, &before.producer_owner, remap, &environment, &action)).collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else { panic!("unique connected holder full creation/Forward provenance counterpart"); };
        (row, (*candidate).clone())
    }).collect::<Vec<_>>();
    for holder in &targets { assert_eq!(pairs.iter().filter(|(_, target)| target == holder).count(), 1); }
    pairs
}
