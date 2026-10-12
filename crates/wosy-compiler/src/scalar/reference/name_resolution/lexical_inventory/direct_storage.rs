use super::*;
use source_boundary::direct_storage_constructor::SourceDirectStorageConstructorRef;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExpectedAddress {
    expression: ScalarExpression,
    occurrence: SourceSpan,
    block: ByteSpan,
    declaration: Declaration,
    function_span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExpectedRebind {
    call: ScalarExpression,
    address: ExpectedAddress,
    association: ReferenceRebindAssociationId,
}

impl Walker<'_> {
    fn address(&self, expression: &ScalarExpression, block: ByteSpan) -> Result<ExpectedAddress, ReferenceAnalysisError> {
        let ScalarExpression::CheckedAddress { place, .. } = expression else { return Err(invalid(&SourceSpan::new(self.identity.clone(), expression_span(expression)))); };
        let ScalarPlace::Name { name, .. } = super::super::super::reference_root(place) else { return Err(invalid(&SourceSpan::new(self.identity.clone(), expression_span(expression)))); };
        let declaration = match self.locals.get(name) {
            Some(declaration) => declaration,
            None => self.globals.get(name).ok_or_else(|| invalid(&SourceSpan::new(self.identity.clone(), expression_span(expression))))?,
        };
        Ok(ExpectedAddress { expression: expression.clone(), occurrence: SourceSpan::new(self.identity.clone(), expression_span(expression)), block,
            declaration: declaration.clone(), function_span: self.function_span })
    }

    pub(super) fn direct_address(&mut self, expression: &ScalarExpression, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        let ScalarExpression::CheckedAddress { place, .. } = expression else { return Err(invalid(&SourceSpan::new(self.identity.clone(), expression_span(expression)))); };
        self.place(place, block)?;
        if matches!(super::super::super::reference_root(place), ScalarPlace::Name { .. }) {
            self.inventory.direct_addresses.push(self.address(expression, block)?);
        }
        Ok(())
    }

    pub(super) fn rebind_address(&mut self, call: &ScalarExpression, expression: &ScalarExpression, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        let ScalarExpression::CheckedAddress { place, .. } = expression else { return Err(invalid(&SourceSpan::new(self.identity.clone(), expression_span(expression)))); };
        let association = ReferenceRebindAssociationId(self.next_rebind_association);
        self.next_rebind_association += 1;
        self.place(place, block)?;
        self.inventory.rebind_addresses.push(ExpectedRebind { call: call.clone(), address: self.address(expression, block)?, association });
        Ok(())
    }
}

fn indices<'a>(place: &'a ScalarPlace, projection: &mut usize, result: &mut Vec<(usize, &'a ScalarExpression)>) {
    match place {
        ScalarPlace::Name { .. } => {},
        ScalarPlace::Field { base, .. } => { indices(base, projection, result); *projection += 1; },
        ScalarPlace::Index { base, index, .. } => { indices(base, projection, result); result.push((*projection, index)); *projection += 1; },
        ScalarPlace::Dereference { .. } => {},
    }
}

fn validate_indices(cfg: &ScalarReferenceCfg, inventory: &Inventory, expected: &ExpectedAddress, point: &ReferenceFlowPoint, actual: &[ReferenceBorrowIndexInput]) -> Result<(), ReferenceAnalysisError> {
    let ScalarExpression::CheckedAddress { place, .. } = &expected.expression else { return Err(invalid(&expected.occurrence)); };
    let mut required = Vec::new();
    indices(place, &mut 0, &mut required);
    if required.len() != actual.len() { return Err(invalid(&expected.occurrence)); }
    for ((projection, expression), actual) in required.iter().zip(actual) {
        if *projection != actual.projection || actual.value_point.0 >= point.id.0 || !connected(cfg, actual.value_point, point.id)
            { return Err(invalid(&expected.occurrence)); }
        operand(cfg, inventory, &expected.occurrence, expected.block, expression, actual.value_point)?;
    }
    for pair in actual.windows(2) {
        if pair[0].value_point.0 >= pair[1].value_point.0 || !connected(cfg, pair[0].value_point, pair[1].value_point) { return Err(invalid(&expected.occurrence)); }
    }
    Ok(())
}

