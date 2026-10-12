use super::*;

use source_boundary::SourceConstructorOriginRef;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::scalar::reference) struct SourceConstructorOriginDefinition {
    pub id: SourceConstructorOriginRef,
    pub definition: ReferenceOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::scalar::reference) struct InitialBorrowOriginReceipt {
    pub event: SourceEventId,
    pub source_span: SourceSpan,
    pub scope: ReferenceScopeId,
    pub request: ReferenceBorrowRequest,
    pub input: ReferenceBorrowInput,
    pub definitions: Vec<SourceConstructorOriginDefinition>,
    pub realized: Vec<ReferenceBorrowCase>,
    pub applied: ReferenceBorrowEvaluation,
}

pub(in crate::scalar::reference) struct InitialTransferReceipt {
    pub errors: Vec<ReferenceAnalysisError>,
    pub borrows: Vec<InitialBorrowOriginReceipt>,
}

fn invalid(point: &ReferenceFlowPoint) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: point.source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

fn origins(input: &ReferenceBorrowInput, cases: &[ReferenceBorrowCase]) -> BTreeSet<ReferenceOriginId> {
    let mut ids = match input {
        ReferenceBorrowInput::Through { origins, .. } => origins.clone(),
        ReferenceBorrowInput::DirectStorage | ReferenceBorrowInput::MissingProducer { .. } => BTreeSet::new(),
    };
    for case in cases {
        match case {
            ReferenceBorrowCase::Accepted { origin, .. } | ReferenceBorrowCase::Invalid { origin, .. } => { ids.insert(*origin); },
            ReferenceBorrowCase::Rejected { .. } => {},
        }
    }
    ids
}

fn definition<'a>(receipt: &'a InitialBorrowOriginReceipt, origin: ReferenceOriginId, point: &ReferenceFlowPoint) -> Result<&'a ReferenceOrigin, ReferenceAnalysisError> {
    let rows = receipt.definitions.iter().filter(|row| row.id.owner == receipt.event.owner && row.id.origin == origin).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(invalid(point)); };
    Ok(&row.definition)
}

