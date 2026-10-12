use super::*;

fn corresponding_type(before: &ScalarType, after: &ScalarType, structures: &[(ScalarStructId, ScalarStructId)], enumerations: &[(ScalarEnumId, ScalarEnumId)], source: &SourceCapturePolicyContext, target: &SourceCapturePolicyContext) -> bool {
    match (before, after) {
        (ScalarType::Struct(a), ScalarType::Struct(b)) => structures.contains(&(a.clone(), b.clone())) && source.structs.iter().filter(|row| row.id == *a).count() == 1 && target.structs.iter().filter(|row| row.id == *b).count() == 1,
        (ScalarType::Enum(a), ScalarType::Enum(b)) => enumerations.contains(&(a.clone(), b.clone())) && source.enums.iter().filter(|row| row.id == *a).count() == 1 && target.enums.iter().filter(|row| row.id == *b).count() == 1,
        (ScalarType::RawPointer(a), ScalarType::RawPointer(b)) => corresponding_type(a, b, structures, enumerations, source, target),
        (ScalarType::CheckedReference { mutability: a, inner: x }, ScalarType::CheckedReference { mutability: b, inner: y }) => a == b && corresponding_type(x, y, structures, enumerations, source, target),
        (ScalarType::Array { element: a, length: x, length_span: p, span: s }, ScalarType::Array { element: b, length: y, length_span: q, span: t }) => x == y && p == q && s == t && corresponding_type(a, b, structures, enumerations, source, target),
        (ScalarType::RuntimeArray { element: a, span: s }, ScalarType::RuntimeArray { element: b, span: t }) => s == t && corresponding_type(a, b, structures, enumerations, source, target),
        (ScalarType::Callable { outputs: a, parameters: x }, ScalarType::Callable { outputs: b, parameters: y }) => a.span == b.span && a.outputs.len() == b.outputs.len() && x.len() == y.len()
            && a.outputs.iter().zip(&b.outputs).all(|(a, b)| a.span == b.span && corresponding_type(&a.ty, &b.ty, structures, enumerations, source, target)) && x.iter().zip(y).all(|(a, b)| corresponding_type(a, b, structures, enumerations, source, target)),
        (ScalarType::Named { name: a, .. }, ScalarType::Named { name: b, .. }) => before == after && template::valid_context(source) && template::valid_context(target) && source.generic_scope == target.generic_scope && template::declaration_span(&source.generic_scope, &source.generic_declarations, a).is_some() && template::declaration_span(&source.generic_scope, &source.generic_declarations, a) == template::declaration_span(&target.generic_scope, &target.generic_declarations, b),
        (ScalarType::Qualified { .. } | ScalarType::Error, _) => false,
        _ => before == after,
    }
}

impl ValueSchemaRemap {
    pub(in crate::scalar) fn validate_qualified_shapes(&self, source: &[ValueTableSource], destination: &[ValueTableSource]) -> Result<(), ValueSchemaRemapError> {
        self.validate_qualified_header_shapes(&source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &destination.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>())
    }

    pub(super) fn validate_qualified_header_shapes(&self, source: &[BoundaryTableHeaders], destination: &[BoundaryTableHeaders]) -> Result<(), ValueSchemaRemapError> {
        self.checked_qualified_header_shapes(source, destination, &header_counterpart::BoundaryHeaderRoleInput::Transported).map(|_| ())
    }

