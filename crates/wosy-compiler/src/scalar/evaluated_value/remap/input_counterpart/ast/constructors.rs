use super::*;

fn point<'a>(visitor: &InputVisitor<'_>, cfg: &'a ScalarReferenceCfg, id: CfgPointId) -> Result<&'a ReferenceFlowPoint, ValueSchemaRemapError> {
    let rows = cfg.initial_points.iter().filter(|point| point.id == id).collect::<Vec<_>>();
    let [point] = rows.as_slice() else { return Err(visitor.invalid()); };
    Ok(point)
}

fn pair(visitor: &InputVisitor<'_>, source: &ReferenceFlowPoint, destination: &ReferenceFlowPoint) -> Result<(), ValueSchemaRemapError> {
    visitor.require(visitor.point(source.id)? == destination.id && visitor.span(&source.source_span, &destination.source_span) && source.scope == destination.scope
        && std::mem::discriminant(&source.kind) == std::mem::discriminant(&destination.kind) && source.successors.len() == destination.successors.len())?;
    for (a, b) in source.successors.iter().zip(&destination.successors) { visitor.require(visitor.point(*a)? == *b)?; }
    Ok(())
}

fn place(visitor: &mut InputVisitor<'_>, source: &ReferencePlaceId, destination: &ReferencePlaceId) -> Result<(), ValueSchemaRemapError> {
    visitor.require(visitor.binding_reference(&source.binding)? == destination.binding && source.function_span == destination.function_span
        && source.declaration_span == destination.declaration_span && source.block_span == destination.block_span && source.projections.len() == destination.projections.len())?;
    for (a, b) in source.projections.iter().zip(&destination.projections) {
        match (a, b) {
            (ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(a)), ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(b))) => visitor.field(&ScalarFieldReference::Resolved(a.clone()), &ScalarFieldReference::Resolved(b.clone()))?,
            (ReferencePlaceProjection::Index(a), ReferencePlaceProjection::Index(b)) => visitor.expression(a, b)?,
            (ReferencePlaceProjection::Dereference(a), ReferencePlaceProjection::Dereference(b)) => visitor.require(visitor.remap.checked_loan(&visitor.source.producer_owner, a)? == **b)?,
            _ => return Err(visitor.invalid()),
        }
    }
    Ok(())
}

fn optional_point(visitor: &InputVisitor<'_>, source: &Option<CfgPointId>, destination: &Option<CfgPointId>) -> Result<(), ValueSchemaRemapError> {
    match (source, destination) { (None, None) => Ok(()), (Some(a), Some(b)) => visitor.require(visitor.point(*a)? == *b), _ => Err(visitor.invalid()) }
}

fn read_place(visitor: &mut InputVisitor<'_>, source: &ReferenceFlowPoint, destination: &ReferenceFlowPoint) -> Result<(), ValueSchemaRemapError> {
    pair(visitor, source, destination)?;
    let (CfgPointKind::ReadPlace { place: a, pointer_point: p, candidates: x, reads_origin: m }, CfgPointKind::ReadPlace { place: b, pointer_point: q, candidates: y, reads_origin: n }) = (&source.kind, &destination.kind) else { return Err(visitor.invalid()); };
    visitor.require(m == n && x.len() == y.len())?;
    optional_point(visitor, p, q)?;
    visitor.place(a, b)?;
    for (a, b) in x.iter().zip(y) { place(visitor, a, b)?; }
    Ok(())
}

