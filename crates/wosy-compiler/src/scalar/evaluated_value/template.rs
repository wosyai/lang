use super::*;

pub(in crate::scalar) fn valid_declarations(scope: &ProducerCfgOwnerId, declarations: &[ScalarGenericParameter]) -> bool {
    match scope {
        ProducerCfgOwnerId::Module(_) => declarations.is_empty(),
        ProducerCfgOwnerId::Function { .. } => declarations.iter().all(|declaration| {
            declaration.span.start < declaration.span.end
                && declarations.iter().filter(|row| row.name == declaration.name).count() == 1
                && declarations.iter().filter(|row| row.span == declaration.span).count() == 1
        }),
        ProducerCfgOwnerId::Literal { .. } => false,
    }
}

pub(in crate::scalar) fn declaration_span(scope: &ProducerCfgOwnerId, declarations: &[ScalarGenericParameter], name: &str) -> Option<SourceSpan> {
    if !valid_declarations(scope, declarations) { return None; }
    let ProducerCfgOwnerId::Function { source, .. } = scope else { return None; };
    let rows = declarations.iter().filter(|row| row.name == name).collect::<Vec<_>>();
    let [declaration] = rows.as_slice() else { return None; };
    Some(SourceSpan::new(source.clone(), declaration.span))
}

pub(in crate::scalar) fn valid_context(context: &SourceCapturePolicyContext) -> bool {
    valid_declarations(&context.generic_scope, &context.generic_declarations)
        && context.generic_parameters.len() == context.generic_declarations.len()
        && context.generic_parameters == context.generic_declarations.iter().map(|row| row.name.clone()).collect::<BTreeSet<_>>()
}

pub(in crate::scalar) fn validate_shapes(shapes: &[ResolvedValueShape], producer: &ProducerCfgOwnerId, scope: &ProducerCfgOwnerId, declarations: &[ScalarGenericParameter]) -> Result<(), ValueShapeError> {
    for shape in shapes.iter().filter(|row| row.id.producer == *producer) {
        match (&shape.ty, &shape.payload) {
            (ScalarType::Named { name, span }, ValuePayloadShape::TemplateParameter { scope: actual, declaration })
                if span.start < span.end && actual == scope && declaration_span(scope, declarations, name).as_ref() == Some(declaration) => {}
            (ScalarType::Named { .. } | ScalarType::Qualified { .. } | ScalarType::Error, _)
                | (_, ValuePayloadShape::TemplateParameter { .. }) => return Err(ValueShapeError::UnresolvedType(shape.ty.clone())),
            _ => {}
        }
        validate_leaves(&shape.ty, shapes, producer, scope, declarations)?;
        validate_generic_edges(shape, shapes, scope, declarations)?;
    }
    Ok(())
}

pub(in crate::scalar) fn validate_type_input(ty: &ScalarType, structs: &[ScalarStruct], enums: &[ScalarEnum], scope: &ProducerCfgOwnerId, declarations: &[ScalarGenericParameter], visited: &mut Vec<ScalarType>) -> Result<(), ValueShapeError> {
    if visited.contains(ty) { return Ok(()); }
    visited.push(ty.clone());
    match ty {
        ScalarType::Named { name, span } => {
            if span.start >= span.end || declaration_span(scope, declarations, name).is_none() { return Err(ValueShapeError::UnresolvedType(ty.clone())); }
        }
        ScalarType::Qualified { .. } | ScalarType::Error => return Err(ValueShapeError::UnresolvedType(ty.clone())),
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => validate_type_input(inner, structs, enums, scope, declarations, visited)?,
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => validate_type_input(element, structs, enums, scope, declarations, visited)?,
        ScalarType::Callable { parameters, outputs } => {
            for output in &outputs.outputs { validate_type_input(&output.ty, structs, enums, scope, declarations, visited)?; }
            for ty in parameters { validate_type_input(ty, structs, enums, scope, declarations, visited)?; }
        }
        ScalarType::Struct(id) => {
            let rows = structs.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
            let [declaration] = rows.as_slice() else { return Err(ValueShapeError::MissingStruct(id.clone())); };
            for field in &declaration.fields { validate_type_input(&field.ty, structs, enums, scope, declarations, visited)?; }
        }
        ScalarType::Enum(id) => {
            if enums.iter().filter(|row| row.id == *id).count() != 1 { return Err(ValueShapeError::MissingEnum(id.clone())); }
        }
        _ => {}
    }
    Ok(())
}

