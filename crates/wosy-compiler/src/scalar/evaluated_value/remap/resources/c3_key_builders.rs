use super::*;

pub(super) fn type_key(a: &ScalarType, b: &ScalarType, source: &SourceCapturePolicyContext, target: &SourceCapturePolicyContext) -> bool {
    match (a, b) {
        (ScalarType::Struct(a), ScalarType::Struct(b)) => {
            let originals = source.structs.iter().filter(|row| row.id == *a).collect::<Vec<_>>();
            let targets = target.structs.iter().filter(|row| row.id == *b).collect::<Vec<_>>();
            let ([x], [y]) = (originals.as_slice(), targets.as_slice()) else { return false; };
            x.name == y.name && x.name_span == y.name_span && x.span == y.span && x.copy_policy == y.copy_policy && x.copy_modifier_span == y.copy_modifier_span
                && x.fields.len() == y.fields.len() && x.fields.iter().zip(&y.fields).all(|(x, y)| x.declaration_index == y.declaration_index && x.name == y.name && x.name_span == y.name_span)
        }
        (ScalarType::Enum(a), ScalarType::Enum(b)) => {
            let originals = source.enums.iter().filter(|row| row.id == *a).collect::<Vec<_>>();
            let targets = target.enums.iter().filter(|row| row.id == *b).collect::<Vec<_>>();
            let ([x], [y]) = (originals.as_slice(), targets.as_slice()) else { return false; };
            x.name == y.name && x.name_span == y.name_span && x.span == y.span && x.copy_policy == y.copy_policy && x.copy_modifier_span == y.copy_modifier_span && x.variants == y.variants
        }
        (ScalarType::CheckedReference { mutability: a, inner: x }, ScalarType::CheckedReference { mutability: b, inner: y }) => a == b && type_key(x, y, source, target),
        (ScalarType::RawPointer(a), ScalarType::RawPointer(b)) => type_key(a, b, source, target),
        (ScalarType::Array { element: a, length: x, length_span: u, span: p }, ScalarType::Array { element: b, length: y, length_span: v, span: q }) => x == y && u == v && p == q && type_key(a, b, source, target),
        (ScalarType::RuntimeArray { element: a, span: x }, ScalarType::RuntimeArray { element: b, span: y }) => x == y && type_key(a, b, source, target),
        (ScalarType::Callable { outputs: a, parameters: x }, ScalarType::Callable { outputs: b, parameters: y }) => a.span == b.span && x.len() == y.len() && a.outputs.len() == b.outputs.len()
            && x.iter().zip(y).all(|(a, b)| type_key(a, b, source, target)) && a.outputs.iter().zip(&b.outputs).all(|(a, b)| a.span == b.span && type_key(&a.ty, &b.ty, source, target)),
        (ScalarType::Named { .. }, ScalarType::Named { .. }) => a == b && source.generic_scope == target.generic_scope && source.generic_declarations == target.generic_declarations,
        _ => a == b,
    }
}

fn source_pair(remap: &ValueSchemaRemap, a: &SourceIdentity, b: &SourceIdentity) -> bool {
    remap.producers.iter().any(|(source, target)| copied::lexical(source) == a && copied::lexical(target) == b)
}

fn point_pair(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: CfgPointId, b: CfgPointId) -> bool {
    remap.points.iter().filter(|row| row.producer == *owner && row.point == a && row.imported == b).count() == 1
}

fn binding(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &ReferenceBindingId, b: &ReferenceBindingId) -> bool {
    if !source_pair(remap, &a.source, &b.source) || a.function_span != b.function_span || a.declaration_span != b.declaration_span || a.block_span != b.block_span { return false; }
    match (&a.kind, &b.kind) {
        (ReferenceBindingKind::CallOutput { point: x, output: u }, ReferenceBindingKind::CallOutput { point: y, output: v }) => u == v && point_pair(remap, owner, *x, *y),
        (ReferenceBindingKind::AssignmentPrevious { snapshot: x, output: u }, ReferenceBindingKind::AssignmentPrevious { snapshot: y, output: v }) => u == v && point_pair(remap, owner, *x, *y),
        (ReferenceBindingKind::ImportedCapture { call_start: x, call_end: u, output: p, producer: s }, ReferenceBindingKind::ImportedCapture { call_start: y, call_end: v, output: q, producer: t }) => x == y && u == v && p == q && binding(remap, owner, s, t),
        _ => a.kind == b.kind,
    }
}

fn projections(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &[ReferencePlaceProjection], b: &[ReferencePlaceProjection]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| match (a, b) {
        (ReferencePlaceProjection::Dereference(a), ReferencePlaceProjection::Dereference(b)) => loan(remap, owner, a, b),
        _ => a == b,
    })
}

fn place(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &ReferencePlaceId, b: &ReferencePlaceId) -> bool {
    source_pair(remap, &a.source, &b.source) && a.function_span == b.function_span && a.declaration_span == b.declaration_span && a.block_span == b.block_span
        && binding(remap, owner, &a.binding, &b.binding) && projections(remap, owner, &a.projections, &b.projections) && a.place == b.place
}

