use super::*;

fn failure(root: &ValueShapeId) -> ValueSchemaRemapError { ValueSchemaRemapError::MissingShape(root.clone()) }

fn declaration_sources(owner: &ProducerCfgOwnerId, actual: &ProducerCfgOwnerId, source: &SourceIdentity, destination: &SourceIdentity) -> bool {
    if source == lexical(owner) { destination == lexical(actual) }
    else { source == destination }
}

fn struct_local(a: &ScalarStruct, b: &ScalarStruct) -> bool {
    a.name == b.name && a.name_span == b.name_span && a.span == b.span && a.copy_policy == b.copy_policy
        && a.copy_modifier_span == b.copy_modifier_span && a.layout == b.layout && a.fields.len() == b.fields.len()
}

fn enum_local(a: &ScalarEnum, b: &ScalarEnum) -> bool {
    a.name == b.name && a.name_span == b.name_span && a.span == b.span && a.copy_policy == b.copy_policy
        && a.copy_modifier_span == b.copy_modifier_span && a.variants == b.variants
        && a.variants.iter().all(|variant| a.variants.iter().filter(|row| row.name == variant.name || row.tag == variant.tag || row.name_span == variant.name_span).count() == 1)
}

fn field_local(a: &ScalarStructField, b: &ScalarStructField) -> bool {
    a.name == b.name && a.name_span == b.name_span && a.declaration_index == b.declaration_index && a.offset == b.offset && a.layout == b.layout
}

fn shared_shape(shape: &ResolvedValueShape, shapes: &[ResolvedValueShape], context: &SourceCapturePolicyContext) -> Result<(), ValueSchemaRemapError> {
    if !template::valid_context(context) || !template::valid_node(shape, context)
        || shapes.iter().filter(|row| row.id == shape.id).count() != 1 { return Err(failure(&shape.id)); }
    let edge = |id: &ValueShapeId, ty: &ScalarType| id.producer == shape.id.producer && shapes.iter().filter(|row| row.id == *id && row.ty == *ty).count() == 1;
    let valid = match (&shape.ty, &shape.payload) {
        (ScalarType::Unit, ValuePayloadShape::Plain(PlainValueKind::Unit)) | (ScalarType::Bool, ValuePayloadShape::Plain(PlainValueKind::Bool))
        | (ScalarType::Char, ValuePayloadShape::Plain(PlainValueKind::Char)) | (ScalarType::ArtifactId, ValuePayloadShape::Plain(PlainValueKind::ArtifactId))
        | (ScalarType::I8, ValuePayloadShape::Plain(PlainValueKind::I8)) | (ScalarType::I16, ValuePayloadShape::Plain(PlainValueKind::I16))
        | (ScalarType::I32, ValuePayloadShape::Plain(PlainValueKind::I32)) | (ScalarType::I64, ValuePayloadShape::Plain(PlainValueKind::I64))
        | (ScalarType::I128, ValuePayloadShape::Plain(PlainValueKind::I128)) | (ScalarType::U8, ValuePayloadShape::Plain(PlainValueKind::U8))
        | (ScalarType::U16, ValuePayloadShape::Plain(PlainValueKind::U16)) | (ScalarType::U32, ValuePayloadShape::Plain(PlainValueKind::U32))
        | (ScalarType::U64, ValuePayloadShape::Plain(PlainValueKind::U64)) | (ScalarType::U128, ValuePayloadShape::Plain(PlainValueKind::U128))
        | (ScalarType::F32, ValuePayloadShape::Plain(PlainValueKind::F32)) | (ScalarType::F64, ValuePayloadShape::Plain(PlainValueKind::F64)) => true,
        (ScalarType::RawPointer(ty), ValuePayloadShape::Raw { pointee }) => edge(pointee, ty),
        (ScalarType::CheckedReference { mutability, inner }, ValuePayloadShape::Checked { mutability: actual, referent }) => mutability == actual && edge(referent, inner),
        (ScalarType::Array { element: ty, length, .. }, ValuePayloadShape::FixedArray { element, length: actual }) => length == actual && edge(element, ty),
        (ScalarType::RuntimeArray { element: ty, .. }, ValuePayloadShape::RuntimeArray { element }) => edge(element, ty),
        (ScalarType::Callable { outputs, parameters }, ValuePayloadShape::Callable { signature }) => {
            shape.ty == *signature && parameters.iter().chain(outputs.outputs.iter().map(|row| &row.ty)).all(|ty| shapes.iter().filter(|row| row.id.producer == shape.id.producer && row.ty == *ty).count() == 1)
        }
        (ScalarType::Struct(id), ValuePayloadShape::Struct { structure, fields }) => {
            let declarations = context.structs.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
            let [declaration] = declarations.as_slice() else { return Err(failure(&shape.id)); };
            id == structure && fields.len() == declaration.fields.len() && fields.iter().zip(&declaration.fields).all(|(field, declared)| {
                field.field == declared.id && declared.id.structure == *id && edge(&field.shape, &declared.ty)
                    && fields.iter().filter(|row| row.field == field.field).count() == 1
            })
        }
        (ScalarType::Enum(id), ValuePayloadShape::Enum { enumeration, variants }) => {
            let declarations = context.enums.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
            let [declaration] = declarations.as_slice() else { return Err(failure(&shape.id)); };
            id == enumeration && variants.len() == declaration.variants.len() && variants.iter().zip(&declaration.variants).all(|(variant, declared)| {
                variant.ordinal == declared.tag as usize && variant.fields.is_empty() && variants.iter().filter(|row| row.ordinal == variant.ordinal).count() == 1
            })
        }
        (ScalarType::Named { name, .. }, ValuePayloadShape::TemplateParameter { scope, declaration }) => scope == &context.generic_scope && template::declaration_span(scope, &context.generic_declarations, name).as_ref() == Some(declaration),
        _ => false,
    };
    if valid { Ok(()) } else { Err(failure(&shape.id)) }
}