fn definition(expected: &ExpectedAddress) -> Result<ReferenceOrigin, ReferenceAnalysisError> {
    let ScalarExpression::CheckedAddress { mutability, place, .. } = &expected.expression else { return Err(invalid(&expected.occurrence)); };
    let binding = &expected.declaration.binding;
    let mut projections = Vec::new();
    super::super::super::collect_reference_projections(place, None, &mut projections);
    Ok(ReferenceOrigin::Fresh {
        allocation: ReturnedAllocationId { source: expected.occurrence.source.clone(), function_span: expected.function_span,
            declaration_span: binding.declaration_span, creation_span: expected.occurrence.range },
        loan: ReferenceLoanId { mode: *mutability, origin_place: ReferencePlaceId { source: expected.occurrence.source.clone(),
            function_span: expected.function_span, declaration_span: binding.declaration_span, block_span: binding.block_span,
            binding: binding.clone(), projections, place: place.clone() }, creation_source: expected.occurrence.source.clone(), creation_span: expected.occurrence.range, parent: None },
    })
}

pub(in crate::scalar) fn direct_constructors(cfg: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Result<Vec<SourceDirectStorageConstructorRef>, ReferenceAnalysisError> {
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(invalid(&cfg.initial_points[0].source_span)); };
    if context.owner != cfg.producer_owner { return Err(invalid(&cfg.initial_points[0].source_span)); }
    let inventory = expected(context, structs)?;
    let mut required = Vec::new();
    let mut visited = Vec::new();
    let mut events = BTreeSet::new();
    for expected in &inventory.direct_addresses {
        if visited.contains(expected) { continue; }
        visited.push(expected.clone());
        let ScalarExpression::CheckedAddress { place: original, mutability, .. } = &expected.expression else { return Err(invalid(&expected.occurrence)); };
        validate_fields(original, structs, &expected.occurrence)?;
        let points = cfg.initial_points.iter().filter(|point| point.source_span == expected.occurrence && scope_matches(&cfg.initial_points, point, expected.block)
            && matches!(&point.kind, CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { place, loan, .. } } if place.place == *original && loan.mode == *mutability)).collect::<Vec<_>>();
        if points.len() != inventory.direct_addresses.iter().filter(|row| *row == expected).count() { return Err(invalid(&expected.occurrence)); }
        let definition = definition(expected)?;
        let ReferenceOrigin::Fresh { loan: expected_loan, .. } = &definition else { return Err(invalid(&expected.occurrence)); };
        for point in points {
            let CfgPointKind::Borrow { request: request @ ReferenceBorrowRequest::DirectStorage { place, loan, origin, indices } } = &point.kind else { return Err(invalid(&expected.occurrence)); };
            if cfg.initial_points.get(point.id.0) != Some(point) || !events.insert(point.id) || loan != expected_loan || place != &expected_loan.origin_place
                || cfg.facts.get(origin.0) != Some(&definition) || cfg.facts.iter().position(|fact| fact == &definition) != Some(origin.0)
                || point.possible_origins != Some(BTreeSet::from([*origin])) { return Err(invalid(&expected.occurrence)); }
            validate_indices(cfg, &inventory, expected, point, indices)?;
            if !evaluated_borrow::validates(&cfg.facts, point, expected.function_span, request, &ReferenceBorrowInput::DirectStorage,
                &ReferenceBorrowCase::Accepted { place: place.clone(), loan: loan.clone(), origin: *origin, access: ReferenceBorrowAccess::DirectStorage }) { return Err(invalid(&expected.occurrence)); }
            required.push(SourceDirectStorageConstructorRef { origin: source_boundary::SourceConstructorOriginRef { owner: cfg.producer_owner.clone(), origin: *origin }, event: SourceEventId { owner: cfg.producer_owner.clone(), point: point.id } });
        }
    }
    allocation_constructors(cfg, structs)?;
    for point in &cfg.initial_points {
        if matches!(point.kind, CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { .. } }) && !events.contains(&point.id) { return Err(invalid(&point.source_span)); }
    }
    required.sort_by_key(|row| row.event.point);
    Ok(required)
}

pub(super) fn validate_fields(place: &ScalarPlace, structs: &[ScalarStruct], occurrence: &SourceSpan) -> Result<(), ReferenceAnalysisError> {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => Ok(()),
        ScalarPlace::Index { base, .. } => validate_fields(base, structs, occurrence),
        ScalarPlace::Field { base, field, .. } => {
            let ScalarFieldReference::Resolved(field) = field else { return Err(invalid(occurrence)); };
            let declarations = structs.iter().filter(|declaration| declaration.id == field.structure).collect::<Vec<_>>();
            let [declaration] = declarations.as_slice() else { return Err(invalid(occurrence)); };
            if declaration.fields.iter().filter(|actual| actual.id == *field).count() != 1 { return Err(invalid(occurrence)); }
            validate_fields(base, structs, occurrence)
        },
    }
}