fn loan(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &ReferenceLoanId, b: &ReferenceLoanId) -> bool {
    a.mode == b.mode && a.creation_span == b.creation_span && source_pair(remap, &a.creation_source, &b.creation_source) && place(remap, owner, &a.origin_place, &b.origin_place)
        && match (&a.parent, &b.parent) { (Some(a), Some(b)) => loan(remap, owner, a, b), (None, None) => true, _ => false }
}

fn runtime(remap: &ValueSchemaRemap, a: &RuntimeValueId, b: &RuntimeValueId) -> bool {
    if !source_pair(remap, &a.source, &b.source) || a.function_span != b.function_span || a.creation_span != b.creation_span { return false; }
    match (&a.position, &b.position) {
        (RuntimeValuePosition::CapturedCall { call: x, output: u, lineage: s }, RuntimeValuePosition::CapturedCall { call: y, output: v, lineage: t }) => x.range == y.range && source_pair(remap, &x.source, &y.source) && u == v && runtime(remap, s, t),
        (RuntimeValuePosition::CallOutput { call: x, output: u, target_source: s, target_declaration: p, origin: h, action: k, lineage: m }, RuntimeValuePosition::CallOutput { call: y, output: v, target_source: t, target_declaration: q, origin: i, action: l, lineage: n }) => x == y && u == v && source_pair(remap, s, t) && p == q && h.range == i.range && source_pair(remap, &h.source, &i.source) && k == l && runtime(remap, m, n),
        _ => a.position == b.position,
    }
}

fn environment(remap: &ValueSchemaRemap, a: &CallableEnvironmentId, b: &CallableEnvironmentId) -> bool {
    source_pair(remap, &a.literal.source, &b.literal.source) && a.literal.range == b.literal.range
}

fn holder_place(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &CallableHolderPlace, b: &CallableHolderPlace) -> bool {
    match (a, b) {
        (CallableHolderPlace::Temporary(a), CallableHolderPlace::Temporary(b)) => point_pair(remap, owner, *a, *b),
        (CallableHolderPlace::Binding(a), CallableHolderPlace::Binding(b)) => binding(remap, owner, a, b),
        (CallableHolderPlace::Projection(a), CallableHolderPlace::Projection(b)) => place(remap, owner, a, b),
        (CallableHolderPlace::OutputProjection { output: a, projections: x }, CallableHolderPlace::OutputProjection { output: b, projections: y }) => a == b && projections(remap, owner, x, y),
        _ => a == b,
    }
}

fn producer(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &HolderProducerRef, b: &HolderProducerRef, forest: &SourceCaptureConstructionForest) -> bool {
    let owner = &creators::constructor(owner, a, forest).expect("genuine original maker ancestry before reserving target key").producer;
    match (a, b) {
        (HolderProducerRef::CaptureCreation { point: a, environment: x }, HolderProducerRef::CaptureCreation { point: b, environment: y })
        | (HolderProducerRef::CallCreation { point: a, environment: x, .. }, HolderProducerRef::CallCreation { point: b, environment: y, .. })
        | (HolderProducerRef::InstalledRead { point: a, environment: x, .. }, HolderProducerRef::InstalledRead { point: b, environment: y, .. }) => point_pair(remap, owner, *a, *b) && environment(remap, x, y),
        (HolderProducerRef::Forward { point: a, environment: x, previous: u, .. }, HolderProducerRef::Forward { point: b, environment: y, previous: v, .. }) => point_pair(remap, owner, *a, *b) && environment(remap, x, y) && producer(remap, owner, u, v, forest),
        (HolderProducerRef::GlobalCallsiteInput { call: a, holder: x, environment: u }, HolderProducerRef::GlobalCallsiteInput { call: b, holder: y, environment: v }) => a.range == b.range && source_pair(remap, &a.source, &b.source) && holder_place(remap, owner, x, y) && environment(remap, u, v),
        _ => false,
    }
}

fn key(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, a: &BoundaryResourceKey, b: &BoundaryResourceKey, forest: &SourceCaptureConstructionForest) -> bool {
    match (a, b) {
        (BoundaryResourceKey::Binding(a), BoundaryResourceKey::Binding(b)) => binding(remap, owner, a, b),
        (BoundaryResourceKey::Place(a), BoundaryResourceKey::Place(b)) => place(remap, owner, a, b),
        (BoundaryResourceKey::Loan(a), BoundaryResourceKey::Loan(b)) => loan(remap, owner, a, b),
        (BoundaryResourceKey::Runtime(a), BoundaryResourceKey::Runtime(b)) => runtime(remap, a, b),
        (BoundaryResourceKey::Environment(a), BoundaryResourceKey::Environment(b)) => environment(remap, a, b),
        (BoundaryResourceKey::Backing(a), BoundaryResourceKey::Backing(b)) => runtime(remap, &a.value, &b.value) && binding(remap, owner, &a.owner, &b.owner) && a.kind == b.kind && a.origin.range == b.origin.range && source_pair(remap, &a.origin.source, &b.origin.source),
        (BoundaryResourceKey::Holder(a), BoundaryResourceKey::Holder(b)) => environment(remap, &a.environment, &b.environment) && holder_place(remap, owner, &a.holder, &b.holder) && producer(remap, owner, &a.producer, &b.producer, forest),
        (BoundaryResourceKey::Origin(_), BoundaryResourceKey::Origin(_)) => false,
        _ => false,
    }
}

