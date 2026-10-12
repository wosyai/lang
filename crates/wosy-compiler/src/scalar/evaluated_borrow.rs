use super::*;

pub(super) mod checked_storage_serde {
    use super::*;

    pub fn serialize<S: serde::Serializer>(
        bindings: &BTreeMap<ReferenceBindingId, ReferencePlaceId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if bindings
            .iter()
            .any(|(binding, place)| binding != &place.binding)
        {
            return Err(serde::ser::Error::custom(
                "checked storage key differs from its binding",
            ));
        }
        bindings.values().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<ReferenceBindingId, ReferencePlaceId>, D::Error> {
        let places = Vec::<ReferencePlaceId>::deserialize(deserializer)?;
        let mut bindings = BTreeMap::new();
        for place in places {
            if bindings.insert(place.binding.clone(), place).is_some() {
                return Err(serde::de::Error::custom(
                    "duplicate checked storage binding",
                ));
            }
        }
        Ok(bindings)
    }
}

pub(super) fn intern(
    facts: &mut Vec<ReferenceOrigin>,
    origin: ReferenceOrigin,
) -> ReferenceOriginId {
    if let Some(index) = facts.iter().position(|fact| fact == &origin) {
        ReferenceOriginId(index)
    } else {
        let id = ReferenceOriginId(facts.len());
        facts.push(origin);
        id
    }
}

pub(super) fn child_fact(
    point: &ReferenceFlowPoint,
    function_span: ByteSpan,
    place: &ScalarPlace,
    mode: ScalarReferenceMutability,
    parent: &ReferenceOrigin,
) -> ReferenceOrigin {
    let parent_loan = match parent {
        ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
        ReferenceOrigin::Null { span }
        | ReferenceOrigin::Invalid {
            origin_span: span, ..
        } => {
            return ReferenceOrigin::Invalid {
                origin_span: *span,
                conflict_span: point.source_span.range,
            };
        }
    };
    let mut projections = Vec::new();
    reference::collect_reference_projections(place, Some(parent_loan), &mut projections);
    let loan = ReferenceLoanId {
        mode,
        origin_place: ReferencePlaceId {
            source: point.source_span.source.clone(),
            function_span,
            declaration_span: parent_loan.origin_place.declaration_span,
            block_span: parent_loan.origin_place.block_span,
            binding: parent_loan.origin_place.binding.clone(),
            projections,
            place: place.clone(),
        },
        creation_source: point.source_span.source.clone(),
        creation_span: point.source_span.range,
        parent: Some(Box::new(parent_loan.clone())),
    };
    match parent {
        ReferenceOrigin::Fresh { allocation, .. } => ReferenceOrigin::Fresh {
            allocation: allocation.clone(),
            loan,
        },
        ReferenceOrigin::BorrowedFrom { parameter, .. } => ReferenceOrigin::BorrowedFrom {
            parameter: *parameter,
            loan,
        },
        _ => unreachable!("valid parent loan"),
    }
}

pub(super) fn realize(
    facts: &mut Vec<ReferenceOrigin>,
    point: &ReferenceFlowPoint,
    function_span: ByteSpan,
    request: &ReferenceBorrowRequest,
    input: &ReferenceBorrowInput,
) -> Vec<ReferenceBorrowCase> {
    match (request, input) {
        (
            ReferenceBorrowRequest::DirectStorage {
                place,
                loan,
                origin,
                ..
            },
            ReferenceBorrowInput::DirectStorage,
        ) => {
            vec![ReferenceBorrowCase::Accepted {
                place: place.clone(),
                loan: loan.clone(),
                origin: *origin,
                access: ReferenceBorrowAccess::DirectStorage,
            }]
        }
        (
            ReferenceBorrowRequest::Through {
                pointer_point,
                place,
                mode,
                ..
            },
            ReferenceBorrowInput::Through { origins, .. },
        ) => origins
            .iter()
            .map(|parent_origin| {
                let fact = child_fact(point, function_span, place, *mode, &facts[parent_origin.0]);
                let origin = intern(facts, fact.clone());
                match fact {
                    ReferenceOrigin::Fresh { loan, .. }
                    | ReferenceOrigin::BorrowedFrom { loan, .. } => ReferenceBorrowCase::Accepted {
                        place: loan.origin_place.clone(),
                        loan,
                        origin,
                        access: ReferenceBorrowAccess::Through {
                            pointer_point: *pointer_point,
                            parent_origin: *parent_origin,
                        },
                    },
                    ReferenceOrigin::Invalid { .. } => ReferenceBorrowCase::Invalid {
                        parent_origin: *parent_origin,
                        origin,
                    },
                    ReferenceOrigin::Null { .. } => unreachable!("child invalid result"),
                }
            })
            .collect(),
        (_, ReferenceBorrowInput::MissingProducer { .. }) => Vec::new(),
        _ => unreachable!("borrow input kind"),
    }
}

pub(super) fn validates(
    facts: &[ReferenceOrigin],
    point: &ReferenceFlowPoint,
    function_span: ByteSpan,
    request: &ReferenceBorrowRequest,
    input: &ReferenceBorrowInput,
    case: &ReferenceBorrowCase,
) -> bool {
    match (request, input, case) {
        (
            ReferenceBorrowRequest::DirectStorage {
                place,
                loan,
                origin,
                ..
            },
            ReferenceBorrowInput::DirectStorage,
            ReferenceBorrowCase::Accepted {
                place: actual_place,
                loan: actual_loan,
                origin: actual_origin,
                access: ReferenceBorrowAccess::DirectStorage,
            },
        ) => {
            actual_place == place
                && actual_loan == loan
                && actual_origin == origin
                && loan.parent.is_none()
                && loan.origin_place == *place
                && loan.creation_source == point.source_span.source
                && loan.creation_span == point.source_span.range
                && matches!(&facts[origin.0], ReferenceOrigin::Fresh { loan: recorded, allocation }
                    if recorded == loan && allocation.source == point.source_span.source
                    && allocation.function_span == place.function_span
                    && allocation.declaration_span == place.declaration_span
                    && allocation.creation_span == point.source_span.range)
        }
        (
            ReferenceBorrowRequest::Through {
                pointer_point,
                place,
                mode,
                ..
            },
            ReferenceBorrowInput::Through {
                pointer_point: input_point,
                origins,
            },
            ReferenceBorrowCase::Accepted {
                place: actual_place,
                loan,
                origin,
                access:
                    ReferenceBorrowAccess::Through {
                        pointer_point: actual_point,
                        parent_origin,
                    },
            },
        ) => {
            if pointer_point != input_point
                || pointer_point != actual_point
                || !origins.contains(parent_origin)
            {
                return false;
            }
            let parent = &facts[parent_origin.0];
            if !matches!(
                parent,
                ReferenceOrigin::Fresh { .. } | ReferenceOrigin::BorrowedFrom { .. }
            ) {
                return false;
            }
            let expected = child_fact(point, function_span, place, *mode, parent);
            let expected_loan = match &expected {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                _ => unreachable!("expected child loan"),
            };
            actual_place == &expected_loan.origin_place
                && loan == expected_loan
                && facts[origin.0] == expected
        }
        (
            ReferenceBorrowRequest::Through {
                pointer_point,
                place,
                mode,
                ..
            },
            ReferenceBorrowInput::Through {
                pointer_point: input_point,
                origins,
            },
            ReferenceBorrowCase::Invalid {
                parent_origin,
                origin,
            },
        ) => {
            pointer_point == input_point
                && origins.contains(parent_origin)
                && matches!(
                    facts[parent_origin.0],
                    ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. }
                )
                && facts[origin.0]
                    == child_fact(point, function_span, place, *mode, &facts[parent_origin.0])
        }
        _ => false,
    }
}

pub(super) fn indices(request: &ReferenceBorrowRequest) -> &[ReferenceBorrowIndexInput] {
    match request {
        ReferenceBorrowRequest::DirectStorage { indices, .. }
        | ReferenceBorrowRequest::Through { indices, .. } => indices,
    }
}

pub(super) fn accepted_cases(
    cfg: &ScalarReferenceCfg,
    point: CfgPointId,
) -> Vec<&ReferenceBorrowCase> {
    let mut cases = Vec::new();
    for evaluation in cfg
        .borrow_evaluations
        .iter()
        .filter(|evaluation| evaluation.point == point)
    {
        for case in &evaluation.cases {
            if matches!(case, ReferenceBorrowCase::Accepted { .. }) && !cases.contains(&case) {
                cases.push(case);
            }
        }
    }
    cases
}

#[cfg(test)]
pub(super) fn accepted_loan(cfg: &ScalarReferenceCfg, point: CfgPointId) -> &ReferenceLoanId {
    let cases = accepted_cases(cfg, point);
    assert_eq!(cases.len(), 1);
    let ReferenceBorrowCase::Accepted { loan, .. } = cases[0] else {
        unreachable!()
    };
    loan
}
