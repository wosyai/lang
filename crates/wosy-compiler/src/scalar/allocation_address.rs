use super::*;

impl ReferenceFlowBuilder {
    pub(super) fn publish_rebind_associations(&mut self) {
        assert_eq!(
            self.rebind_associations.len(),
            self.rebind_association_count
        );
        for index in 0..self.rebind_association_count {
            self.cfg.rebind_associations.push(
                self.rebind_associations
                    .remove(&index)
                    .expect("complete constructed association"),
            );
        }
    }

    pub(super) fn direct_address_candidate(
        &mut self,
        place: &ScalarPlace,
        mode: ScalarReferenceMutability,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<(InvalidationCandidate, Vec<ReferenceBorrowIndexInput>), ReferenceAnalysisError>
    {
        let ScalarPlace::Name {
            name,
            span: name_span,
        } = reference_root(place)
        else {
            unreachable!("resolved direct storage")
        };
        let binding = self.binding(name, *name_span)?;
        let mut indices = Vec::new();
        self.borrow_indices(place, scope, &mut indices)?;
        let mut projections = Vec::new();
        collect_reference_projections(place, None, &mut projections);
        let loan = ReferenceLoanId {
            mode,
            origin_place: ReferencePlaceId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: binding.declaration_span,
                block_span: binding.block_span,
                binding: binding.clone(),
                projections,
                place: place.clone(),
            },
            creation_source: self.source.clone(),
            creation_span: span,
            parent: None,
        };
        let origin = self.intern(ReferenceOrigin::Fresh {
            allocation: ReturnedAllocationId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: binding.declaration_span,
                creation_span: span,
            },
            loan: loan.clone(),
        });
        Ok((
            InvalidationCandidate {
                origin,
                place: loan.origin_place.clone(),
                loan,
            },
            indices,
        ))
    }

    pub(super) fn allocation_identity_address(
        &mut self,
        expression: &ScalarExpression,
        association: ReferenceRebindAssociationId,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        let ScalarExpression::CheckedAddress {
            mutability, place, ..
        } = expression
        else {
            unreachable!("direct Rebind address")
        };
        let span = expression_span(expression);
        let (candidate, indices) =
            self.direct_address_candidate(place, *mutability, span, scope)?;
        let origin = candidate.origin;
        let point = self.point(
            CfgPointKind::AllocationIdentityAddress {
                request: ReferenceAllocationAddressRequest {
                    association,
                    candidate,
                    indices,
                },
            },
            span,
            scope,
            BTreeSet::new(),
        );
        self.cfg.evaluated_values.source_origin_derivations.push(source_boundary::allocation_identity_constructor::derivation(&self.cfg.producer_owner, point, origin, association));
        Ok(())
    }
}

fn missing(point: &ReferenceFlowPoint) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: point.source_span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
    }
}

fn connected(points: &[ReferenceFlowPoint], from: CfgPointId, to: CfgPointId) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![from];
    while let Some(id) = pending.pop() {
        if id == to {
            return true;
        }
        if seen.insert(id) {
            let Some(point) = points.get(id.0) else {
                return false;
            };
            pending.extend(&point.successors);
        }
    }
    false
}

fn candidate_corresponds(cfg: &ScalarReferenceCfg, candidate: &InvalidationCandidate) -> bool {
    let place = &candidate.place;
    let binding = &place.binding;
    if candidate.loan.origin_place != *place
        || place.source != binding.source
        || place.function_span != binding.function_span
        || place.declaration_span != binding.declaration_span
        || place.block_span != binding.block_span
    {
        return false;
    }
    match cfg.facts.get(candidate.origin.0) {
        Some(ReferenceOrigin::Fresh { allocation, loan }) => {
            loan == &candidate.loan
                && allocation.source == loan.creation_source
                && allocation.function_span == place.function_span
                && allocation.declaration_span == place.declaration_span
                && allocation.creation_span == loan.creation_span
        }
        Some(ReferenceOrigin::BorrowedFrom { loan, .. }) => loan == &candidate.loan,
        _ => false,
    }
}