pub(super) fn install(remap: &mut ValueSchemaRemap, source: &original::OriginalResourceClosure, target: &original::OriginalResourceClosure, forest: &SourceCaptureConstructionForest) {
    for carrier in &source.originals {
        let cfg = carrier.original();
        let owner = remap.producer(&cfg.producer_owner).expect("actual original owner");
        let targets = target.originals.iter().filter(|row| row.original().producer_owner == owner).collect::<Vec<_>>();
        let [actual] = targets.as_slice() else { panic!("unique actual original target CFG"); };
        let origins = cfg.facts.iter().map(|fact| {
            let candidates = actual.original().facts.iter().enumerate().filter(|(_, counterpart)| match (fact, *counterpart) {
                (ReferenceOrigin::Fresh { allocation: a, loan: x }, ReferenceOrigin::Fresh { allocation: b, loan: y }) => a.function_span == b.function_span && a.declaration_span == b.declaration_span && a.creation_span == b.creation_span && source_pair(remap, &a.source, &b.source) && loan(remap, &cfg.producer_owner, x, y),
                (ReferenceOrigin::BorrowedFrom { parameter: a, loan: x }, ReferenceOrigin::BorrowedFrom { parameter: b, loan: y }) => a == b && loan(remap, &cfg.producer_owner, x, y),
                (ReferenceOrigin::Null { span: a }, ReferenceOrigin::Null { span: b }) => a == b,
                (ReferenceOrigin::Invalid { origin_span: a, conflict_span: x }, ReferenceOrigin::Invalid { origin_span: b, conflict_span: y }) => a == b && x == y,
                _ => false,
            }).collect::<Vec<_>>();
            let [(index, _)] = candidates.as_slice() else { panic!("unique full actual original origin constructor"); };
            ReferenceOriginId(*index)
        }).collect::<Vec<_>>();
        remap.imports.iter_mut().find(|row| row.producer == cfg.producer_owner).expect("complete original tagged import").origins = origins;
    }
    let mut pairs = Vec::new();
    for vertex in &source.vertices {
        if matches!(vertex.reference.key, BoundaryResourceKey::Origin(_)) { continue; }
        let owner = remap.producer(&vertex.reference.owner).expect("declared original owner pair");
        let candidates = target.vertices.iter().filter(|actual| actual.reference.owner == owner && key(remap, &vertex.reference.owner, &vertex.reference.key, &actual.reference.key, forest)).collect::<Vec<_>>();
        let [actual] = candidates.as_slice() else { panic!("unique actual independently inventoried typed resource constructor key"); };
        pairs.push((vertex.reference.clone(), actual.reference.clone()));
    }
    for (source, target) in &pairs {
        let import = remap.imports.iter_mut().find(|row| row.producer == source.owner).expect("source-tagged complete import");
        match (&source.key, &target.key) {
            (BoundaryResourceKey::Binding(a), BoundaryResourceKey::Binding(b)) => import.bindings.push((a.clone(), b.clone())),
            (BoundaryResourceKey::Place(a), BoundaryResourceKey::Place(b)) => import.places.push((a.clone(), b.clone())),
            (BoundaryResourceKey::Loan(a), BoundaryResourceKey::Loan(b)) => import.loans.push((a.clone(), b.clone())),
            (BoundaryResourceKey::Runtime(a), BoundaryResourceKey::Runtime(b)) => import.runtime_values.push((a.clone(), b.clone())),
            (BoundaryResourceKey::Environment(a), BoundaryResourceKey::Environment(b)) => import.environments.push((a.clone(), b.clone())),
            (BoundaryResourceKey::Backing(a), BoundaryResourceKey::Backing(b)) => import.backings.push((a.clone(), b.clone())),
            (BoundaryResourceKey::Holder(a), BoundaryResourceKey::Holder(b)) => import.holders.push((a.clone(), b.clone())),
            _ => unreachable!(),
        }
    }
    for vertex in &source.vertices {
        let counterpart = allocation::proposed(remap, &vertex.reference).expect("all genuine typed resource keys installed before payload lookup");
        let actual = target.vertices.iter().filter(|row| row.reference == counterpart).collect::<Vec<_>>();
        let [actual] = actual.as_slice() else { panic!("actual original full resource counterpart"); };
        let expected = copied::CopyResourceConstruction::paired(remap, &vertex.reference.owner, &counterpart.owner, forest).value(&vertex.value).expect("all dependency keys installed before full original payload traversal");
        assert_eq!(expected, actual.value, "WHOLE payload/parent/projection/creator ancestry;key candidate supplies zero success authority");
    }
}