fn validate(receipt: &InitialBorrowOriginReceipt, point: &ReferenceFlowPoint, function_span: ByteSpan) -> Result<(), ReferenceAnalysisError> {
    let fail = || invalid(point);
    if receipt.event.point != point.id || receipt.source_span != point.source_span || receipt.scope != point.scope
        || !matches!(&point.kind, CfgPointKind::Borrow { request } if request == &receipt.request)
        || receipt.applied.owner != receipt.event.owner || receipt.applied.point != point.id || receipt.applied.input != receipt.input
        || receipt.applied.cases.len() != receipt.realized.len() {
        return Err(fail());
    }
    let required = origins(&receipt.input, &receipt.realized);
    if required.len() != receipt.definitions.len() { return Err(fail()); }
    for id in &required { definition(receipt, *id, point)?; }
    match (&receipt.request, &receipt.input) {
        (ReferenceBorrowRequest::DirectStorage { .. }, ReferenceBorrowInput::DirectStorage) if receipt.realized.len() == 1 => {},
        (ReferenceBorrowRequest::Through { pointer_point, .. }, ReferenceBorrowInput::Through { pointer_point: actual, origins })
            if pointer_point == actual && !origins.is_empty() && origins.len() == receipt.realized.len() => {},
        (ReferenceBorrowRequest::Through { pointer_point, .. }, ReferenceBorrowInput::MissingProducer { pointer_point: actual })
            if pointer_point == actual && receipt.realized.is_empty() && receipt.applied.output.is_empty() => {},
        _ => return Err(fail()),
    }
    let mut parents = BTreeSet::new();
    let mut output = BTreeSet::new();
    for (case, applied) in receipt.realized.iter().zip(&receipt.applied.cases) {
        let rejected_input = match (&receipt.request, case) {
            (ReferenceBorrowRequest::DirectStorage { place, loan, origin, .. }, ReferenceBorrowCase::Accepted { place: actual_place, loan: actual_loan, origin: actual_origin, access: ReferenceBorrowAccess::DirectStorage }) => {
                let expected = ReferenceOrigin::Fresh {
                    allocation: ReturnedAllocationId {
                        source: point.source_span.source.clone(), function_span: place.function_span,
                        declaration_span: place.declaration_span, creation_span: point.source_span.range,
                    },
                    loan: loan.clone(),
                };
                if place != actual_place || loan != actual_loan || origin != actual_origin || loan.parent.is_some()
                    || loan.origin_place != *place || loan.creation_source != point.source_span.source
                    || loan.creation_span != point.source_span.range || definition(receipt, *origin, point)? != &expected { return Err(fail()); }
                ReferenceBorrowRejectedInput::DirectStorage
            },
            (ReferenceBorrowRequest::Through { pointer_point, place, mode, .. }, ReferenceBorrowCase::Accepted { place: actual_place, loan, origin, access: ReferenceBorrowAccess::Through { pointer_point: actual_pointer, parent_origin } }) => {
                let ReferenceBorrowInput::Through { origins, .. } = &receipt.input else { return Err(fail()); };
                if pointer_point != actual_pointer || !origins.contains(parent_origin) || !parents.insert(*parent_origin) { return Err(fail()); }
                let parent = definition(receipt, *parent_origin, point)?;
                if !matches!(parent, ReferenceOrigin::Fresh { .. } | ReferenceOrigin::BorrowedFrom { .. }) { return Err(fail()); }
                let expected = evaluated_borrow::child_fact(point, function_span, place, *mode, parent);
                let expected_loan = match &expected { ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => loan, _ => return Err(fail()) };
                if definition(receipt, *origin, point)? != &expected || actual_place != &expected_loan.origin_place || loan != expected_loan { return Err(fail()); }
                ReferenceBorrowRejectedInput::Through { parent_origin: *parent_origin }
            },
            (ReferenceBorrowRequest::Through { place, mode, .. }, ReferenceBorrowCase::Invalid { parent_origin, origin }) => {
                let ReferenceBorrowInput::Through { origins, .. } = &receipt.input else { return Err(fail()); };
                if !origins.contains(parent_origin) || !parents.insert(*parent_origin) { return Err(fail()); }
                let parent = definition(receipt, *parent_origin, point)?;
                if !matches!(parent, ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. })
                    || definition(receipt, *origin, point)? != &evaluated_borrow::child_fact(point, function_span, place, *mode, parent) { return Err(fail()); }
                ReferenceBorrowRejectedInput::Through { parent_origin: *parent_origin }
            },
            _ => return Err(fail()),
        };
        match applied {
            ReferenceBorrowCase::Accepted { origin, .. } | ReferenceBorrowCase::Invalid { origin, .. } => {
                if case != applied { return Err(fail()); }
                output.insert(*origin);
            },
            ReferenceBorrowCase::Rejected { input, diagnostic } => {
                if *input != rejected_input { return Err(fail()); }
                match diagnostic {
                    ReferenceBorrowDiagnostic::Missing { source_span, reason } => {
                        if *source_span != point.source_span || !matches!(reason,
                            ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin
                                | ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin) { return Err(fail()); }
                    },
                    ReferenceBorrowDiagnostic::Conflict { source_span, origins } => {
                        if *source_span != point.source_span || origins.is_empty() { return Err(fail()); }
                    },
                }
            },
        }
    }
    if let ReferenceBorrowInput::Through { origins, .. } = &receipt.input {
        if parents != *origins { return Err(fail()); }
    }
    if output != receipt.applied.output { return Err(fail()); }
    Ok(())
}

pub(in crate::scalar::reference) fn record(
    owner: &ProducerCfgOwnerId,
    original: &ReferenceFlowPoint,
    function_span: ByteSpan,
    facts: &[ReferenceOrigin],
    realized: Vec<ReferenceBorrowCase>,
    applied: &ReferenceBorrowEvaluation,
    receipts: &mut Vec<InitialBorrowOriginReceipt>,
) -> Result<(), ReferenceAnalysisError> {
    let CfgPointKind::Borrow { request } = &original.kind else { return Err(invalid(original)); };
    let mut definitions = Vec::new();
    for origin in origins(&applied.input, &realized) {
        let definition = facts.get(origin.0).ok_or_else(|| invalid(original))?;
        definitions.push(SourceConstructorOriginDefinition { id: SourceConstructorOriginRef { owner: owner.clone(), origin }, definition: definition.clone() });
    }
    let receipt = InitialBorrowOriginReceipt {
        event: SourceEventId { owner: owner.clone(), point: original.id }, source_span: original.source_span.clone(),
        scope: original.scope, request: request.clone(), input: applied.input.clone(), definitions, realized, applied: applied.clone(),
    };
    validate(&receipt, original, function_span)?;
    if !receipts.contains(&receipt) { receipts.push(receipt); }
    Ok(())
}