    pub(super) fn checked_qualified_header_shapes(&self, source: &[BoundaryTableHeaders], destination: &[BoundaryTableHeaders], input: &header_counterpart::BoundaryHeaderRoleInput<'_>) -> Result<input_counterpart::BoundaryInputTypeEvidence, ValueSchemaRemapError> {
        let mut pairs = Vec::new();
        let mut structures = Vec::new();
        let mut enumerations = Vec::new();
        for original in source {
            let owner = self.boundary_producer(&original.producer)?;
            let targets = destination.iter().filter(|row| row.producer == owner).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner)); };
            for context in [&original.policy_context, &target.policy_context] {
                if !template::valid_context(context) { return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone())); }
                if context.structs.iter().any(|declaration| context.structs.iter().filter(|row| row.id == declaration.id).count() != 1)
                    || context.enums.iter().any(|declaration| context.enums.iter().filter(|row| row.id == declaration.id).count() != 1) { return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone())); }
            }
            for before in &original.tables.shapes {
                let id = self.boundary_shape(&before.id)?;
                if original.tables.shapes.iter().filter(|row| row.id == before.id).count() != 1 { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); }
                let targets = target.tables.shapes.iter().filter(|row| row.id == id).collect::<Vec<_>>();
                let [after] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                match (&before.ty, &before.payload, &after.ty, &after.payload) {
                    (ScalarType::Struct(a), ValuePayloadShape::Struct { structure: x, .. }, ScalarType::Struct(b), ValuePayloadShape::Struct { structure: y, .. }) if a == x && b == y => {
                        let pair = (a.clone(), b.clone()); if !structures.contains(&pair) { structures.push(pair); }
                    }
                    (ScalarType::Enum(a), ValuePayloadShape::Enum { enumeration: x, .. }, ScalarType::Enum(b), ValuePayloadShape::Enum { enumeration: y, .. }) if a == x && b == y => {
                        let pair = (a.clone(), b.clone()); if !enumerations.contains(&pair) { enumerations.push(pair); }
                    }
                    _ => {}
                }
                pairs.push((original, *target, before, *after));
            }
        }
        let evidence = input_counterpart::checked_header_types(self, source, destination, input)?;
        for (original, target, before, after) in pairs {
            let types = |a: &ScalarType, b: &ScalarType| match &evidence {
                input_counterpart::BoundaryInputTypeEvidence::Transported => corresponding_type(a, b, &structures, &enumerations, &original.policy_context, &target.policy_context),
                input_counterpart::BoundaryInputTypeEvidence::Independent(checked) => checked.type_pair(&original.producer, a, b),
            };
            let edge = |a: &ValueShapeId, b: &ValueShapeId, x: &ScalarType, y: &ScalarType| {
                Ok(self.boundary_shape(a)? == *b && original.tables.shapes.iter().filter(|row| row.id == *a && row.ty == *x).count() == 1
                    && target.tables.shapes.iter().filter(|row| row.id == *b && row.ty == *y).count() == 1 && types(x, y))
            };
            if !types(&before.ty, &after.ty) { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); }
            let valid = match (&before.payload, &after.payload) {
                (ValuePayloadShape::TemplateParameter { scope: a, declaration: x }, ValuePayloadShape::TemplateParameter { scope: b, declaration: y }) => {
                    let (ScalarType::Named { name: p, .. }, ScalarType::Named { name: q, .. }) = (&before.ty, &after.ty) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    a == b && x == y && before.ty == after.ty && a == &original.policy_context.generic_scope && b == &target.policy_context.generic_scope
                        && template::declaration_span(a, &original.policy_context.generic_declarations, p).as_ref() == Some(x)
                        && template::declaration_span(b, &target.policy_context.generic_declarations, q).as_ref() == Some(y)
                }
                (ValuePayloadShape::Plain(a), ValuePayloadShape::Plain(b)) => a == b && matches!((&before.ty, a),
                    (ScalarType::Unit, PlainValueKind::Unit) | (ScalarType::Bool, PlainValueKind::Bool) | (ScalarType::Char, PlainValueKind::Char) | (ScalarType::ArtifactId, PlainValueKind::ArtifactId)
                    | (ScalarType::I8, PlainValueKind::I8) | (ScalarType::I16, PlainValueKind::I16) | (ScalarType::I32, PlainValueKind::I32) | (ScalarType::I64, PlainValueKind::I64) | (ScalarType::I128, PlainValueKind::I128)
                    | (ScalarType::U8, PlainValueKind::U8) | (ScalarType::U16, PlainValueKind::U16) | (ScalarType::U32, PlainValueKind::U32) | (ScalarType::U64, PlainValueKind::U64) | (ScalarType::U128, PlainValueKind::U128)
                    | (ScalarType::F32, PlainValueKind::F32) | (ScalarType::F64, PlainValueKind::F64)),
                (ValuePayloadShape::Checked { mutability: a, referent: x }, ValuePayloadShape::Checked { mutability: b, referent: y }) => {
                    let (ScalarType::CheckedReference { mutability: p, inner: s }, ScalarType::CheckedReference { mutability: q, inner: t }) = (&before.ty, &after.ty) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    a == p && b == q && a == b && edge(x, y, s, t)?
                }
                (ValuePayloadShape::Raw { pointee: a }, ValuePayloadShape::Raw { pointee: b }) => {
                    let (ScalarType::RawPointer(x), ScalarType::RawPointer(y)) = (&before.ty, &after.ty) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    edge(a, b, x, y)?
                }
                (ValuePayloadShape::Callable { signature: a }, ValuePayloadShape::Callable { signature: b }) => {
                    let (ScalarType::Callable { outputs: x, parameters: p }, ScalarType::Callable { outputs: y, parameters: q }) = (a, b) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    let mut valid = before.ty == *a && after.ty == *b && types(a, b);
                    for (x, y) in p.iter().zip(q).chain(x.outputs.iter().zip(&y.outputs).map(|(x, y)| (&x.ty, &y.ty))) {
                        let originals = original.tables.shapes.iter().filter(|row| row.ty == *x).collect::<Vec<_>>();
                        let targets = target.tables.shapes.iter().filter(|row| row.ty == *y).collect::<Vec<_>>();
                        let ([original_shape], [target_shape]) = (originals.as_slice(), targets.as_slice()) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                        valid &= edge(&original_shape.id, &target_shape.id, x, y)?;
                    }
                    valid
                }
                (ValuePayloadShape::Struct { structure: a, fields: x }, ValuePayloadShape::Struct { structure: b, fields: y }) => {
                    let originals = original.policy_context.structs.iter().filter(|row| row.id == *a).collect::<Vec<_>>();
                    let targets = target.policy_context.structs.iter().filter(|row| row.id == *b).collect::<Vec<_>>();
                    let ([declaration], [receiving]) = (originals.as_slice(), targets.as_slice()) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    let mut valid = structures.contains(&(a.clone(), b.clone())) && before.ty == ScalarType::Struct(a.clone()) && after.ty == ScalarType::Struct(b.clone())
                        && declaration.copy_policy == receiving.copy_policy && declaration.copy_modifier_span == receiving.copy_modifier_span
                        && x.len() == declaration.fields.len() && y.len() == receiving.fields.len() && x.len() == y.len();
                    for field in x {
                        let declarations = declaration.fields.iter().filter(|row| row.id == field.field && row.id.structure == *a).collect::<Vec<_>>();
                        let [declared] = declarations.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(field.shape.clone())); };
                        let shapes = original.tables.shapes.iter().filter(|row| row.id == field.shape && row.ty == declared.ty).collect::<Vec<_>>();
                        let [_] = shapes.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(field.shape.clone())); };
                        let mapped_shape = self.boundary_shape(&field.shape)?;
                        let candidates = y.iter().filter(|field| field.shape == mapped_shape && receiving.fields.iter().any(|row| row.id == field.field && row.id.structure == *b && row.declaration_index == declared.declaration_index && types(&declared.ty, &row.ty))).collect::<Vec<_>>();
                        let [mapped] = candidates.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(field.shape.clone())); };
                        let fields = receiving.fields.iter().filter(|row| row.id == mapped.field && row.id.structure == *b).collect::<Vec<_>>();
                        let [receiving_field] = fields.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(field.shape.clone())); };
                        valid &= x.iter().filter(|row| row.field == field.field).count() == 1 && y.iter().filter(|row| row.field == mapped.field).count() == 1
                            && receiving.fields.iter().filter(|row| row.declaration_index == receiving_field.declaration_index).count() == 1
                            && declaration.fields.iter().filter(|row| row.declaration_index == declared.declaration_index).count() == 1
                            && target.tables.shapes.iter().filter(|row| row.id == mapped.shape && row.ty == receiving_field.ty).count() == 1;
                    }
                    for field in y { valid &= field.field.structure == *b && receiving.fields.iter().filter(|row| row.id == field.field).count() == 1; }
                    valid
                }
                (ValuePayloadShape::Enum { enumeration: a, variants: x }, ValuePayloadShape::Enum { enumeration: b, variants: y }) => {
                    let originals = original.policy_context.enums.iter().filter(|row| row.id == *a).collect::<Vec<_>>();
                    let targets = target.policy_context.enums.iter().filter(|row| row.id == *b).collect::<Vec<_>>();
                    let ([declaration], [receiving]) = (originals.as_slice(), targets.as_slice()) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    enumerations.contains(&(a.clone(), b.clone())) && before.ty == ScalarType::Enum(a.clone()) && after.ty == ScalarType::Enum(b.clone())
                        && declaration.copy_policy == receiving.copy_policy && declaration.copy_modifier_span == receiving.copy_modifier_span && declaration.variants == receiving.variants
                        && x.len() == declaration.variants.len() && y.len() == receiving.variants.len()
                        && x.iter().all(|variant| variant.fields.is_empty() && declaration.variants.iter().filter(|row| row.tag as usize == variant.ordinal).count() == 1 && x.iter().filter(|row| row.ordinal == variant.ordinal).count() == 1)
                        && y.iter().all(|variant| variant.fields.is_empty() && receiving.variants.iter().filter(|row| row.tag as usize == variant.ordinal).count() == 1 && y.iter().filter(|row| row.ordinal == variant.ordinal).count() == 1)
                }
                (ValuePayloadShape::FixedArray { element: a, length: x }, ValuePayloadShape::FixedArray { element: b, length: y }) => {
                    let (ScalarType::Array { element: s, length: p, .. }, ScalarType::Array { element: t, length: q, .. }) = (&before.ty, &after.ty) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    x == p && y == q && x == y && edge(a, b, s, t)?
                }
                (ValuePayloadShape::RuntimeArray { element: a }, ValuePayloadShape::RuntimeArray { element: b }) => {
                    let (ScalarType::RuntimeArray { element: x, .. }, ScalarType::RuntimeArray { element: y, .. }) = (&before.ty, &after.ty) else { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); };
                    edge(a, b, x, y)?
                }
                _ => false,
            };
            if !valid { return Err(ValueSchemaRemapError::MissingShape(before.id.clone())); }
        }
        Ok(evidence)
    }
}
