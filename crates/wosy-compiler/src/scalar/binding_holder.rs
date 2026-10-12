use super::*;

fn invalid(input: &BindingOutputInput) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: input.occurrence.source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

pub(super) fn field_input(cfg: &ScalarReferenceCfg, point: CfgPointId) -> Result<Option<&BindingOutputInput>, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: cfg.points[point.0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let rows = cfg.binding_points.iter().filter(|row| row.point == point
        && row.role == ScalarBindingPointRole::CopyCallableFields).collect::<Vec<_>>();
    match rows.as_slice() {
        [] => {
            let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Assignment(_), .. } = &cfg.initial_points[point.0].kind else { return Err(invalid()); };
            let assignments = cfg.assignment_points.iter().filter(|row| row.point == point
                && row.role == ScalarAssignmentPointRole::CopyCallableFields).collect::<Vec<_>>();
            if assignments.len() != 1 { return Err(invalid()); }
            Ok(None)
        }
        [row] => {
            let receivers = cfg.binding_points.iter().filter(|receiver| receiver.role == ScalarBindingPointRole::Bind
                && receiver.occurrence == row.occurrence && receiver.output == row.output).collect::<Vec<_>>();
            let [receiver] = receivers.as_slice() else { return Err(invalid()); };
            let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == receiver.point).collect::<Vec<_>>();
            let [input] = inputs.as_slice() else { return Err(invalid()); };
            let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(source), target } = &cfg.initial_points[point.0].kind else { return Err(invalid()); };
            if !matches!(&cfg.points[receiver.point.0].kind, CfgPointKind::Bind { binding } if binding == target)
                || input.source != *source || row.source != receiver.source || input.occurrence != row.occurrence || input.output != row.output
                || !dominates(&cfg.points, receiver.point, point) { return Err(invalid()); }
            Ok(Some(*input))
        }
        _ => Err(invalid()),
    }
}

pub(super) fn field_prefix(cfg: &ScalarReferenceCfg, point: CfgPointId, receipt: Option<&evaluated_aggregate_receiving::AggregateReceiptPlan>) -> Result<Vec<ReferencePlaceProjection>, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: cfg.points[point.0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let Some(receipt) = receipt else { return Ok(Vec::new()); };
    match &cfg.initial_points[receipt.source.point.0].kind {
        CfgPointKind::ReadPlace { candidates, .. } => {
            let [place] = candidates.as_slice() else { return Err(invalid()); };
            let place = available_place(place);
            Ok(place.projections)
        }
        CfgPointKind::Read { .. } | CfgPointKind::Call { .. } => Ok(Vec::new()),
        CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }) => {
            Ok(Vec::new())
        }
        _ => Err(invalid()),
    }
}

pub(super) fn field_relative(place: &ReferencePlaceId, source: &ReferenceBindingId, prefix: &[ReferencePlaceProjection]) -> Option<ReferencePlaceId> {
    if place.binding != *source || place.projections.len() < prefix.len() { return None; }
    let mut selected = place.clone();
    selected.projections = prefix.to_vec();
    let mut actual = place.clone();
    actual.projections.truncate(prefix.len());
    if !reference_places_same(&selected, &actual) { return None; }
    let mut relative = place.clone();
    relative.projections.drain(..prefix.len());
    Some(relative)
}

pub(super) fn field_first(cfg: &ScalarReferenceCfg, point: CfgPointId, receipt: Option<&evaluated_aggregate_receiving::AggregateReceiptPlan>) -> Result<Option<CfgPointId>, ReferenceAnalysisError> {
    let Some(input) = field_input(cfg, point)? else { return Ok(None); };
    let Some(first) = first(cfg, input) else { return Ok(None); };
    let CfgPointKind::CopyCallableFields { source, .. } = &cfg.points[point.0].kind else { unreachable!("actual copy event"); };
    if !matches!(cfg.initial_points[input.source.point.0].kind, CfgPointKind::Call { .. }) { return Ok(None); }
    let copies = cfg.binding_points.iter().filter(|copy| copy.role == ScalarBindingPointRole::CopyCallableFields
        && copy.occurrence == first.occurrence && copy.output == first.output).collect::<Vec<_>>();
    let [copy] = copies.as_slice() else { return Err(invalid(input)); };
    let CfgPointKind::CopyCallableFields { source: original, target } = &cfg.points[copy.point.0].kind else { return Err(invalid(input)); };
    if original != source || !matches!(&cfg.points[first.consumer.0].kind, CfgPointKind::Bind { binding } if binding == target)
        || field_prefix(cfg, copy.point, Some(&evaluated_aggregate_receiving::plan(cfg, copy.point)?))? != field_prefix(cfg, point, receipt)?
        || !dominates(&cfg.points, copy.point, point) { return Err(invalid(input)); }
    Ok(Some(copy.point))
}