pub(in crate::scalar) fn validate_reused_nominals(ty: &ScalarType, shapes: &[ResolvedValueShape], producer: &ProducerCfgOwnerId, structs: &[ScalarStruct], enums: &[ScalarEnum], visited: &mut Vec<ScalarType>) -> Result<(), ValueShapeError> {
    if visited.contains(ty) { return Ok(()); }
    visited.push(ty.clone());
    let fail = || ValueShapeError::UnresolvedType(ty.clone());
    let rows = shapes.iter().filter(|row| row.id.producer == *producer && scalar_type_equal(&row.ty, ty)).collect::<Vec<_>>();
    if rows.len() > 1 { return Err(fail()); }
    match ty {
        ScalarType::Struct(id) => {
            let declared = structs.iter().find(|row| row.id == *id).ok_or_else(|| ValueShapeError::MissingStruct(id.clone()))?;
            if let [shape] = rows.as_slice() {
                let ValuePayloadShape::Struct { structure, fields } = &shape.payload else { return Err(fail()); };
                if structure != id || fields.len() != declared.fields.len() { return Err(fail()); }
                for (field, original) in fields.iter().zip(&declared.fields) {
                    if field.field != original.id || original.id.structure != *id
                        || field.shape.producer != *producer || fields.iter().filter(|row| row.field == field.field).count() != 1
                        || shapes.iter().filter(|row| row.id == field.shape && scalar_type_equal(&row.ty, &original.ty)).count() != 1 { return Err(fail()); }
                }
            }
            for field in &declared.fields { validate_reused_nominals(&field.ty, shapes, producer, structs, enums, visited)?; }
        }
        ScalarType::Enum(id) => {
            let declared = enums.iter().find(|row| row.id == *id).ok_or_else(|| ValueShapeError::MissingEnum(id.clone()))?;
            if let [shape] = rows.as_slice() {
                let ValuePayloadShape::Enum { enumeration, variants } = &shape.payload else { return Err(fail()); };
                if enumeration != id || variants.len() != declared.variants.len() { return Err(fail()); }
                for (variant, original) in variants.iter().zip(&declared.variants) {
                    if variant.ordinal != original.tag as usize || !variant.fields.is_empty()
                        || variants.iter().filter(|row| row.ordinal == variant.ordinal).count() != 1 { return Err(fail()); }
                }
            }
        }
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => validate_reused_nominals(inner, shapes, producer, structs, enums, visited)?,
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => validate_reused_nominals(element, shapes, producer, structs, enums, visited)?,
        ScalarType::Callable { parameters, outputs } => {
            for output in &outputs.outputs { validate_reused_nominals(&output.ty, shapes, producer, structs, enums, visited)?; }
            for ty in parameters { validate_reused_nominals(ty, shapes, producer, structs, enums, visited)?; }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub(in crate::scalar) fn valid_node(shape: &ResolvedValueShape, context: &SourceCapturePolicyContext) -> bool {
    if !valid_abstract_type(&shape.ty, context) { return false; }
    match (&shape.ty, &shape.payload) {
        (ScalarType::Named { name, span }, ValuePayloadShape::TemplateParameter { scope, declaration }) => {
            valid_context(context) && span.start < span.end && scope == &context.generic_scope
                && declaration_span(scope, &context.generic_declarations, name).as_ref() == Some(declaration)
        }
        (ScalarType::Named { .. } | ScalarType::Qualified { .. } | ScalarType::Error, _)
            | (_, ValuePayloadShape::TemplateParameter { .. }) => false,
        _ => true,
    }
}

fn valid_abstract_type(ty: &ScalarType, context: &SourceCapturePolicyContext) -> bool {
    match ty {
        ScalarType::Named { name, span } => span.start < span.end && declaration_span(&context.generic_scope, &context.generic_declarations, name).is_some(),
        ScalarType::Qualified { .. } | ScalarType::Error => false,
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => valid_abstract_type(inner, context),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => valid_abstract_type(element, context),
        ScalarType::Callable { parameters, outputs } => parameters.iter().all(|ty| valid_abstract_type(ty, context)) && outputs.outputs.iter().all(|row| valid_abstract_type(&row.ty, context)),
        _ => true,
    }
}

pub(in crate::scalar) fn validate_graph_node(shape: &ResolvedValueShape, shapes: &[ResolvedValueShape], context: &SourceCapturePolicyContext) -> Result<(), ValueShapeError> {
    validate_leaves(&shape.ty, shapes, &shape.id.producer, &context.generic_scope, &context.generic_declarations)?;
    validate_generic_edges(shape, shapes, &context.generic_scope, &context.generic_declarations)
}

fn generic_type(ty: &ScalarType) -> bool {
    match ty {
        ScalarType::Named { .. } => true,
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => generic_type(inner),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => generic_type(element),
        ScalarType::Callable { parameters, outputs } => parameters.iter().any(generic_type) || outputs.outputs.iter().any(|row| generic_type(&row.ty)),
        _ => false,
    }
}

fn validate_generic_edges(shape: &ResolvedValueShape, shapes: &[ResolvedValueShape], scope: &ProducerCfgOwnerId, declarations: &[ScalarGenericParameter]) -> Result<(), ValueShapeError> {
    let fail = || ValueShapeError::UnresolvedType(shape.ty.clone());
    let edge = |id: &ValueShapeId, ty: &ScalarType| {
        let rows = shapes.iter().filter(|row| row.id == *id && row.ty == *ty).collect::<Vec<_>>();
        let [child] = rows.as_slice() else { return Err(fail()); };
        validate_leaves(&child.ty, shapes, &child.id.producer, scope, declarations)
    };
    match (&shape.ty, &shape.payload) {
        (ScalarType::RawPointer(ty), ValuePayloadShape::Raw { pointee }) if generic_type(ty) => edge(pointee, ty)?,
        (ScalarType::CheckedReference { mutability, inner }, ValuePayloadShape::Checked { mutability: actual, referent }) if generic_type(inner) => {
            if mutability != actual { return Err(fail()); }
            edge(referent, inner)?;
        }
        (ScalarType::Array { element: ty, length, .. }, ValuePayloadShape::FixedArray { element, length: actual }) if generic_type(ty) => {
            if length != actual { return Err(fail()); }
            edge(element, ty)?;
        }
        (ScalarType::RuntimeArray { element: ty, .. }, ValuePayloadShape::RuntimeArray { element }) if generic_type(ty) => edge(element, ty)?,
        (ScalarType::Callable { .. }, ValuePayloadShape::Callable { signature }) if generic_type(&shape.ty) => {
            if signature != &shape.ty { return Err(fail()); }
        }
        (ScalarType::Named { .. }, ValuePayloadShape::TemplateParameter { .. }) => {}
        (ty, _) if generic_type(ty) => return Err(fail()),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::scalar) fn concrete_contexts(owners: &[ProducerCfgOwnerId], structs: &[ScalarStruct], enums: &[ScalarEnum]) -> Vec<SourceCapturePolicyContext> {
    owners.iter().map(|owner| {
        let mut scope = owner;
        while let ProducerCfgOwnerId::Literal { enclosing, .. } = scope { scope = enclosing; }
        SourceCapturePolicyContext {
            structs: structs.to_vec(), enums: enums.to_vec(), generic_scope: scope.clone(),
            generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(),
        }
    }).collect()
}

fn validate_leaves(ty: &ScalarType, shapes: &[ResolvedValueShape], producer: &ProducerCfgOwnerId, scope: &ProducerCfgOwnerId, declarations: &[ScalarGenericParameter]) -> Result<(), ValueShapeError> {
    match ty {
        ScalarType::Named { name, .. } => {
            let rows = shapes.iter().filter(|row| row.id.producer == *producer && row.ty == *ty).collect::<Vec<_>>();
            let [shape] = rows.as_slice() else { return Err(ValueShapeError::UnresolvedType(ty.clone())); };
            let ValuePayloadShape::TemplateParameter { scope: actual, declaration } = &shape.payload else { return Err(ValueShapeError::UnresolvedType(ty.clone())); };
            if actual != scope || declaration_span(scope, declarations, name).as_ref() != Some(declaration) {
                return Err(ValueShapeError::UnresolvedType(ty.clone()));
            }
        }
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => validate_leaves(inner, shapes, producer, scope, declarations)?,
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => validate_leaves(element, shapes, producer, scope, declarations)?,
        ScalarType::Callable { parameters, outputs } => {
            for ty in parameters { validate_leaves(ty, shapes, producer, scope, declarations)?; }
            for output in &outputs.outputs { validate_leaves(&output.ty, shapes, producer, scope, declarations)?; }
        }
        ScalarType::Qualified { .. } | ScalarType::Error => return Err(ValueShapeError::UnresolvedType(ty.clone())),
        _ => {}
    }
    Ok(())
}