pub(super) fn read(visitor: &mut InputVisitor<'_>, source: &ScalarExpression, destination: &ScalarExpression, occurrence: &ScalarReadOccurrence, actual: &ScalarReadOccurrence) -> Result<(), ValueSchemaRemapError> {
    let role = match source { ScalarExpression::Dereference { .. } => ScalarReadRole::Dereference, ScalarExpression::IndexedRead { .. } => ScalarReadRole::IndexedRead, _ => return Err(visitor.invalid()) };
    visitor.require(occurrence.role == role && actual.role == role && std::mem::discriminant(source) == std::mem::discriminant(destination))?;
    let a = visitor.source.read_points.iter().filter(|row| row.occurrence == *occurrence).collect::<Vec<_>>();
    let b = visitor.destination.read_points.iter().filter(|row| row.occurrence == *actual).collect::<Vec<_>>();
    if a.is_empty() && b.is_empty() {
        let originals = visitor.source.initial_points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Evaluate(expression) if expression == source)).collect::<Vec<_>>();
        let actuals = visitor.destination.initial_points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Evaluate(expression) if expression == destination)).collect::<Vec<_>>();
        let ([original], [actual]) = (originals.as_slice(), actuals.as_slice()) else { return Err(visitor.invalid()); };
        pair(visitor, original, actual)?;
        return outputs::point_values(visitor, original.id, actual.id);
    }
    let ([a], [b]) = (a.as_slice(), b.as_slice()) else { return Err(visitor.invalid()); };
    visitor.require(a.function_span == b.function_span && a.indices.len() == b.indices.len())?;
    let (ScalarExpression::Dereference { place: source_place, .. } | ScalarExpression::IndexedRead { place: source_place, .. }, ScalarExpression::Dereference { place: actual_place, .. } | ScalarExpression::IndexedRead { place: actual_place, .. }) = (source, destination) else { return Err(visitor.invalid()); };
    let original_point = point(visitor, visitor.source, a.point)?;
    let actual_point = point(visitor, visitor.destination, b.point)?;
    visitor.require(matches!(&original_point.kind, CfgPointKind::ReadPlace { place, .. } if place == source_place) && matches!(&actual_point.kind, CfgPointKind::ReadPlace { place, .. } if place == actual_place))?;
    read_place(visitor, original_point, actual_point)?;
    let source_indices = indices(source_place);
    let actual_indices = indices(actual_place);
    visitor.require(source_indices.len() == a.indices.len() && actual_indices.len() == b.indices.len())?;
    for (registration, (span, expression)) in a.indices.iter().zip(source_indices) {
        visitor.require(registration.source_span.range == span)?;
        operand(visitor, visitor.source, expression, registration.point)?;
    }
    for (registration, (span, expression)) in b.indices.iter().zip(actual_indices) {
        visitor.require(registration.source_span.range == span)?;
        operand(visitor, visitor.destination, expression, registration.point)?;
    }
    for (a, b) in a.indices.iter().zip(&b.indices) {
        visitor.require(visitor.span(&a.source_span, &b.source_span))?;
        let a = point(visitor, visitor.source, a.point)?;
        let b = point(visitor, visitor.destination, b.point)?;
        pair(visitor, a, b)?;
        match (&a.kind, &b.kind) {
            (CfgPointKind::Evaluate(a), CfgPointKind::Evaluate(b)) | (CfgPointKind::Call { expression: a, .. }, CfgPointKind::Call { expression: b, .. }) => visitor.expression(a, b)?,
            (CfgPointKind::Read { binding: a }, CfgPointKind::Read { binding: b }) => visitor.require(visitor.binding_reference(a)? == *b)?,
            _ => return Err(visitor.invalid()),
        }
        outputs::point_values(visitor, a.id, b.id)?;
    }
    Ok(())
}

fn indices(place: &ScalarPlace) -> Vec<(ByteSpan, &ScalarExpression)> {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => Vec::new(),
        ScalarPlace::Field { base, .. } => indices(base),
        ScalarPlace::Index { base, index, index_span, .. } => { let mut result = indices(base); result.push((*index_span, index)); result }
    }
}

fn operand(visitor: &InputVisitor<'_>, cfg: &ScalarReferenceCfg, expression: &ScalarExpression, id: CfgPointId) -> Result<(), ValueSchemaRemapError> {
    let original = point(visitor, cfg, id)?;
    visitor.require(original.source_span.range == expression_span(expression))?;
    match (&original.kind, expression) {
        (CfgPointKind::Evaluate(actual), _) | (CfgPointKind::Call { expression: actual, .. }, _) => visitor.require(actual == expression),
        (CfgPointKind::Read { binding }, ScalarExpression::Name { .. }) => visitor.require(binding.source == original.source_span.source && cfg.evaluated_values.values.iter().any(|fact| fact.id.point == id)),
        (CfgPointKind::ReadPlace { place, .. }, ScalarExpression::Dereference { place: actual, .. } | ScalarExpression::IndexedRead { place: actual, .. }) => visitor.require(place == actual),
        _ => Err(visitor.invalid()),
    }
}