pub(super) fn roots(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, source: &ValueTableSource, destination: &ValueTableSource, qualification: &BoundaryInputQualification) -> Result<(), ValueSchemaRemapError> {
    qualification.validate(source, destination)?;
    let cfg = source.resource_construction.original();
    let actual = destination.resource_construction.original();
    let mut work = Vec::new();
    for shape in &cfg.evaluated_values.shapes {
        let id = qualification.shape(remap, &shape.id)?;
        let targets = actual.evaluated_values.shapes.iter().filter(|row| row.id == id).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(failure(&shape.id)); };
        if matches!(qualification, BoundaryInputQualification::SharedOriginal { .. }) { shared_shape(shape, &cfg.evaluated_values.shapes, &source.policy_context)?; }
        let record = BoundaryShapeTypeCounterpart { owner: source.producer.clone(), source: shape.clone(), destination: (*target).clone() };
        let existing = checked.shapes.iter().filter(|row| row.owner == record.owner && row.source.id == record.source.id).collect::<Vec<_>>();
        match existing.as_slice() {
            [] => checked.shapes.push(record),
            [old] if **old == record => {},
            _ => return Err(failure(&shape.id)),
        }
        work.push((shape.ty.clone(), target.ty.clone(), shape.id.clone()));
    }
    for production in &cfg.evaluated_values.source_capture_productions {
        let event = qualification.point(remap, &production.event)?;
        let targets = actual.evaluated_values.source_capture_productions.iter().filter(|row| row.event == event).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: production.event.owner.clone(), point: production.event.point }); };
        let values = cfg.evaluated_values.values.iter().filter(|row| row.id == production.output).collect::<Vec<_>>();
        let [value] = values.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: production.event.owner.clone(), point: production.event.point }); };
        let mapped_shape = qualification.shape(remap, &value.shape)?;
        let actual_values = actual.evaluated_values.values.iter().filter(|row| row.id == target.output && row.shape == mapped_shape).collect::<Vec<_>>();
        let [_] = actual_values.as_slice() else { return Err(failure(&value.shape)); };
        if production.inputs.len() != target.inputs.len() { return Err(failure(&value.shape)); }
        work.push((production.signature.clone(), target.signature.clone(), value.shape.clone()));
        for (before, after) in production.inputs.iter().zip(&target.inputs) {
            work.push((before.capture.ty.clone(), after.capture.ty.clone(), before.shape.clone()));
            work.push((before.capture.place_type.clone(), after.capture.place_type.clone(), before.shape.clone()));
        }
    }
    extend(checked, source, destination, qualification, work)
}