pub(super) fn validate_association(
    cfg: &ScalarReferenceCfg,
    point: &ReferenceFlowPoint,
    request: &ReferenceAllocationAddressRequest,
) -> Result<ReferenceRebindAssociation, ReferenceAnalysisError> {
    let invalid = || missing(point);
    let association = cfg
        .rebind_associations
        .get(request.association.0)
        .ok_or_else(invalid)?;
    let points = &cfg.initial_points;
    let address = points
        .get(association.address_point.0)
        .ok_or_else(invalid)?;
    let effect = points.get(association.effect_point.0).ok_or_else(invalid)?;
    let call = points.get(association.call_point.0).ok_or_else(invalid)?;
    let raw = points.get(association.raw_point.0).ok_or_else(invalid)?;
    let length = points.get(association.length_point.0).ok_or_else(invalid)?;
    let CfgPointKind::AllocationIdentityAddress {
        request: actual_request,
    } = &address.kind
    else {
        return Err(invalid());
    };
    let CfgPointKind::Call {
        expression:
            ScalarExpression::Call {
                arguments,
                occurrence,
                ..
            },
        output: 0,
        target: ResolvedCallTarget::Core(CoreOperationId::Rebind),
        argument_points,
    } = &call.kind
    else {
        return Err(invalid());
    };
    let CfgPointKind::Invalidate {
        address_point,
        operation: CoreOperationId::Rebind,
        address_span,
        inputs:
            InvalidationInputs::Rebind {
                raw_address,
                raw_span,
                length: length_expression,
                length_span,
            },
        ..
    } = &effect.kind
    else {
        return Err(invalid());
    };
    let [ScalarExpression::CheckedAddress {
        place, mutability, ..
    }, actual_raw, actual_length] = arguments.as_slice()
    else {
        return Err(invalid());
    };
    let mut projections = Vec::new();
    collect_reference_projections(place, None, &mut projections);
    let candidate = &request.candidate;
    let ScalarPlace::Name {
        span: name_span, ..
    } = reference_root(place)
    else {
        return Err(invalid());
    };
    if association.owner != cfg.producer_owner
        || association.occurrence != **occurrence
        || association.occurrence.source_span != call.source_span
        || effect.source_span != call.source_span
        || address.source_span.source != call.source_span.source
        || *actual_request != *request
        || !matches!(point.id, id if id == address.id || id == effect.id)
        || *argument_points != [address.id, raw.id, length.id]
        || *address_point != address.id
        || *address_span != address.source_span
        || address.source_span.range != expression_span(&arguments[0])
        || raw_address != actual_raw
        || length_expression != actual_length
        || *raw_span != raw.source_span
        || *length_span != length.source_span
        || raw_span.range != expression_span(actual_raw)
        || length_span.range != expression_span(actual_length)
        || candidate.place.place != *place
        || candidate.place.projections != projections
        || candidate.loan.mode != *mutability
        || candidate.loan.parent.is_some()
        || candidate.loan.creation_source != address.source_span.source
        || candidate.loan.creation_span != address.source_span.range
        || candidate.place.source != address.source_span.source
        || !matches!(reference_root(&candidate.place.place), ScalarPlace::Name { span, .. } if span == name_span)
        || !candidate_corresponds(cfg, candidate)
        || !cfg.allocations.contains(&candidate.place.binding)
        || !connected(points, address.id, raw.id)
        || !connected(points, raw.id, length.id)
        || !connected(points, length.id, effect.id)
        || !effect.successors.contains(&call.id)
    {
        return Err(invalid());
    }
    for index in &request.indices {
        if !connected(points, index.value_point, address.id)
            || !matches!(
                candidate.place.projections.get(index.projection),
                Some(ReferencePlaceProjection::Index(_))
            )
        {
            return Err(invalid());
        }
    }
    Ok(association.clone())
}

fn available(
    cfg: &ScalarReferenceCfg,
    state: &ReferenceEdgeState,
    candidate: &InvalidationCandidate,
) -> bool {
    let binding = &candidate.place.binding;
    cfg.allocations.contains(binding)
        && !state.uninitialized.contains(binding)
        && !state.released.contains(binding)
        && !state
            .moved
            .iter()
            .any(|(place, _)| reference_places_overlap(place, &candidate.place))
        && state.backing_bindings.get(binding).is_some_and(|backings| {
            !backings.is_empty()
                && backings.iter().all(|backing| {
                    backing.owner == *binding && backing.kind == RuntimeBackingKind::OwnedAllocation
                })
        })
}

pub(super) fn evaluate_address(
    cfg: &mut ScalarReferenceCfg,
    state: &mut ReferenceEdgeState,
    point: &ReferenceFlowPoint,
    request: &ReferenceAllocationAddressRequest,
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    let checked = validate_association(cfg, point, request).and_then(|_| {
        if available(cfg, state, &request.candidate) {
            Ok(())
        } else {
            Err(missing(point))
        }
    });
    let (case, candidates) = match checked {
        Ok(()) => (
            ReferenceAllocationAddressCase::Accepted {
                candidate: request.candidate.clone(),
            },
            vec![request.candidate.clone()],
        ),
        Err(error) => {
            errors.push(error);
            (
                ReferenceAllocationAddressCase::Rejected {
                    diagnostic: ReferenceBorrowDiagnostic::Missing {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    },
                },
                Vec::new(),
            )
        }
    };
    state.allocation_address_values[point.id.0] = Some(ReferenceAllocationAddressValue {
        association: request.association,
        candidates,
    });
    state.value = Some(BTreeSet::new());
    let evaluation = ReferenceAllocationAddressEvaluation {
        owner: cfg.producer_owner.clone(),
        point: point.id,
        association: request.association,
        cases: vec![case],
    };
    if !cfg.allocation_address_evaluations.contains(&evaluation) {
        cfg.allocation_address_evaluations.push(evaluation);
    }
}