pub(super) fn binding(visitor: &mut InputVisitor<'_>, source: &ScalarBindingCfgPoint, destination: &ScalarBindingCfgPoint) -> Result<(), ValueSchemaRemapError> {
    let a = point(visitor, visitor.source, source.point)?;
    let b = point(visitor, visitor.destination, destination.point)?;
    pair(visitor, a, b)?;
    match (&source.role, &a.kind, &b.kind) {
        (ScalarBindingPointRole::Bind, CfgPointKind::Bind { binding: a }, CfgPointKind::Bind { binding: b }) => visitor.require(visitor.binding_reference(a)? == *b),
        (ScalarBindingPointRole::CopyCallableFields, CfgPointKind::CopyCallableFields { source: a, target: x }, CfgPointKind::CopyCallableFields { source: b, target: y }) => { callable_source(visitor, a, b)?; visitor.require(visitor.binding_reference(x)? == *y) }
        (ScalarBindingPointRole::FieldAssign(field), CfgPointKind::Assign { target: a, rhs_point: x, .. }, CfgPointKind::Assign { target: b, rhs_point: y, .. }) => {
            visitor.require(a.projections.iter().any(|projection| *projection == ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field.clone()))) && visitor.point(*x)? == *y)?;
            place(visitor, a, b)
        }
        (ScalarBindingPointRole::ArrayElement(index), CfgPointKind::BindArrayElement { target: a, rhs_point: x }, CfgPointKind::BindArrayElement { target: b, rhs_point: y }) => {
            visitor.require(a.projections.last().is_some_and(|projection| matches!(projection, ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }) if *value == (*index).into())) && visitor.point(*x)? == *y)?;
            place(visitor, a, b)
        }
        _ => Err(visitor.invalid()),
    }
}

fn callable_source(visitor: &mut InputVisitor<'_>, source: &CallableFieldSource, destination: &CallableFieldSource) -> Result<(), ValueSchemaRemapError> {
    match (source, destination) {
        (CallableFieldSource::Evaluated(a), CallableFieldSource::Evaluated(b)) => outputs::value(visitor, a, b),
        (CallableFieldSource::Assignment(a), CallableFieldSource::Assignment(b)) => visitor.require(visitor.binding_reference(a)? == *b),
        _ => Err(visitor.invalid()),
    }
}

pub(super) fn assignment(visitor: &mut InputVisitor<'_>, source: &ScalarAssignmentCfgPoint, destination: &ScalarAssignmentCfgPoint) -> Result<(), ValueSchemaRemapError> {
    let a = point(visitor, visitor.source, source.point)?;
    let b = point(visitor, visitor.destination, destination.point)?;
    pair(visitor, a, b)?;
    match (source.role, &a.kind, &b.kind) {
        (ScalarAssignmentPointRole::Destination, CfgPointKind::AssignmentDestination { place: a, .. }, CfgPointKind::AssignmentDestination { place: b, .. }) => visitor.place(a, b),
        (ScalarAssignmentPointRole::PreviousSnapshot, CfgPointKind::AssignmentSnapshot { destination_point: a, holder: x, observed: m }, CfgPointKind::AssignmentSnapshot { destination_point: b, holder: y, observed: n }) => visitor.require(visitor.point(*a)? == *b && visitor.binding_reference(x)? == *y && m == n),
        (ScalarAssignmentPointRole::Result, CfgPointKind::AssignmentResult { snapshot_point: a, output: x }, CfgPointKind::AssignmentResult { snapshot_point: b, output: y }) => visitor.require(visitor.point(*a)? == *b && x == y),
        (ScalarAssignmentPointRole::Dispose, CfgPointKind::AssignmentDispose { snapshot_point: a }, CfgPointKind::AssignmentDispose { snapshot_point: b }) => visitor.require(visitor.point(*a)? == *b),
        (ScalarAssignmentPointRole::SelectElement, CfgPointKind::SelectArrayElement { target: a, index_point: x }, CfgPointKind::SelectArrayElement { target: b, index_point: y }) => { visitor.require(visitor.point(*x)? == *y)?; place(visitor, a, b) }
        (ScalarAssignmentPointRole::Assign, CfgPointKind::Assign { target: a, rhs_point: x, .. }, CfgPointKind::Assign { target: b, rhs_point: y, .. }) => { visitor.require(visitor.point(*x)? == *y)?; place(visitor, a, b) }
        (ScalarAssignmentPointRole::AssignThrough, CfgPointKind::AssignThrough { place: a, pointer_point: p, rhs_point: x, .. }, CfgPointKind::AssignThrough { place: b, pointer_point: q, rhs_point: y, .. }) => { visitor.require(visitor.point(*p)? == *q && visitor.point(*x)? == *y)?; visitor.place(a, b) }
        (ScalarAssignmentPointRole::CopyCallableFields, CfgPointKind::CopyCallableFields { source: a, target: x }, CfgPointKind::CopyCallableFields { source: b, target: y }) => { callable_source(visitor, a, b)?; visitor.require(visitor.binding_reference(x)? == *y) }
        (ScalarAssignmentPointRole::RhsOutput, CfgPointKind::Evaluate(a), CfgPointKind::Evaluate(b)) | (ScalarAssignmentPointRole::RhsOutput, CfgPointKind::Call { expression: a, .. }, CfgPointKind::Call { expression: b, .. }) => visitor.expression(a, b),
        (ScalarAssignmentPointRole::RhsOutput, CfgPointKind::Read { binding: a }, CfgPointKind::Read { binding: b }) => visitor.require(visitor.binding_reference(a)? == *b),
        _ => Err(visitor.invalid()),
    }
}