pub(super) fn field_route(cfg: &ScalarReferenceCfg, point: CfgPointId, receipt: Option<&evaluated_aggregate_receiving::AggregateReceiptPlan>) -> Result<(ReferenceBindingId, Vec<ReferencePlaceProjection>, Option<CfgPointId>), ReferenceAnalysisError> {
    let CfgPointKind::CopyCallableFields { source, .. } = &cfg.points[point.0].kind else { unreachable!("actual copy event"); };
    match field_first(cfg, point, receipt)? {
        Some(first) => {
            let CfgPointKind::CopyCallableFields { target, .. } = &cfg.points[first.0].kind else { unreachable!("validated first copy event"); };
            Ok((target.clone(), Vec::new(), Some(first)))
        }
        None => match source {
            CallableFieldSource::Assignment(binding) => Ok((binding.clone(), field_prefix(cfg, point, receipt)?, None)),
            CallableFieldSource::Evaluated(_) => {
                let Some(input) = field_input(cfg, point)? else {
                    return Err(ReferenceAnalysisError::MissingContext {
                        source_span: cfg.points[point.0].source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    });
                };
                let donor = match &cfg.points[input.source.point.0].kind {
                    CfgPointKind::Read { binding } => binding.clone(),
                    CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1
                        && !matches!(candidates[0].binding.kind, ReferenceBindingKind::CallOutput { .. }) => candidates[0].binding.clone(),
                    _ => {
                        if !cfg.binding_points.iter().any(|binding| binding.point == input.consumer && binding.role == ScalarBindingPointRole::Bind) {
                            return Err(ReferenceAnalysisError::MissingContext {
                                source_span: cfg.points[point.0].source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                            });
                        }
                        let consumer = match first(cfg, input) {
                            Some(first_input) => first_input.consumer,
                            None => input.consumer,
                        };
                        let CfgPointKind::Bind { binding } = &cfg.points[consumer.0].kind else {
                            return Err(ReferenceAnalysisError::MissingContext {
                                source_span: cfg.points[point.0].source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                            });
                        };
                        binding.clone()
                    }
                };
                Ok((donor, field_prefix(cfg, point, receipt)?, None))
            },
        },
    }
}

pub(super) fn field_donor(
    points: &[ReferenceFlowPoint],
    point: CfgPointId,
    first: Option<CfgPointId>,
    state: &ReferenceEdgeState,
    original: &ReferencePlaceId,
    relative: &ReferencePlaceId,
    relation: &CallableHolderRelation,
) -> Result<CallableHolderPlace, ReferenceAnalysisError> {
    let missing = || ReferenceAnalysisError::MissingContext { source_span: points[point.0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer };
    let source = CallableHolderPlace::Projection(original.clone());
    let donor = match first {
        Some(first) => {
            let CfgPointKind::CopyCallableFields { target, .. } = &points[first.0].kind else { return Err(missing()); };
            let mut place = relative.clone();
            place.binding = target.clone();
            place.source = target.source.clone();
            place.function_span = target.function_span;
            place.declaration_span = target.declaration_span;
            place.block_span = target.block_span;
            CallableHolderPlace::Projection(place)
        }
        None => source.clone(),
    };
    if !state.holder_relations.iter().any(|live| live.environment == relation.environment
        && live.capture_paths == relation.capture_paths && live.holders.iter().any(|holder| holder_place_same(holder, &donor))) { return Err(missing()); }
    let chain = state.holder_provenance.lookup(&relation.environment, &donor).ok_or_else(missing)?;
    if !chain_environment(chain, &relation.environment) { return Err(missing()); }
    if let Some(first) = first {
        let HolderProducerRef::Forward { point, action, previous, .. } = chain else { return Err(missing()); };
        if *point != first || !matches!(action, CallableHolderAction::Move { environment, from, to, .. }
            if environment == &relation.environment && holder_place_same(from, &source) && holder_place_same(to, &donor))
            || !chain_environment(previous, &relation.environment) { return Err(missing()); }
        let ReferenceBindingKind::CallOutput { point: call, output } = original.binding.kind else { return Err(missing()); };
        let CfgPointKind::Call { target, output: actual_output, .. } = &points[call.0].kind else { return Err(missing()); };
        let HolderProducerRef::CallCreation { point, environment, selection } = previous.as_ref() else { return Err(missing()); };
        if *point != call || environment != &relation.environment || selection.call != call
            || selection.target != *target || selection.output != output || *actual_output != output
            || selection.destination != relation.environment || selection.projection != original.projections {
            return Err(missing());
        }
    }
    Ok(donor)
}

fn group<'a>(cfg: &'a ScalarReferenceCfg, input: &BindingOutputInput) -> Vec<&'a BindingOutputInput> {
    let mut rows = cfg.evaluated_values.binding_inputs.iter().filter(|row|
        row.producer == input.producer && row.occurrence == input.occurrence && row.source == input.source).collect::<Vec<_>>();
    rows.sort_by_key(|row| row.output);
    rows
}