fn extend(checked: &mut CheckedBoundaryInputTypes, source: &ValueTableSource, destination: &ValueTableSource, qualification: &BoundaryInputQualification, mut work: Vec<(ScalarType, ScalarType, ValueShapeId)>) -> Result<(), ValueSchemaRemapError> {
    qualification.validate(source, destination)?;
    let a = &source.policy_context;
    let b = &destination.policy_context;
    let mut reserved = Vec::new();
    let mut visited = Vec::new();
    let mut cursor = 0;
    while cursor < work.len() {
        let (before, after, root) = work[cursor].clone();
        cursor += 1;
        let record = BoundaryTypeCounterpart { owner: source.producer.clone(), source: before.clone(), destination: after.clone() };
        let existing = reserved.iter().chain(&checked.types).filter(|row: &&BoundaryTypeCounterpart| row.owner == record.owner && row.source == record.source).collect::<Vec<_>>();
        if !existing.is_empty() {
            if existing.iter().any(|row| **row != record) { return Err(failure(&root)); }
        }
        if visited.contains(&(record.clone(), root.clone())) { continue; }
        visited.push((record.clone(), root.clone()));
        if existing.is_empty() { reserved.push(record); }
        match (&before, &after) {
            (ScalarType::Struct(x), ScalarType::Struct(y)) => {
                let originals = a.structs.iter().filter(|row| row.id == *x).collect::<Vec<_>>();
                let targets = b.structs.iter().filter(|row| row.id == *y).collect::<Vec<_>>();
                let ([original], [actual]) = (originals.as_slice(), targets.as_slice()) else { return Err(failure(&root)); };
                if !declaration_sources(&source.producer, &destination.producer, &x.source, &y.source) || !struct_local(original, actual)
                    || (x.source != *lexical(&source.producer) && *original != *actual) { return Err(failure(&root)); }
                let mut fields = Vec::new();
                for (field, receiving) in original.fields.iter().zip(&actual.fields) {
                    if field.id.structure != *x || receiving.id.structure != *y || !field_local(field, receiving)
                        || original.fields.iter().filter(|row| row.id == field.id || row.declaration_index == field.declaration_index).count() != 1
                        || actual.fields.iter().filter(|row| row.id == receiving.id || row.declaration_index == receiving.declaration_index).count() != 1 { return Err(failure(&root)); }
                    fields.push(BoundaryFieldCounterpart { owner: source.producer.clone(), source: field.clone(), destination: receiving.clone() });
                    work.push((field.ty.clone(), receiving.ty.clone(), root.clone()));
                }
                let nominal = BoundaryNominalCounterpart::Struct { owner: source.producer.clone(), source: (*original).clone(), destination: (*actual).clone(), fields: fields.clone() };
                retain_nominal(checked, nominal, &root)?;
                for field in fields { if !checked.fields.contains(&field) { checked.fields.push(field); } }
            }
            (ScalarType::Enum(x), ScalarType::Enum(y)) => {
                let originals = a.enums.iter().filter(|row| row.id == *x).collect::<Vec<_>>();
                let targets = b.enums.iter().filter(|row| row.id == *y).collect::<Vec<_>>();
                let ([original], [actual]) = (originals.as_slice(), targets.as_slice()) else { return Err(failure(&root)); };
                if !declaration_sources(&source.producer, &destination.producer, &x.source, &y.source) || !enum_local(original, actual)
                    || (x.source != *lexical(&source.producer) && *original != *actual) { return Err(failure(&root)); }
                retain_nominal(checked, BoundaryNominalCounterpart::Enum { owner: source.producer.clone(), source: (*original).clone(), destination: (*actual).clone() }, &root)?;
            }
            (ScalarType::RawPointer(x), ScalarType::RawPointer(y)) => work.push(((**x).clone(), (**y).clone(), root)),
            (ScalarType::CheckedReference { mutability: x, inner: p }, ScalarType::CheckedReference { mutability: y, inner: q }) if x == y => work.push(((**p).clone(), (**q).clone(), root)),
            (ScalarType::Array { element: x, length: p, length_span: s, span: u }, ScalarType::Array { element: y, length: q, length_span: t, span: v }) if p == q && s == t && u == v => work.push(((**x).clone(), (**y).clone(), root)),
            (ScalarType::RuntimeArray { element: x, span: p }, ScalarType::RuntimeArray { element: y, span: q }) if p == q => work.push(((**x).clone(), (**y).clone(), root)),
            (ScalarType::Callable { outputs: x, parameters: p }, ScalarType::Callable { outputs: y, parameters: q }) if x.span == y.span && x.outputs.len() == y.outputs.len() && p.len() == q.len() => {
                for (x, y) in p.iter().zip(q) { work.push((x.clone(), y.clone(), root.clone())); }
                for (x, y) in x.outputs.iter().zip(&y.outputs) {
                    if x.span != y.span { return Err(failure(&root)); }
                    work.push((x.ty.clone(), y.ty.clone(), root.clone()));
                }
            }
            (ScalarType::Named { name: x, span }, ScalarType::Named { name: y, .. }) => {
                if span.start >= span.end || before != after || !template::valid_context(a) || !template::valid_context(b) || a.generic_scope != b.generic_scope
                    || template::declaration_span(&a.generic_scope, &a.generic_declarations, x).is_none()
                    || template::declaration_span(&a.generic_scope, &a.generic_declarations, x) != template::declaration_span(&b.generic_scope, &b.generic_declarations, y) { return Err(failure(&root)); }
            }
            (ScalarType::Unit, ScalarType::Unit) | (ScalarType::Bool, ScalarType::Bool) | (ScalarType::Char, ScalarType::Char) | (ScalarType::ArtifactId, ScalarType::ArtifactId)
            | (ScalarType::I8, ScalarType::I8) | (ScalarType::I16, ScalarType::I16) | (ScalarType::I32, ScalarType::I32) | (ScalarType::I64, ScalarType::I64) | (ScalarType::I128, ScalarType::I128)
            | (ScalarType::U8, ScalarType::U8) | (ScalarType::U16, ScalarType::U16) | (ScalarType::U32, ScalarType::U32) | (ScalarType::U64, ScalarType::U64) | (ScalarType::U128, ScalarType::U128)
            | (ScalarType::F32, ScalarType::F32) | (ScalarType::F64, ScalarType::F64) => {},
            _ => return Err(failure(&root)),
        }
    }
    for record in reserved { checked.types.push(record); }
    Ok(())
}