pub(super) fn while_point(visitor: &mut InputVisitor<'_>, source: &ScalarWhileCfgPoint, destination: &ScalarWhileCfgPoint) -> Result<(), ValueSchemaRemapError> {
    let a = point(visitor, visitor.source, source.point)?;
    let b = point(visitor, visitor.destination, destination.point)?;
    pair(visitor, a, b)?;
    match (source.role, &a.kind, &b.kind) {
        (ScalarWhilePointRole::LoopTest, CfgPointKind::LoopTest { condition: a }, CfgPointKind::LoopTest { condition: b }) | (ScalarWhilePointRole::Branch, CfgPointKind::Branch { condition: a }, CfgPointKind::Branch { condition: b }) => visitor.expression(a, b),
        (ScalarWhilePointRole::BodyScopeExit, CfgPointKind::ScopeExit, CfgPointKind::ScopeExit) => Ok(()),
        _ => Err(visitor.invalid()),
    }
}

pub(super) fn member(visitor: &mut InputVisitor<'_>, source: &ScalarExpression, destination: &ScalarExpression) -> Result<(), ValueSchemaRemapError> {
    let (ScalarExpression::Member { receiver: a, name: x, enum_tag: p, .. }, ScalarExpression::Member { receiver: b, name: y, enum_tag: q, .. }) = (source, destination) else { return Err(visitor.invalid()); };
    match (p, q) {
        (None, None) => Ok(()),
        (Some(p), Some(q)) => {
            let before = carrier(&visitor.checked.source_forest, &visitor.source.producer_owner)?;
            let after = carrier(&visitor.checked.destination_forest, &visitor.destination.producer_owner)?;
            let sources = before.policy_context.enums.iter().filter(|declaration| declaration.name == *a && declaration.variants.iter().any(|variant| variant.name == *x && variant.tag == *p)).collect::<Vec<_>>();
            let targets = after.policy_context.enums.iter().filter(|declaration| declaration.name == *b && declaration.variants.iter().any(|variant| variant.name == *y && variant.tag == *q)).collect::<Vec<_>>();
            let ([source], [target]) = (sources.as_slice(), targets.as_slice()) else { return Err(visitor.invalid()); };
            visitor.ty(&ScalarType::Enum(source.id.clone()), &ScalarType::Enum(target.id.clone()))
        }
        _ => Err(visitor.invalid()),
    }
}