fn reaches(points: &[ReferenceFlowPoint], start: CfgPointId, end: CfgPointId, excluded: Option<CfgPointId>) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(point) = pending.pop() {
        if Some(point) == excluded { continue; }
        if point == end { return true; }
        if visited.insert(point) { pending.extend(&points[point.0].successors); }
    }
    false
}

pub(super) fn dominates(points: &[ReferenceFlowPoint], before: CfgPointId, after: CfgPointId) -> bool {
    reaches(points, CfgPointId(0), after, None)
        && reaches(points, before, after, None)
        && !reaches(points, CfgPointId(0), after, Some(before))
}

pub(super) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    for point in &cfg.points {
        if matches!(point.kind, CfgPointKind::CopyCallableFields { .. }) { field_input(cfg, point.id)?; }
    }
    for receiving in &cfg.binding_points {
        if receiving.role == ScalarBindingPointRole::CopyCallableFields {
            let invalid = || ReferenceAnalysisError::MissingContext { source_span: receiving.occurrence.source_span.clone(),
                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence };
            let point = cfg.points.get(receiving.point.0).ok_or_else(invalid)?;
            if !matches!(point.kind, CfgPointKind::CopyCallableFields { .. }) { return Err(invalid()); }
        }
    }
    for input in &cfg.evaluated_values.binding_inputs {
        cfg.binding_points.iter().find(|binding| binding.point == input.consumer
            && binding.role == ScalarBindingPointRole::Bind).ok_or_else(|| invalid(input))?;
        let mut declaration = cfg.binding_points.iter().filter(|binding| binding.role == ScalarBindingPointRole::Bind
            && binding.occurrence == input.occurrence).collect::<Vec<_>>();
        declaration.sort_by_key(|binding| cfg.points[binding.point.0].source_span.range.start);
        if declaration.iter().enumerate().any(|(ordinal, binding)| binding.output != ordinal) { return Err(invalid(input)); }
        let rows = group(cfg, input);
        let first = rows.first().ok_or_else(|| invalid(input))?;
        let CfgPointKind::Bind { binding: first_binding } = &cfg.points[first.consumer.0].kind else { return Err(invalid(input)); };
        let policy = output_sequence::binding_disposition(&cfg.evaluated_values, first, first_binding)?.policy;
        if !dominates(&cfg.points, input.source.point, first.consumer) { return Err(invalid(input)); }
        for pair in rows.windows(2) {
            if pair[0].output >= pair[1].output || !dominates(&cfg.points, pair[0].consumer, pair[1].consumer) { return Err(invalid(input)); }
        }
        for row in &rows {
            let CfgPointKind::Bind { binding } = &cfg.points[row.consumer.0].kind else { return Err(invalid(input)); };
            if output_sequence::binding_disposition(&cfg.evaluated_values, row, binding)?.policy != policy { return Err(invalid(input)); }
        }
        if rows.len() > 1 && policy != CopyPolicy::Copy {
            return Err(ReferenceAnalysisError::MissingContext {
                source_span: input.occurrence.source_span.clone(),
                reason: if policy == CopyPolicy::Deferred { ReferenceAnalysisErrorReason::MissingResolvedBinding }
                    else { ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence },
            });
        }
    }
    evaluated_aggregate_receiving::validate(cfg)
}

pub(super) fn first<'a>(cfg: &'a ScalarReferenceCfg, input: &BindingOutputInput) -> Option<&'a BindingOutputInput> {
    cfg.binding_points.iter().find(|binding| binding.point == input.consumer
        && binding.role == ScalarBindingPointRole::Bind).expect("validated actual receiving event");
    let rows = group(cfg, input);
    let first = *rows.first().expect("validated actual receiving group");
    if first.consumer == input.consumer { None } else { Some(first) }
}