fn retain_nominal(checked: &mut CheckedBoundaryInputTypes, nominal: BoundaryNominalCounterpart, root: &ValueShapeId) -> Result<(), ValueSchemaRemapError> {
    let demand = BoundaryNominalDemand { counterpart: nominal.clone(), root: root.clone() };
    let owner = match &nominal { BoundaryNominalCounterpart::Struct { owner, .. } | BoundaryNominalCounterpart::Enum { owner, .. } => owner };
    let original = carrier(&checked.source_forest, owner)?;
    if root.producer != *owner || original.resource_construction.original().evaluated_values.shapes.iter().filter(|shape| shape.id == *root).count() != 1 { return Err(failure(root)); }
    for row in &checked.nominals {
        let coherent = match (row, &nominal) {
            (BoundaryNominalCounterpart::Struct { source: x, destination: p, .. }, BoundaryNominalCounterpart::Struct { source: y, destination: q, .. }) => x.id != y.id || (x == y && p == q),
            (BoundaryNominalCounterpart::Enum { source: x, destination: p, .. }, BoundaryNominalCounterpart::Enum { source: y, destination: q, .. }) => x.id != y.id || (x == y && p == q),
            _ => true,
        };
        if !coherent { return Err(failure(root)); }
        let same = match (row, &nominal) {
            (BoundaryNominalCounterpart::Struct { owner: a, source: x, .. }, BoundaryNominalCounterpart::Struct { owner: b, source: y, .. }) => a == b && x.id == y.id,
            (BoundaryNominalCounterpart::Enum { owner: a, source: x, .. }, BoundaryNominalCounterpart::Enum { owner: b, source: y, .. }) => a == b && x.id == y.id,
            _ => false,
        };
        if same {
            if *row != nominal { return Err(failure(root)); }
            if !checked.nominal_demands.contains(&demand) { checked.nominal_demands.push(demand); }
            return Ok(());
        }
    }
    checked.nominals.push(nominal);
    checked.nominal_demands.push(demand);
    Ok(())
}