pub(super) fn address(visitor: &mut InputVisitor<'_>, source: &ScalarExpression, destination: &ScalarExpression) -> Result<(), ValueSchemaRemapError> {
    let span = expression_span(source);
    let sources = visitor.source.initial_points.iter().filter(|point| point.source_span.range == span && matches!(point.kind, CfgPointKind::Borrow { .. } | CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. }))).collect::<Vec<_>>();
    let targets = visitor.destination.initial_points.iter().filter(|point| point.source_span.range == expression_span(destination) && matches!(point.kind, CfgPointKind::Borrow { .. } | CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. }))).collect::<Vec<_>>();
    visitor.require(sources.len() == targets.len())?;
    for a in sources {
        let id = visitor.point(a.id)?;
        let matches = targets.iter().filter(|point| point.id == id).collect::<Vec<_>>();
        let [b] = matches.as_slice() else { return Err(visitor.invalid()); };
        pair(visitor, a, b)?;
        match (&a.kind, &b.kind) {
            (CfgPointKind::Evaluate(x), CfgPointKind::Evaluate(y)) => {
                visitor.require(x == source && y == destination)?;
                raw_address(visitor, a.id, b.id)?;
            }
            (CfgPointKind::Borrow { request: x }, CfgPointKind::Borrow { request: y }) => {
                borrow(visitor, source, x, y)?;
                borrow_evaluation(visitor, a.id, b.id)?;
            }
            _ => return Err(visitor.invalid()),
        }
    }
    Ok(())
}

fn raw_address(visitor: &mut InputVisitor<'_>, source: CfgPointId, destination: CfgPointId) -> Result<(), ValueSchemaRemapError> {
    let a = visitor.source.evaluated_values.raw_address_productions.iter().filter(|row| row.value.point == source).collect::<Vec<_>>();
    let b = visitor.destination.evaluated_values.raw_address_productions.iter().filter(|row| row.value.point == destination).collect::<Vec<_>>();
    let ([a], [b]) = (a.as_slice(), b.as_slice()) else { return Err(visitor.invalid()); };
    outputs::value(visitor, &a.value, &b.value)?;
    match (&a.source, &b.source) {
        (RawAddressInputSource::CompilerStorage, RawAddressInputSource::CompilerStorage) => {
            let sources = visitor.source.evaluated_values.raw_address_inputs.iter().filter(|row| row.value == a.value).collect::<Vec<_>>();
            let targets = visitor.destination.evaluated_values.raw_address_inputs.iter().filter(|row| row.value == b.value).collect::<Vec<_>>();
            let ([source], [target]) = (sources.as_slice(), targets.as_slice()) else { return Err(visitor.invalid()); };
            visitor.require(visitor.point(source.read)? == target.read && visitor.checked.qualification(visitor.remap, &visitor.source.producer_owner)?.shape(visitor.remap, &source.pointee)? == target.pointee)?;
            place(visitor, &source.place, &target.place)?;
            read_place(visitor, point(visitor, visitor.source, source.read)?, point(visitor, visitor.destination, target.read)?)
        }
        (RawAddressInputSource::ThroughPointer { pointer: a, place: x }, RawAddressInputSource::ThroughPointer { pointer: b, place: y }) => { outputs::value(visitor, a, b)?; visitor.place(x, y) }
        _ => Err(visitor.invalid()),
    }
}