pub(super) fn uses(cfg: &ScalarReferenceCfg) -> BTreeMap<CfgPointId, ReferenceBindingId> {
    let mut uses = cfg.evaluated_values.binding_inputs.iter().filter_map(|input| {
        first(cfg, input).map(|first| {
            let CfgPointKind::Bind { binding } = &cfg.points[first.consumer.0].kind else { unreachable!("validated first Bind"); };
            let donor = match &cfg.points[input.source.point.0].kind {
                CfgPointKind::Read { binding } => binding,
                CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1
                    && !matches!(candidates[0].binding.kind, ReferenceBindingKind::CallOutput { .. }) => &candidates[0].binding,
                _ => binding,
            };
            (input.consumer, donor.clone())
        })
    }).collect::<BTreeMap<_, _>>();
    for receiving in &cfg.binding_points {
        if receiving.role == ScalarBindingPointRole::CopyCallableFields {
            let receipt = evaluated_aggregate_receiving::plan(cfg, receiving.point).expect("validated actual aggregate receipt");
            let (source, _, _) = field_route(cfg, receiving.point, Some(&receipt)).expect("validated callable-field receiving route");
            uses.insert(receiving.point, source);
        }
    }
    uses
}

fn chain_environment(chain: &HolderProducerRef, environment: &CallableEnvironmentId) -> bool {
    match chain {
        HolderProducerRef::CaptureCreation { environment: actual, .. }
        | HolderProducerRef::CallCreation { environment: actual, .. }
        | HolderProducerRef::InstalledRead { environment: actual, .. }
        | HolderProducerRef::GlobalCallsiteInput { environment: actual, .. } => actual == environment,
        HolderProducerRef::Forward { environment: actual, previous, .. } =>
            actual == environment && chain_environment(previous, environment),
    }
}

pub(super) fn donor(
    points: &[ReferenceFlowPoint],
    first: Option<&BindingOutputInput>,
    state: &ReferenceEdgeState,
    input: &BindingOutputInput,
    source: &CallableHolderPlace,
    relation: &CallableHolderRelation,
) -> Result<CallableHolderPlace, ReferenceAnalysisError> {
    let environment = &relation.environment;
    let missing = || ReferenceAnalysisError::MissingContext {
        source_span: points[input.consumer.0].source_span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    let consumed = matches!(source, CallableHolderPlace::Temporary(_))
        || matches!(source, CallableHolderPlace::Projection(place) if matches!(place.binding.kind, ReferenceBindingKind::CallOutput { .. }));
    let donor = if consumed {
        if let Some(first) = first {
            let CfgPointKind::Bind { binding } = &points[first.consumer.0].kind else { return Err(missing()); };
            match source {
                CallableHolderPlace::Temporary(_) => CallableHolderPlace::Binding(binding.clone()),
                CallableHolderPlace::Projection(place) => {
                    if matches!(points[input.source.point.0].kind, CfgPointKind::ReadPlace { .. }) {
                        CallableHolderPlace::Binding(binding.clone())
                    } else {
                        let mut projected = place.clone();
                        projected.source = binding.source.clone();
                        projected.function_span = binding.function_span;
                        projected.declaration_span = binding.declaration_span;
                        projected.block_span = binding.block_span;
                        projected.binding = binding.clone();
                        CallableHolderPlace::Projection(projected)
                    }
                }
                _ => unreachable!(),
            }
        } else { source.clone() }
    } else { source.clone() };
    if !state.holder_relations.iter().any(|live| live.environment == *environment
        && live.capture_paths == relation.capture_paths
        && live.holders.iter().any(|holder| holder_place_same(holder, &donor))) { return Err(missing()); }
    let chain = state.holder_provenance.lookup(environment, &donor).ok_or_else(missing)?;
    if !chain_environment(chain, environment) { return Err(missing()); }
    if consumed {
        if let Some(first) = first {
            let HolderProducerRef::Forward { point, environment: actual, action, previous } = chain else { return Err(missing()); };
            if *point != first.consumer || actual != environment
                || !matches!(action, CallableHolderAction::Move { environment: actual, from, to, .. }
                    if actual == environment && holder_place_same(from, source) && holder_place_same(to, &donor))
                || !chain_environment(previous, environment) { return Err(missing()); }
            if matches!(&points[input.source.point.0].kind, CfgPointKind::Capture { .. })
                && !matches!(previous.as_ref(), HolderProducerRef::CaptureCreation { point, environment: actual }
                    if *point == input.source.point && actual == environment) { return Err(missing()); }
        }
    }
    Ok(donor)
}