pub(in crate::scalar) fn allocation_constructors(cfg: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Result<Vec<source_boundary::allocation_identity_constructor::SourceAllocationIdentityConstructorRef>, ReferenceAnalysisError> {
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(invalid(&cfg.initial_points[0].source_span)); };
    if context.owner != cfg.producer_owner { return Err(invalid(&cfg.initial_points[0].source_span)); }
    let inventory = expected(context, structs)?;
    let mut required = Vec::new();
    let mut events = BTreeSet::new();
    let mut reservations = BTreeSet::new();
    for excluded in &inventory.rebind_addresses {
        let fail = || invalid(&excluded.address.occurrence);
        let ScalarExpression::CheckedAddress { place: original, mutability, .. } = &excluded.address.expression else { return Err(fail()); };
        validate_fields(original, structs, &excluded.address.occurrence)?;
        let points = cfg.initial_points.iter().filter(|point| point.source_span == excluded.address.occurrence
            && scope_matches(&cfg.initial_points, point, excluded.address.block)
            && matches!(&point.kind, CfgPointKind::AllocationIdentityAddress { request } if request.association == excluded.association
                && request.candidate.place.place == *original && request.candidate.loan.mode == *mutability)).collect::<Vec<_>>();
        let [point] = points.as_slice() else { return Err(fail()); };
        let CfgPointKind::AllocationIdentityAddress { request } = &point.kind else { return Err(fail()); };
        let definition = definition(&excluded.address)?;
        let ReferenceOrigin::Fresh { loan, .. } = &definition else { return Err(fail()); };
        if cfg.initial_points.get(point.id.0) != Some(*point) || !events.insert(point.id) || !reservations.insert(request.association)
            || point.possible_origins != Some(BTreeSet::new()) || request.candidate.place != loan.origin_place || request.candidate.loan != *loan
            || cfg.facts.get(request.candidate.origin.0) != Some(&definition)
            || cfg.facts.iter().position(|row| row == &definition) != Some(request.candidate.origin.0) { return Err(fail()); }
        validate_indices(cfg, &inventory, &excluded.address, point, &request.indices)?;
        let association = allocation_address::validate_association(cfg, point, request)?;
        let call = cfg.initial_points.get(association.call_point.0).ok_or_else(fail)?;
        let ScalarExpression::Call { arguments, occurrence, .. } = &excluded.call else { return Err(fail()); };
        let [address, raw, length] = arguments.as_slice() else { return Err(fail()); };
        if address != &excluded.address.expression || association.owner != cfg.producer_owner || association.occurrence != **occurrence
            || association.address_point != point.id || !matches!(&call.kind, CfgPointKind::Call { expression, target: ResolvedCallTarget::Core(CoreOperationId::Rebind), output: 0, argument_points }
                if expression == &excluded.call && argument_points == &[point.id, association.raw_point, association.length_point])
            || call.id != association.call_point || call.source_span != occurrence.source_span
            || !scope_matches(&cfg.initial_points, call, excluded.address.block)
            || !(point.id < association.raw_point && association.raw_point < association.length_point
                && association.length_point < association.effect_point && association.effect_point < association.call_point) { return Err(fail()); }
        for id in [association.raw_point, association.length_point, association.effect_point, association.call_point] {
            let actual = cfg.initial_points.get(id.0).ok_or_else(fail)?;
            if actual.id != id || actual.source_span.source != excluded.address.occurrence.source { return Err(fail()); }
        }
        operand(cfg, &inventory, &excluded.address.occurrence, excluded.address.block, raw, association.raw_point)?;
        operand(cfg, &inventory, &excluded.address.occurrence, excluded.address.block, length, association.length_point)?;
        required.push(source_boundary::allocation_identity_constructor::SourceAllocationIdentityConstructorRef {
            origin: source_boundary::SourceConstructorOriginRef { owner: cfg.producer_owner.clone(), origin: request.candidate.origin },
            event: SourceEventId { owner: cfg.producer_owner.clone(), point: point.id },
            association: source_boundary::allocation_identity_constructor::SourceAllocationIdentityAssociationRef { owner: cfg.producer_owner.clone(), association: request.association },
        });
    }
    if reservations.iter().copied().collect::<Vec<_>>() != (0..cfg.rebind_associations.len()).map(ReferenceRebindAssociationId).collect::<Vec<_>>() { return Err(invalid(&cfg.initial_points[0].source_span)); }
    for point in &cfg.initial_points {
        if matches!(point.kind, CfgPointKind::AllocationIdentityAddress { .. }) && !events.contains(&point.id) { return Err(invalid(&point.source_span)); }
    }
    required.sort_by_key(|row| row.event.point);
    Ok(required)
}