fn borrow_evaluation(visitor: &mut InputVisitor<'_>, source: CfgPointId, destination: CfgPointId) -> Result<(), ValueSchemaRemapError> {
    let a = visitor.source.borrow_evaluations.iter().filter(|row| row.point == source && row.owner == visitor.source.producer_owner).collect::<Vec<_>>();
    let b = visitor.destination.borrow_evaluations.iter().filter(|row| row.point == destination && row.owner == visitor.destination.producer_owner).collect::<Vec<_>>();
    let ([a], [b]) = (a.as_slice(), b.as_slice()) else { return Err(visitor.invalid()); };
    match (&a.input, &b.input) {
        (ReferenceBorrowInput::DirectStorage, ReferenceBorrowInput::DirectStorage) => {},
        (ReferenceBorrowInput::Through { pointer_point: a, origins: x }, ReferenceBorrowInput::Through { pointer_point: b, origins: y }) => { visitor.require(visitor.point(*a)? == *b && x.len() == y.len())?; for (a, b) in x.iter().zip(y) { visitor.require(visitor.remap.origin(&visitor.source.producer_owner, *a)? == *b)?; } }
        (ReferenceBorrowInput::MissingProducer { pointer_point: a }, ReferenceBorrowInput::MissingProducer { pointer_point: b }) => visitor.require(visitor.point(*a)? == *b)?,
        _ => return Err(visitor.invalid()),
    }
    visitor.require(a.cases.len() == b.cases.len() && a.output.len() == b.output.len())?;
    for (a, b) in a.output.iter().zip(&b.output) { visitor.require(visitor.remap.origin(&visitor.source.producer_owner, *a)? == *b)?; }
    for (a, b) in a.cases.iter().zip(&b.cases) {
        match (a, b) {
            (ReferenceBorrowCase::Accepted { place: a, loan: x, origin: p, access: m }, ReferenceBorrowCase::Accepted { place: b, loan: y, origin: q, access: n }) => {
                place(visitor, a, b)?;
                visitor.require(visitor.remap.checked_loan(&visitor.source.producer_owner, x)? == *y && visitor.remap.origin(&visitor.source.producer_owner, *p)? == *q)?;
                match (m, n) {
                    (ReferenceBorrowAccess::DirectStorage, ReferenceBorrowAccess::DirectStorage) => {},
                    (ReferenceBorrowAccess::Through { pointer_point: a, parent_origin: x }, ReferenceBorrowAccess::Through { pointer_point: b, parent_origin: y }) => visitor.require(visitor.point(*a)? == *b && visitor.remap.origin(&visitor.source.producer_owner, *x)? == *y)?,
                    _ => return Err(visitor.invalid()),
                }
            }
            (ReferenceBorrowCase::Invalid { parent_origin: a, origin: x }, ReferenceBorrowCase::Invalid { parent_origin: b, origin: y }) => visitor.require(visitor.remap.origin(&visitor.source.producer_owner, *a)? == *b && visitor.remap.origin(&visitor.source.producer_owner, *x)? == *y)?,
            (ReferenceBorrowCase::Rejected { input: a, diagnostic: x }, ReferenceBorrowCase::Rejected { input: b, diagnostic: y }) => {
                match (a, b) { (ReferenceBorrowRejectedInput::DirectStorage, ReferenceBorrowRejectedInput::DirectStorage) => {}, (ReferenceBorrowRejectedInput::Through { parent_origin: a }, ReferenceBorrowRejectedInput::Through { parent_origin: b }) => visitor.require(visitor.remap.origin(&visitor.source.producer_owner, *a)? == *b)?, _ => return Err(visitor.invalid()) }
                match (x, y) {
                    (ReferenceBorrowDiagnostic::Missing { source_span: a, reason: x }, ReferenceBorrowDiagnostic::Missing { source_span: b, reason: y }) => visitor.require(visitor.span(a, b) && x == y)?,
                    (ReferenceBorrowDiagnostic::Conflict { source_span: a, origins: x }, ReferenceBorrowDiagnostic::Conflict { source_span: b, origins: y }) => visitor.require(visitor.span(a, b) && x.len() == y.len() && x.iter().zip(y).all(|(a, b)| visitor.span(a, b)))?,
                    _ => return Err(visitor.invalid()),
                }
            }
            _ => return Err(visitor.invalid()),
        }
    }
    Ok(())
}

fn borrow(visitor: &mut InputVisitor<'_>, expression: &ScalarExpression, source: &ReferenceBorrowRequest, destination: &ReferenceBorrowRequest) -> Result<(), ValueSchemaRemapError> {
    let ScalarExpression::CheckedAddress { mutability, .. } = expression else { return Err(visitor.invalid()); };
    let (indices, actual_indices) = match (source, destination) {
        (ReferenceBorrowRequest::DirectStorage { place: a, loan: x, origin: p, indices: s }, ReferenceBorrowRequest::DirectStorage { place: b, loan: y, origin: q, indices: t }) => {
            visitor.require(x.mode == *mutability && y.mode == *mutability && visitor.remap.checked_loan(&visitor.source.producer_owner, x)? == *y && visitor.remap.origin(&visitor.source.producer_owner, *p)? == *q)?;
            place(visitor, a, b)?;
            (s, t)
        }
        (ReferenceBorrowRequest::Through { pointer_point: a, place: x, mode: p, indices: s }, ReferenceBorrowRequest::Through { pointer_point: b, place: y, mode: q, indices: t }) => {
            visitor.require(p == mutability && q == mutability && visitor.point(*a)? == *b)?;
            visitor.place(x, y)?;
            (s, t)
        }
        _ => return Err(visitor.invalid()),
    };
    visitor.require(indices.len() == actual_indices.len())?;
    for (a, b) in indices.iter().zip(actual_indices) { visitor.require(a.projection == b.projection && visitor.point(a.value_point)? == b.value_point)?; outputs::point_values(visitor, a.value_point, b.value_point)?; }
    Ok(())
}