pub(in crate::scalar::reference) fn validate_handoff(
    parent: &ScalarReferenceCfg,
    source: &source_boundary::SourceCapturePreflightInput,
    transfer: &InitialTransferReceipt,
) -> Result<(), ReferenceAnalysisError> {
    let endpoints = source.construction_forest.sources.iter().filter(|row| row.producer == parent.producer_owner).collect::<Vec<_>>();
    let [endpoint] = endpoints.as_slice() else { return Err(invalid(&parent.initial_points[0])); };
    let original = endpoint.resource_construction.original();
    super::validate_null_constructors(original, &source.policy_context.structs)?;
    if parent.source_null_constructors != original.source_null_constructors { return Err(invalid(&parent.initial_points[0])); }
    let original_points = if matches!(parent.producer_owner, ProducerCfgOwnerId::Module(_)) {
        let (_, points) = source_boundary::module_source_topology::constructor_handoff(original, parent)?;
        for point in points.iter().filter(|point| matches!(point.kind, CfgPointKind::Capture { .. })) {
            let event = SourceEventId { owner: parent.producer_owner.clone(), point: point.id };
            source_boundary::module_source_topology::constructor_capture(original, &event)?;
        }
        points
    } else {
        if original.initial_points != parent.initial_points { return Err(invalid(&parent.initial_points[0])); }
        original.initial_points.as_slice()
    };
    for receipt in &transfer.borrows {
        let point = original_points.get(receipt.event.point.0).ok_or_else(|| invalid(&original_points[0]))?;
        if receipt.event.owner != parent.producer_owner { return Err(invalid(point)); }
        validate(receipt, point, original_points[0].source_span.range)?;
        if parent.borrow_evaluations.iter().filter(|evaluation| **evaluation == receipt.applied).count() != 1 { return Err(invalid(point)); }
        for row in &receipt.definitions {
            if parent.facts.get(row.id.origin.0) != Some(&row.definition) { return Err(invalid(point)); }
        }
    }
    for evaluation in &parent.borrow_evaluations {
        if !transfer.borrows.iter().any(|receipt| receipt.applied == *evaluation) {
            let point = original_points.get(evaluation.point.0).ok_or_else(|| invalid(&original_points[0]))?;
            return Err(invalid(point));
        }
    }
    Ok(())
}

pub(in crate::scalar::reference) fn original_arena(
    parent: &ScalarReferenceCfg,
    source: &source_boundary::SourceCapturePreflightInput,
    transfer: &InitialTransferReceipt,
) -> Result<Vec<ReferenceOrigin>, ReferenceAnalysisError> {
    validate_handoff(parent, source, transfer)?;
    let endpoints = source.construction_forest.sources.iter().filter(|row| row.producer == parent.producer_owner).collect::<Vec<_>>();
    let [endpoint] = endpoints.as_slice() else { return Err(invalid(&parent.initial_points[0])); };
    let original = endpoint.resource_construction.original();
    let mut arena = original.facts.clone();
    for receipt in &transfer.borrows {
        let point = &parent.initial_points[receipt.event.point.0];
        for case in &receipt.realized {
            let (origin, expected) = match (&receipt.request, case) {
                (ReferenceBorrowRequest::DirectStorage { origin, .. }, ReferenceBorrowCase::Accepted { .. }) => {
                    let expected = arena.get(origin.0).ok_or_else(|| invalid(point))?.clone();
                    (*origin, expected)
                },
                (ReferenceBorrowRequest::Through { place, mode, .. }, ReferenceBorrowCase::Accepted { origin, access: ReferenceBorrowAccess::Through { parent_origin, .. }, .. }
                    | ReferenceBorrowCase::Invalid { parent_origin, origin }) => {
                    let parent_definition = arena.get(parent_origin.0).ok_or_else(|| invalid(point))?;
                    (*origin, evaluated_borrow::child_fact(point, original.initial_points[0].source_span.range, place, *mode, parent_definition))
                },
                _ => return Err(invalid(point)),
            };
            if evaluated_borrow::intern(&mut arena, expected.clone()) != origin
                || definition(receipt, origin, point)? != &expected { return Err(invalid(point)); }
        }
        for row in &receipt.definitions {
            if arena.get(row.id.origin.0) != Some(&row.definition) { return Err(invalid(point)); }
        }
    }
    if arena != parent.facts { return Err(invalid(&parent.initial_points[0])); }
    Ok(original.facts.clone())
}