pub(super) fn extend_original_types(remap: &ValueSchemaRemap, checked: &CheckedBoundaryInputTypes, owner: &ProducerCfgOwnerId, source: &[ScalarType], destination: &[ScalarType], root: &ValueShapeId) -> Result<CheckedBoundaryInputTypes, ValueSchemaRemapError> {
    let original = carrier(&checked.source_forest, owner)?;
    let qualification = checked.qualification(remap, owner)?;
    let actual = carrier(&checked.destination_forest, qualification.destination())?;
    if root.producer != *owner || source.len() != destination.len()
        || original.resource_construction.original().evaluated_values.shapes.iter().filter(|row| row.id == *root).count() != 1
        || !checked.originals.iter().any(|row| row.owner == *owner && row.construction == original.resource_construction && row.policy_context == original.policy_context)
        || !checked.originals.iter().any(|row| row.owner == actual.producer && row.construction == actual.resource_construction && row.policy_context == actual.policy_context) { return Err(failure(root)); }
    let mut extended = checked.clone();
    extend(&mut extended, original, actual, &qualification, source.iter().zip(destination).map(|(a, b)| (a.clone(), b.clone(), root.clone())).collect())?;
    Ok(extended)
}

pub(super) fn merge(checked: &mut CheckedBoundaryInputTypes, demands: &[BoundaryNominalDemand], fields: &[BoundaryFieldCounterpart], types: &[BoundaryTypeCounterpart], shapes: &[BoundaryShapeTypeCounterpart]) -> Result<(), ValueSchemaRemapError> {
    for shape in shapes {
        let rows = checked.shapes.iter().filter(|row| row.owner == shape.owner && row.source.id == shape.source.id).collect::<Vec<_>>();
        match rows.as_slice() { [] => checked.shapes.push(shape.clone()), [old] if **old == *shape => {}, _ => return Err(failure(&shape.source.id)) }
    }
    for demand in demands {
        retain_nominal(checked, demand.counterpart.clone(), &demand.root)?;
    }
    for field in fields { if !checked.fields.contains(field) { checked.fields.push(field.clone()); } }
    for record in types {
        let rows = checked.types.iter().filter(|row| row.owner == record.owner && row.source == record.source).collect::<Vec<_>>();
        match rows.as_slice() { [] => checked.types.push(record.clone()), [old] if **old == *record => {}, _ => return Err(ValueSchemaRemapError::MissingProducer(record.owner.clone())) }
    }
    Ok(())
}