pub(super) fn validate_rebind_inputs(
    cfg: &ScalarReferenceCfg,
    state: &ReferenceEdgeState,
    point: &ReferenceFlowPoint,
) -> Result<(Vec<InvalidationCandidate>, CfgPointId, CfgPointId), ReferenceAnalysisError> {
    let CfgPointKind::Invalidate {
        address_point,
        operation: CoreOperationId::Rebind,
        inputs:
            InvalidationInputs::Rebind {
                raw_address,
                raw_span,
                length,
                length_span,
            },
        ..
    } = &point.kind
    else {
        return Err(missing(point));
    };
    let address_point = *address_point;
    let raw_address = raw_address.clone();
    let raw_span = raw_span.clone();
    let length = length.clone();
    let length_span = length_span.clone();
    let evaluated = (|| {
        let address = cfg
            .initial_points
            .get(address_point.0)
            .ok_or_else(|| missing(point))?;
        let call = point
            .successors
            .iter()
            .filter_map(|id| cfg.initial_points.get(id.0))
            .find(|call| {
                matches!(
                    &call.kind,
                    CfgPointKind::Call {
                        target: ResolvedCallTarget::Core(CoreOperationId::Rebind),
                        output: 0,
                        ..
                    }
                )
            })
            .ok_or_else(|| missing(point))?;
        let CfgPointKind::Call {
            argument_points,
            expression:
                ScalarExpression::Call {
                    arguments,
                    occurrence,
                    ..
                },
            ..
        } = &call.kind
        else {
            return Err(missing(point));
        };
        let [actual_address, raw_point, length_point] = argument_points.as_slice() else {
            return Err(missing(point));
        };
        if *actual_address != address_point
            || occurrence.source_span != point.source_span
            || arguments.get(1) != Some(&raw_address)
            || arguments.get(2) != Some(&length)
            || cfg.initial_points.get(raw_point.0).map(|p| &p.source_span) != Some(&raw_span)
            || cfg
                .initial_points
                .get(length_point.0)
                .map(|p| &p.source_span)
                != Some(&length_span)
            || !connected(&cfg.initial_points, address_point, *raw_point)
            || !connected(&cfg.initial_points, *raw_point, *length_point)
            || !connected(&cfg.initial_points, *length_point, point.id)
            || state
                .point_values
                .get(raw_point.0)
                .and_then(Option::as_ref)
                .is_none()
            || state
                .point_values
                .get(length_point.0)
                .and_then(Option::as_ref)
                .is_none()
        {
            return Err(missing(point));
        }
        let candidates = match &address.kind {
            CfgPointKind::AllocationIdentityAddress { request } => {
                validate_association(cfg, point, request)?;
                let value = state
                    .allocation_address_values
                    .get(address_point.0)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| missing(point))?;
                if value.association != request.association
                    || value.candidates != [request.candidate.clone()]
                {
                    return Err(missing(point));
                }
                value.candidates.clone()
            }
            _ => {
                let origins = state
                    .point_values
                    .get(address_point.0)
                    .and_then(Option::as_ref)
                    .filter(|origins| !origins.is_empty())
                    .ok_or_else(|| missing(point))?;
                let mut candidates = Vec::new();
                for origin in origins {
                    let loan = match cfg.facts.get(origin.0) {
                        Some(
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. },
                        ) => loan,
                        _ => return Err(missing(point)),
                    };
                    candidates.push(InvalidationCandidate {
                        origin: *origin,
                        place: loan.origin_place.clone(),
                        loan: loan.clone(),
                    });
                }
                candidates
            }
        };
        let donor_uses = binding_holder::uses(cfg);
        for candidate in &candidates {
            if !candidate_corresponds(cfg, candidate) || !available(cfg, state, candidate) {
                return Err(missing(point));
            }
            let origins = conflicting_live_loan(
                &cfg.initial_points,
                &donor_uses,
                &cfg.facts,
                &cfg.environments,
                state,
                point.id,
                &candidate.place,
                ScalarReferenceMutability::Mutable,
                AccessAuthority::Through(candidate.origin),
            );
            if !origins.is_empty() {
                return Err(ReferenceAnalysisError::ConflictingPlace {
                    source_span: point.source_span.clone(),
                    origins,
                });
            }
            let origins = live_captured_backing_origins(
                &cfg.initial_points,
                &donor_uses,
                &cfg.environments,
                state,
                point.id,
                &candidate.place.binding,
            );
            if !origins.is_empty() {
                return Err(ReferenceAnalysisError::ConflictingPlace {
                    source_span: point.source_span.clone(),
                    origins,
                });
            }
        }
        Ok((candidates, *raw_point, *length_point))
    })();
    evaluated
}
