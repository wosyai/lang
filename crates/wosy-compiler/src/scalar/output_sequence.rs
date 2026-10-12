use super::*;

#[cfg(test)]
#[path = "direct_binding_tests.rs"]
mod direct_binding_tests;

pub(in crate::scalar) fn binding_disposition<'a>(
    tables: &'a EvaluatedValueTables,
    input: &BindingOutputInput,
    binding: &ReferenceBindingId,
) -> Result<&'a ResolvedValueDispositionInput, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: input.occurrence.source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let consumer = ValueConsumerId {
        producer: input.producer.clone(),
        point: input.consumer,
        kind: ValueConsumerKind::Binding { binding: binding.clone(), projection: Vec::new() },
    };
    let rows = tables.disposition_inputs.iter().filter(|row| row.consumer == consumer).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(invalid()); };
    let facts = tables.values.iter().filter(|fact| fact.id == input.source).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(invalid()); };
    let shapes = tables.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
    if shapes.len() != 1 || fact.shape.producer != input.producer
        || input.source.resolved.producer != input.producer || row.source != input.source {
        return Err(invalid());
    }
    Ok(row)
}

fn binding_source_key(source: &ScalarBindingOutputSource) -> Option<(&SourceSpan, &[usize], usize)> {
    match source {
        ScalarBindingOutputSource::Direct { occurrence, output_projection, output } =>
            Some((occurrence, output_projection, *output)),
        ScalarBindingOutputSource::Call { occurrence, output } =>
            Some((&occurrence.source_span, &occurrence.output_projection, *output)),
        ScalarBindingOutputSource::Conditional { occurrence, then_block, output } =>
            Some((occurrence, &then_block.output_projection, *output)),
        ScalarBindingOutputSource::Block { occurrence, output } =>
            Some((&occurrence.source_span, &occurrence.output_projection, *output)),
        ScalarBindingOutputSource::Expression { .. } => None,
    }
}

pub(super) fn retained_source_value<'a>(
    cfg: &'a ScalarReferenceCfg,
    source: &ScalarBindingOutputSource,
    receiver: &SourceSpan,
) -> Result<&'a EvaluatedValueId, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: receiver.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let (occurrence, projection, output) = binding_source_key(source).ok_or_else(invalid)?;
    if matches!(source, ScalarBindingOutputSource::Direct { .. }) {
        let facts = cfg.evaluated_values.values.iter().filter(|fact| {
            fact.id.resolved.producer == cfg.producer_owner
                && fact.id.resolved.occurrence == *occurrence
                && fact.id.resolved.output_projection == projection
                && fact.id.resolved.output == output
        }).collect::<Vec<_>>();
        let [fact] = facts.as_slice() else { return Err(invalid()); };
        let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else { return Err(invalid()); };
        let point = cfg.points.get(fact.id.point.0).ok_or_else(invalid)?;
        if shape.id.producer != cfg.producer_owner || point.id != fact.id.point
            || point.source_span != *occurrence || output != 0 {
            return Err(invalid());
        }
        match &point.kind {
            CfgPointKind::Capture { literal, environment, signature, body, .. } => {
                let captures = cfg.capture_points.iter().filter(|capture| {
                    capture.point == point.id && capture.role == ScalarCapturePointRole::Capture
                }).collect::<Vec<_>>();
                let [capture] = captures.as_slice() else { return Err(invalid()); };
                if *literal != occurrence.range || environment.literal != *occurrence
                    || !scalar_type_equal(signature, &shape.ty)
                    || capture.occurrence != ScalarCaptureOccurrence::for_literal(*literal, body)
                    || body.occurrence.source_span.source != occurrence.source {
                    return Err(invalid());
                }
            }
            CfgPointKind::Evaluate(expression) => {
                if expression_span(expression) != occurrence.range || !projection.is_empty()
                    || matches!(expression, ScalarExpression::Assignment(_) | ScalarExpression::Call { .. }
                        | ScalarExpression::If { .. } | ScalarExpression::UnitIf { .. } | ScalarExpression::Block(_)) {
                    return Err(invalid());
                }
            }
            CfgPointKind::Read { .. } | CfgPointKind::Borrow { .. }
                | CfgPointKind::AllocationIdentityAddress { .. } if projection.is_empty() => {}
            CfgPointKind::ReadPlace { .. } => {
                if !projection.is_empty() && !cfg.read_points.iter().any(|read| read.point == point.id
                    && read.occurrence.source_span == *occurrence
                    && read.occurrence.output_projection == projection) {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
        return Ok(&fact.id);
    }
    let productions = cfg.evaluated_values.productions.iter().filter(|production| {
        production.producer == cfg.producer_owner && production.occurrence == *occurrence
            && production.output_projection == projection
    }).collect::<Vec<_>>();
    let [production] = productions.as_slice() else { return Err(invalid()); };
    match (source, &production.source) {
        (ScalarBindingOutputSource::Call { occurrence, .. }, OutputProductionSource::Call { occurrence: actual, .. })
            if occurrence == actual => {}
        (ScalarBindingOutputSource::Conditional { then_block, .. }, OutputProductionSource::Conditional { branch, .. }) => {
            let branches = cfg.conditional_points.iter().filter(|event| {
                event.then_block == *then_block && event.role == ScalarConditionalPointRole::Branch
            }).collect::<Vec<_>>();
            if !matches!(branches.as_slice(), [event] if event.point == *branch) { return Err(invalid()); }
        }
        (ScalarBindingOutputSource::Block { occurrence, .. }, OutputProductionSource::Block { occurrence: actual, .. })
            if occurrence == actual => {}
        _ => return Err(invalid()),
    }
    let value = production.outputs.get(output).ok_or_else(invalid)?;
    let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *value).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(invalid()); };
    let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
    if shapes.len() != 1 || fact.shape.producer != cfg.producer_owner
        || value.resolved.producer != cfg.producer_owner || value.resolved.occurrence != *occurrence
        || value.resolved.output_projection != projection || value.resolved.output != output {
        return Err(invalid());
    }
    let point = cfg.points.get(value.point.0).ok_or_else(invalid)?;
    if point.id != value.point || point.source_span != *occurrence { return Err(invalid()); }
    Ok(value)
}

#[cfg(test)]
pub(in crate::scalar) fn assert_call_binding_channels(
    cfg: &ScalarReferenceCfg,
    trace: &ReferenceValidationTestTrace,
) {
    let pass = trace
        .passes
        .iter()
        .rfind(|pass| pass.owner == cfg.producer_owner)
        .expect("actual final producer pass");
    assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
    for input in &cfg.evaluated_values.binding_inputs {
        let fact = cfg
            .evaluated_values
            .values
            .iter()
            .find(|fact| fact.id == input.source)
            .expect("actual binding source fact");
        let shape = cfg
            .evaluated_values
            .shapes
            .iter()
            .find(|shape| shape.id == fact.shape)
            .expect("actual binding source shape");
        if !matches!(shape.ty, ScalarType::Callable { .. }) {
            continue;
        }
        let CfgPointKind::Bind { binding } = &cfg.initial_points[input.consumer.0].kind else {
            unreachable!()
        };
        let before = pass
            .observations
            .points
            .iter()
            .filter(|snapshot| {
                snapshot.point == input.consumer
                    && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
            })
            .collect::<Vec<_>>();
        assert!(!before.is_empty());
        for snapshot in before {
            let environments = &snapshot.state.callable_point_values[input.source.point.0];
            assert!(!environments.is_empty());
            let source = CallableHolderPlace::Temporary(input.source.point);
            assert!(snapshot.state.holder_point_sources[input.source.point.0].contains(&source));
            for environment in environments {
                assert!(snapshot
                    .state
                    .holder_relations
                    .iter()
                    .any(|relation| relation.environment == *environment
                        && relation.holders.contains(&source)));
                assert!(snapshot
                    .state
                    .holder_provenance
                    .lookup(environment, &source)
                    .is_some());
                assert!(cfg.holder_edges.iter().filter(|edge| edge.from == input.consumer).flat_map(|edge| &edge.actions).any(|action| {
                    matches!(action, CallableHolderAction::Move { environment: actual, from, to: CallableHolderPlace::Binding(target), .. }
                        if actual == environment && from == &source && target == binding)
                }));
            }
        }
        let after = pass
            .observations
            .points
            .iter()
            .filter(|snapshot| {
                snapshot.point == input.consumer
                    && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
            })
            .collect::<Vec<_>>();
        assert!(!after.is_empty());
        for snapshot in after {
            let environments = &snapshot.state.callable_point_values[input.source.point.0];
            assert_eq!(&snapshot.state.callable_bindings[binding], environments);
            for environment in environments {
                let holder = CallableHolderPlace::Binding(binding.clone());
                assert!(snapshot
                    .state
                    .holder_relations
                    .iter()
                    .any(|relation| relation.environment == *environment
                        && relation.holders.contains(&holder)));
                assert!(snapshot
                    .state
                    .holder_provenance
                    .lookup(environment, &holder)
                    .is_some());
                assert!(!snapshot
                    .state
                    .holder_relations
                    .iter()
                    .any(|relation| relation.environment == *environment
                        && relation
                            .holders
                            .contains(&CallableHolderPlace::Temporary(input.source.point))));
            }
        }
        let calls = cfg.points.iter().filter(|point| matches!(&point.kind,
            CfgPointKind::Call { target: ResolvedCallTarget::LocalCallable(callee), output: 0, .. } if callee == binding)).collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].possible_callee_environments,
            cfg.points[input.consumer.0].possible_environments
        );
    }
    let input = cfg
        .evaluated_values
        .binding_inputs
        .iter()
        .find(|input| input.source.resolved.output == 0)
        .expect("actual required Call binding row");
    let retained = cfg
        .binding_points
        .iter()
        .find(|binding| {
            binding.point == input.consumer && binding.role == ScalarBindingPointRole::Bind
        })
        .expect("actual retained binding source");
    assert!(
        matches!(&retained.source, ScalarBindingOutputSource::Call { occurrence, output }
        if occurrence.source_span == input.source.resolved.occurrence && *output == input.source.resolved.output
            && occurrence.output_projection == input.source.resolved.output_projection)
    );
    let mut malformed = cfg.clone();
    malformed
        .evaluated_values
        .binding_inputs
        .retain(|row| row.consumer != input.consumer);
    assert_eq!(
        malformed.evaluated_values.binding_inputs.len() + 1,
        cfg.evaluated_values.binding_inputs.len()
    );
    assert_eq!(malformed.binding_points, cfg.binding_points);
    let before = malformed.clone();
    let mut negative_trace = ReferenceValidationTestTrace::new();
    let errors = transfer_reference_facts(
        &mut malformed,
        &pass.context.deferred_policies,
        &pass.context.contracts,
        &pass.context.literal_contracts,
        &pass.context.call_exits,
        &pass.context.capture_input,
        pass.role.clone(),
        &mut negative_trace,
    );
    assert_eq!(
        errors,
        vec![ReferenceAnalysisError::MissingContext {
            source_span: retained.occurrence.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        }]
    );
    assert_eq!(malformed, before);
    assert_eq!(negative_trace.passes.len(), 1);
    assert_eq!(negative_trace.passes[0].owner, cfg.producer_owner);
    assert!(negative_trace.passes[0].observations.points.is_empty());
    assert!(negative_trace.passes[0].observations.borrows.is_empty());
}

pub(super) fn call_argument_expressions<'a>(
    tables: &EvaluatedValueTables,
    arguments: &'a [ScalarExpression],
) -> Vec<&'a ScalarExpression> {
    let mut inputs = Vec::new();
    for argument in arguments {
        if let ScalarExpression::Call { occurrence, .. } = argument {
            let production = tables
                .productions
                .iter()
                .find(|production| {
                    production.occurrence == occurrence.source_span
                        && production.output_projection == occurrence.output_projection
                })
                .expect("actual argument call production");
            inputs.extend(production.outputs.iter().map(|_| argument));
        } else {
            inputs.push(argument);
        }
    }
    inputs
}

#[cfg(test)]
pub(in crate::scalar) fn assert_control_binding_channels(
    cfg: &ScalarReferenceCfg,
    trace: &ReferenceValidationTestTrace,
) {
    let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
    assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
    let mut callable = 0;
    for binding in cfg.binding_points.iter().filter(|binding| binding.role == ScalarBindingPointRole::Bind
        && matches!(binding.source, ScalarBindingOutputSource::Conditional { .. } | ScalarBindingOutputSource::Block { .. })) {
        let input = cfg.evaluated_values.binding_inputs.iter().find(|input| input.consumer == binding.point).unwrap();
        let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
        let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap();
        if matches!(shape.ty, ScalarType::Callable { .. }) {
            callable += 1;
            let CfgPointKind::Bind { binding: receiver } = &cfg.initial_points[binding.point.0].kind else { panic!("actual control receiver"); };
            let before = pass.observations.points.iter().filter(|snapshot| snapshot.point == input.consumer
                && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation).collect::<Vec<_>>();
            assert!(!before.is_empty());
            for snapshot in before {
                let environments = &snapshot.state.callable_point_values[input.source.point.0];
                assert!(!environments.is_empty());
                let source = CallableHolderPlace::Temporary(input.source.point);
                assert!(snapshot.state.holder_point_sources[input.source.point.0].contains(&source));
                for environment in environments {
                    assert!(snapshot.state.holder_relations.iter().any(|relation| relation.environment == *environment && relation.holders.contains(&source)));
                    assert!(snapshot.state.holder_provenance.lookup(environment, &source).is_some());
                    assert!(cfg.holder_edges.iter().filter(|edge| edge.from == input.consumer).flat_map(|edge| &edge.actions).any(|action| {
                        matches!(action, CallableHolderAction::Move { environment: actual, from, to: CallableHolderPlace::Binding(target), .. }
                            if actual == environment && from == &source && target == receiver)
                    }));
                }
            }
            let after = pass.observations.points.iter().filter(|snapshot| snapshot.point == input.consumer
                && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).collect::<Vec<_>>();
            assert!(!after.is_empty());
            for snapshot in after {
                let environments = &snapshot.state.callable_point_values[input.source.point.0];
                assert_eq!(&snapshot.state.callable_bindings[receiver], environments);
                for environment in environments {
                    let holder = CallableHolderPlace::Binding(receiver.clone());
                    assert!(snapshot.state.holder_relations.iter().any(|relation| relation.environment == *environment && relation.holders.contains(&holder)));
                    assert!(snapshot.state.holder_provenance.lookup(environment, &holder).is_some());
                    assert!(snapshot.state.holder_provenance.lookup(environment, &CallableHolderPlace::Temporary(input.source.point)).is_none());
                }
            }
            let calls = cfg.points.iter().filter(|point| matches!(&point.kind,
                CfgPointKind::Call { target: ResolvedCallTarget::LocalCallable(callee), output: 0, .. } if callee == receiver)).collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].possible_callee_environments, cfg.points[input.consumer.0].possible_environments);
        }
        let mut malformed = cfg.clone();
        malformed.evaluated_values.binding_inputs.retain(|row| row.consumer != binding.point);
        let before = malformed.clone();
        let mut negative_trace = ReferenceValidationTestTrace::new();
        let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
            &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
            &pass.context.capture_input,
            pass.role.clone(), &mut negative_trace);
        assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
            source_span: binding.occurrence.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        }]);
        assert_eq!(malformed, before);
        assert_eq!(negative_trace.passes.len(), 1);
        assert!(negative_trace.passes[0].observations.points.is_empty());
        assert!(negative_trace.passes[0].observations.borrows.is_empty());
    }
    assert_eq!(callable, 1);
}

#[cfg(test)]
pub(in crate::scalar) fn assert_control_binding_rejection(
    cfg: &ScalarReferenceCfg,
    trace: &ReferenceValidationTestTrace,
    binding: &ScalarBindingCfgPoint,
) {
    assert!(matches!(binding.source, ScalarBindingOutputSource::Conditional { .. } | ScalarBindingOutputSource::Block { .. }));
    let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
    let mut malformed = cfg.clone();
    malformed.evaluated_values.binding_inputs.retain(|row| row.consumer != binding.point);
    let before = malformed.clone();
    let mut negative_trace = ReferenceValidationTestTrace::new();
    let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
        &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
        &pass.context.capture_input,
        pass.role.clone(), &mut negative_trace);
    assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
        source_span: binding.occurrence.source_span.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }]);
    assert_eq!(malformed, before);
    assert_eq!(negative_trace.passes.len(), 1);
    assert_eq!(negative_trace.passes[0].owner, cfg.producer_owner);
    assert!(negative_trace.passes[0].observations.points.is_empty());
    assert!(negative_trace.passes[0].observations.borrows.is_empty());
}

pub(in crate::scalar) fn validate_binding_inputs(
    cfg: &ScalarReferenceCfg,
) -> Result<(), ReferenceAnalysisError> {
    for binding in cfg
        .binding_points
        .iter()
        .filter(|binding| binding.role == ScalarBindingPointRole::Bind)
    {
        let invalid = || ReferenceAnalysisError::MissingContext {
            source_span: binding.occurrence.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        let point = cfg.points.get(binding.point.0).ok_or_else(invalid)?;
        let CfgPointKind::Bind { binding: receiver } = &point.kind else {
            return Err(invalid());
        };
        if receiver.source != binding.occurrence.source_span.source
            || point.source_span.range != receiver.declaration_span
        {
            return Err(invalid());
        }
        if cfg
            .binding_points
            .iter()
            .filter(|known| {
                known.role == ScalarBindingPointRole::Bind
                    && (known.point == binding.point
                        || (known.occurrence == binding.occurrence
                            && known.output == binding.output))
            })
            .count()
            != 1
        {
            return Err(invalid());
        }
        let inputs = cfg
            .evaluated_values
            .binding_inputs
            .iter()
            .filter(|input| input.consumer == binding.point)
            .collect::<Vec<_>>();
        match &binding.source {
            ScalarBindingOutputSource::Direct { .. } => {
                let [input] = inputs.as_slice() else { return Err(invalid()); };
                let source = retained_source_value(cfg, &binding.source, &binding.occurrence.source_span)?;
                if input.producer != cfg.producer_owner || input.occurrence != binding.occurrence
                    || input.output != binding.output || input.source != *source {
                    return Err(invalid());
                }
            }
            ScalarBindingOutputSource::Call { occurrence, output } => {
                let [input] = inputs.as_slice() else {
                    return Err(invalid());
                };
                let productions = cfg
                    .evaluated_values
                    .productions
                    .iter()
                    .filter(|production| {
                        production.producer == cfg.producer_owner
                            && production.occurrence == occurrence.source_span
                            && production.output_projection == occurrence.output_projection
                    })
                    .collect::<Vec<_>>();
                let [production] = productions.as_slice() else {
                    return Err(invalid());
                };
                let OutputProductionSource::Call {
                    occurrence: actual,
                    projections,
                } = &production.source
                else {
                    return Err(invalid());
                };
                let source = production.outputs.get(*output).ok_or_else(invalid)?;
                let calls = cfg
                    .call_points
                    .iter()
                    .filter(|call| call.occurrence == *occurrence)
                    .collect::<Vec<_>>();
                let [call] = calls.as_slice() else {
                    return Err(invalid());
                };
                if actual != occurrence
                    || input.producer != cfg.producer_owner
                    || input.occurrence != binding.occurrence
                    || input.output != binding.output
                    || input.source != *source
                    || source.resolved.output != *output
                    || source.resolved.occurrence != occurrence.source_span
                    || source.resolved.output_projection != occurrence.output_projection
                    || call.evaluated_point != production.evaluation_point
                    || call.projections != *projections
                    || !projections.iter().any(|projection| {
                        projection.output == *output && projection.point == source.point
                    })
                {
                    return Err(invalid());
                }
            }
            ScalarBindingOutputSource::Expression { .. } => {
                if !inputs.is_empty() {
                    return Err(invalid());
                }
            }
            ScalarBindingOutputSource::Conditional { .. } | ScalarBindingOutputSource::Block { .. } => {
                let [input] = inputs.as_slice() else { return Err(invalid()); };
                let source = retained_source_value(cfg, &binding.source, &binding.occurrence.source_span)?;
                if input.producer != cfg.producer_owner || input.occurrence != binding.occurrence
                    || input.output != binding.output || input.source != *source {
                    return Err(invalid());
                }
                let productions = cfg.evaluated_values.productions.iter().filter(|production| {
                    production.producer == cfg.producer_owner && production.outputs.contains(source)
                }).collect::<Vec<_>>();
                let [production] = productions.as_slice() else { return Err(invalid()); };
                let source_point = cfg.points.get(source.point.0).ok_or_else(invalid)?;
                if production.outputs.len() == 1 {
                    if source.point != production.evaluation_point { return Err(invalid()); }
                } else if !matches!(source_point.kind, CfgPointKind::ValueOutputProjection { production: anchor, output }
                    if anchor == production.evaluation_point && output == source.resolved.output) {
                    return Err(invalid());
                }
            }
        }
    }
    for input in &cfg.evaluated_values.binding_inputs {
        let invalid = || ReferenceAnalysisError::MissingContext {
            source_span: input.occurrence.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        if input.producer != cfg.producer_owner
            || input.source.resolved.producer != cfg.producer_owner
        {
            return Err(invalid());
        }
        let points = cfg
            .binding_points
            .iter()
            .filter(|point| {
                point.point == input.consumer
                    && point.role == ScalarBindingPointRole::Bind
                    && point.occurrence == input.occurrence
                    && point.output == input.output
            })
            .collect::<Vec<_>>();
        if points.len() != 1
            || cfg
                .evaluated_values
                .binding_inputs
                .iter()
                .filter(|known| known.consumer == input.consumer)
                .count()
                != 1
        {
            return Err(invalid());
        }
        let point = cfg.points.get(input.consumer.0).ok_or_else(invalid)?;
        let CfgPointKind::Bind { binding } = &point.kind else {
            return Err(invalid());
        };
        if binding.source != input.occurrence.source_span.source
            || point.source_span.range != binding.declaration_span
        {
            return Err(invalid());
        }
        binding_disposition(&cfg.evaluated_values, input, binding)?;
        if !cfg
            .evaluated_values
            .values
            .iter()
            .any(|fact| fact.id == input.source)
            || (!matches!(points[0].source, ScalarBindingOutputSource::Direct { .. })
                && !cfg.evaluated_values.productions.iter().any(|production| {
                production.producer == input.producer
                    && production.outputs.contains(&input.source)
                    && matches!(production.source, OutputProductionSource::Call { .. }
                        | OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })
            }))
        {
            return Err(invalid());
        }
        let source_point = cfg.points.get(input.source.point.0).ok_or_else(invalid)?;
        if matches!(points[0].source, ScalarBindingOutputSource::Call { .. }) {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { occurrence, .. },
            output,
            ..
        } = &source_point.kind
        else {
            return Err(invalid());
        };
        if input.source.resolved.occurrence != occurrence.source_span
            || input.source.resolved.output_projection != occurrence.output_projection
            || input.source.resolved.output != *output
            || !cfg.call_points.iter().any(|call| {
                call.occurrence == **occurrence
                    && call.projections.iter().any(|projection| {
                        projection.point == input.source.point && projection.output == *output
                    })
            })
        {
            return Err(invalid());
        }
        }
        if cfg.checked_bindings.contains_key(binding) && !cfg.checked_installs.iter().any(|install| {
            install.point == input.consumer && matches!(install.input,
                ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == input.source.point)
        }) {
            return Err(invalid());
        }
    }
    for disposition in &cfg.evaluated_values.disposition_inputs {
        if let ValueConsumerKind::Binding { binding, projection } = &disposition.consumer.kind {
            let invalid = || ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(binding.source.clone(), binding.declaration_span),
                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            };
            let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input|
                input.consumer == disposition.consumer.point && input.producer == disposition.consumer.producer).collect::<Vec<_>>();
            let [input] = inputs.as_slice() else { return Err(invalid()); };
            if disposition.consumer.producer != cfg.producer_owner || !projection.is_empty()
                || !matches!(&cfg.points.get(input.consumer.0).ok_or_else(invalid)?.kind,
                    CfgPointKind::Bind { binding: actual } if actual == binding) {
                return Err(invalid());
            }
            binding_disposition(&cfg.evaluated_values, input, binding)?;
        }
    }
    Ok(())
}

pub(super) fn binding_source_points(
    cfg: &ScalarReferenceCfg,
    consumer: CfgPointId,
) -> Vec<&ReferenceFlowPoint> {
    match binding_input(cfg, consumer) {
        Some(input) => vec![&cfg.points[input.source.point.0]],
        None => cfg
            .points
            .iter()
            .filter(|point| point.successors.contains(&consumer))
            .collect(),
    }
}

pub(super) fn binding_input(
    cfg: &ScalarReferenceCfg,
    consumer: CfgPointId,
) -> Option<&BindingOutputInput> {
    let binding = cfg.binding_points.iter().find(|binding| {
        binding.role == ScalarBindingPointRole::Bind && binding.point == consumer
    })?;
    match &binding.source {
        ScalarBindingOutputSource::Direct { .. } | ScalarBindingOutputSource::Call { .. } | ScalarBindingOutputSource::Conditional { .. }
        | ScalarBindingOutputSource::Block { .. } => Some(
            cfg.evaluated_values
                .binding_inputs
                .iter()
                .find(|input| input.consumer == consumer)
                .expect("validated actual Call binding input"),
        ),
        ScalarBindingOutputSource::Expression { .. } => None,
    }
}

impl ReferenceFlowBuilder {
    pub(super) fn binding_output_source(
        &self,
        binding: &ScalarBinding,
        output: usize,
    ) -> Result<ScalarBindingOutputSource, ReferenceAnalysisError> {
        let invalid = || self.missing_context(binding.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
        if output >= binding.receivers.len() { return Err(invalid()); }
        let expression = match binding.output_origin {
            ScalarBindingOutputOrigin::SingleExpression => &binding.value,
            ScalarBindingOutputOrigin::IndependentExpressions => {
                if binding.output_values.len() != binding.receivers.len() { return Err(invalid()); }
                &binding.output_values.get(output).ok_or_else(invalid)?.value
            }
        };
        Ok(match expression {
            ScalarExpression::Call { occurrence, .. } => {
                let facts = self.value_types.expressions.iter().filter(|fact| fact.source_span == occurrence.source_span).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { return Err(invalid()); };
                let source_outputs = &fact.outputs.outputs;
                let source_output = match binding.output_origin {
                    ScalarBindingOutputOrigin::SingleExpression if source_outputs.len() > 1 => {
                        output
                    }
                    ScalarBindingOutputOrigin::SingleExpression
                    | ScalarBindingOutputOrigin::IndependentExpressions => 0,
                };
                if source_output >= source_outputs.len() { return Err(invalid()); }
                ScalarBindingOutputSource::Call {
                    occurrence: (**occurrence).clone(),
                    output: source_output,
                }
            }
            ScalarExpression::If { then_branch, span, .. } => {
                let mut then_block = then_branch.occurrence.as_ref();
                loop {
                    let aliases = self.cfg.evaluated_values.block_output_aliases.iter()
                        .filter(|alias| alias.selected == *then_block).collect::<Vec<_>>();
                    match aliases.as_slice() {
                        [] => break,
                        [alias] if alias.selected.source_span == alias.source.source_span
                            && alias.selected.output_projection.len() == alias.source.output_projection.len() + 1
                            && alias.selected.output_projection[1..] == alias.source.output_projection => then_block = &alias.source,
                        _ => return Err(invalid()),
                    }
                }
                let occurrence = SourceSpan::new(self.source.clone(), *span);
                let facts = self.value_types.expressions.iter().filter(|fact| fact.source_span == occurrence).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { return Err(invalid()); };
                let source_output = match binding.output_origin {
                    ScalarBindingOutputOrigin::SingleExpression if fact.outputs.outputs.len() > 1 => output,
                    ScalarBindingOutputOrigin::SingleExpression | ScalarBindingOutputOrigin::IndependentExpressions => 0,
                };
                if source_output >= fact.outputs.outputs.len() { return Err(invalid()); }
                ScalarBindingOutputSource::Conditional { occurrence, then_block: then_block.clone(), output: source_output }
            }
            ScalarExpression::Block(block) => {
                let mut occurrence = block.occurrence.as_ref();
                loop {
                    let aliases = self.cfg.evaluated_values.block_output_aliases.iter()
                        .filter(|alias| alias.selected == *occurrence).collect::<Vec<_>>();
                    match aliases.as_slice() {
                        [] => break,
                        [alias] if alias.selected.source_span == alias.source.source_span
                            && alias.selected.output_projection.len() == alias.source.output_projection.len() + 1
                            && alias.selected.output_projection[1..] == alias.source.output_projection => occurrence = &alias.source,
                        _ => return Err(invalid()),
                    }
                }
                let facts = self.value_types.expressions.iter().filter(|fact| fact.source_span == block.occurrence.source_span).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { return Err(invalid()); };
                let source_output = match binding.output_origin {
                    ScalarBindingOutputOrigin::SingleExpression if fact.outputs.outputs.len() > 1 => output,
                    ScalarBindingOutputOrigin::SingleExpression | ScalarBindingOutputOrigin::IndependentExpressions => 0,
                };
                if source_output >= fact.outputs.outputs.len() { return Err(invalid()); }
                ScalarBindingOutputSource::Block { occurrence: occurrence.clone(), output: source_output }
            }
            ScalarExpression::Assignment(_) | ScalarExpression::UnitIf { .. } => ScalarBindingOutputSource::Expression {
                occurrence: SourceSpan::new(self.source.clone(), expression_span(expression)),
            },
            _ if binding.is_allocation => ScalarBindingOutputSource::Expression {
                occurrence: SourceSpan::new(self.source.clone(), expression_span(expression)),
            },
            _ => {
                let occurrence = SourceSpan::new(self.source.clone(), expression_span(expression));
                let facts = self.value_types.expressions.iter().filter(|fact| fact.source_span == occurrence).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { return Err(invalid()); };
                let source_output = match binding.output_origin {
                    ScalarBindingOutputOrigin::SingleExpression if fact.outputs.outputs.len() > 1 => output,
                    ScalarBindingOutputOrigin::SingleExpression | ScalarBindingOutputOrigin::IndependentExpressions => 0,
                };
                let ty = self.value_types.selected_output(&occurrence, source_output).map_err(|_| invalid())?;
                let output_projection = match expression {
                    ScalarExpression::Dereference { occurrence, .. } | ScalarExpression::IndexedRead { occurrence, .. } => occurrence.output_projection.clone(),
                    _ => Vec::new(),
                };
                let retained = ScalarBindingOutputSource::Direct { occurrence, output_projection, output: source_output };
                let value = retained_source_value(&self.cfg, &retained, &binding.occurrence.source_span)?;
                let value_fact = self.cfg.evaluated_values.values.iter().find(|fact| fact.id == *value).ok_or_else(invalid)?;
                let shape = self.cfg.evaluated_values.shapes.iter().find(|shape| shape.id == value_fact.shape).ok_or_else(invalid)?;
                if !scalar_type_equal(&shape.ty, ty) { return Err(invalid()); }
                retained
            }
        })
    }

    pub(super) fn binding_source_point(
        &mut self,
        binding: &ScalarBinding,
        output: usize,
        point: CfgPointId,
    ) -> Result<(), ReferenceAnalysisError> {
        let retained = self.binding_output_source(binding, output)?;
        let source = match binding_source_key(&retained) {
            Some(_) => Some(retained_source_value(&self.cfg, &retained, &binding.occurrence.source_span)?.clone()),
            None => None,
        };
        let disposition = if let Some(source) = &source {
            let invalid = || self.missing_context(binding.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
            let ty = self.value_types.selected_output(&source.resolved.occurrence, source.resolved.output).map_err(|_| invalid())?;
            let fact = self.cfg.evaluated_values.values.iter().find(|fact| fact.id == *source).ok_or_else(invalid)?;
            let shape = self.cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).ok_or_else(invalid)?;
            let CfgPointKind::Bind { binding: receiver } = &self.cfg.points.get(point.0).ok_or_else(invalid)?.kind else { return Err(invalid()); };
            if !scalar_type_equal(&shape.ty, ty) || receiver.declaration_span != binding.receivers[output].name_span { return Err(invalid()); }
            let policy = copy_policy(ty, &self.structs, &self.enums, &self.generic_parameters)
                .map_err(|_| self.missing_context(binding.span, ReferenceAnalysisErrorReason::MissingResolvedBinding))?;
            let consumer = ValueConsumerId {
                producer: self.cfg.producer_owner.clone(), point,
                kind: ValueConsumerKind::Binding { binding: receiver.clone(), projection: Vec::new() },
            };
            if self.cfg.evaluated_values.disposition_inputs.iter().any(|row| row.consumer == consumer) { return Err(invalid()); }
            Some(ResolvedValueDispositionInput { source: source.clone(), consumer, policy })
        } else { None };
        if self.cfg.binding_points.iter().any(|known| known.role == ScalarBindingPointRole::Bind
            && (known.point == point || (known.occurrence == *binding.occurrence && known.output == output)))
            || self.cfg.evaluated_values.binding_inputs.iter().any(|known| known.consumer == point) {
            return Err(self.missing_context(binding.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
        }
        self.cfg.binding_points.push(ScalarBindingCfgPoint {
            occurrence: binding.occurrence.as_ref().clone(), output,
            role: ScalarBindingPointRole::Bind, point, source: retained,
        });
        if let Some(source) = source {
            let input = BindingOutputInput {
                producer: self.cfg.producer_owner.clone(),
                consumer: point,
                occurrence: (*binding.occurrence).clone(),
                output,
                source,
            };
            self.cfg.evaluated_values.binding_inputs.push(input);
        }
        if let Some(disposition) = disposition { self.cfg.evaluated_values.disposition_inputs.push(disposition); }
        Ok(())
    }

    pub(super) fn validate_call_receivers(
        &self,
        binding: &ScalarBinding,
    ) -> Result<(), ReferenceAnalysisError> {
        for (output, receiver) in binding.receivers.iter().enumerate() {
            let retained = self.binding_output_source(binding, output)?;
            if matches!(retained, ScalarBindingOutputSource::Expression { .. }) {
                continue;
            }
            let positions = self
                .cfg
                .binding_points
                .iter()
                .filter(|point| {
                    point.occurrence == *binding.occurrence
                        && point.output == output
                        && point.role == ScalarBindingPointRole::Bind
                })
                .collect::<Vec<_>>();
            let [point] = positions.as_slice() else {
                return Err(self.missing_context(
                    binding.span,
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            };
            let inputs = self
                .cfg
                .evaluated_values
                .binding_inputs
                .iter()
                .filter(|input| input.consumer == point.point)
                .collect::<Vec<_>>();
            let [input] = inputs.as_slice() else {
                return Err(self.missing_context(
                    binding.span,
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            };
            let expected = retained_source_value(&self.cfg, &retained, &binding.occurrence.source_span)?.clone();
            if input.source != expected
                || input.producer != self.cfg.producer_owner
                || input.occurrence != *binding.occurrence
                || input.output != output
                || point.source != retained
                || !matches!(&self.cfg.points[point.point.0].kind,
                CfgPointKind::Bind { binding } if binding.declaration_span == receiver.name_span
                    && binding.source == self.source && binding.function_span == self.function_span)
            {
                return Err(self.missing_context(
                    binding.span,
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn complete_call_outputs(
        &mut self,
        expression: &ScalarExpression,
        anchor: CfgPointId,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        let source_span = SourceSpan::new(self.source.clone(), expression_span(expression));
        let source_outputs = self
            .value_types
            .expressions
            .iter()
            .find(|fact| fact.source_span == source_span)
            .ok_or_else(|| {
                self.missing_context(
                    source_span.range,
                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                )
            })?
            .outputs
            .clone();
        let CfgPointKind::Call {
            target,
            argument_points,
            output: 0,
            ..
        } = self.cfg.points[anchor.0].kind.clone()
        else {
            return Err(self.missing_context(
                source_span.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        };
        for output in 1..source_outputs.outputs.len() {
            self.point_with_origins(
                CfgPointKind::Call {
                    expression: expression.clone(),
                    output,
                    target: target.clone(),
                    argument_points: argument_points.clone(),
                },
                source_span.range,
                scope,
                None,
            );
        }
        let sequence = self.bind_final_output_values(expression, anchor)?;
        let ScalarExpression::Call { occurrence, .. } = expression else {
            unreachable!()
        };
        let call = self
            .cfg
            .call_points
            .iter()
            .find(|call| call.occurrence == **occurrence)
            .expect("canonical call output correspondence");
        let production = EvaluatedOutputProduction {
            producer: self.cfg.producer_owner.clone(),
            occurrence: source_span.clone(),
            output_projection: occurrence.output_projection.clone(),
            evaluation_point: anchor,
            outputs: sequence
                .entries
                .into_iter()
                .map(|entry| entry.value)
                .collect(),
            source: OutputProductionSource::Call {
                occurrence: (**occurrence).clone(),
                projections: call.projections.clone(),
            },
        };
        if let Some(known) = self.cfg.evaluated_values.productions.iter().find(|known| {
            known.producer == production.producer
                && known.occurrence == production.occurrence
                && known.output_projection == production.output_projection
        }) {
            if known != &production {
                return Err(self.missing_context(
                    source_span.range,
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
        } else {
            self.cfg.evaluated_values.productions.push(production);
        }
        Ok(())
    }

    pub(in crate::scalar) fn call_output_sequence(
        &self,
        expression: &ScalarExpression,
    ) -> Result<assignment::EvaluatedOutputSequence, ReferenceAnalysisError> {
        let ScalarExpression::Call { occurrence, .. } = expression else {
            unreachable!()
        };
        let production = self
            .cfg
            .evaluated_values
            .productions
            .iter()
            .find(|production| {
                production.producer == self.cfg.producer_owner
                    && production.occurrence == occurrence.source_span
                    && production.output_projection == occurrence.output_projection
            })
            .ok_or_else(|| {
                self.missing_context(
                    expression_span(expression),
                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                )
            })?;
        let entries = production
            .outputs
            .iter()
            .map(|value| {
                let fact = self
                    .cfg
                    .evaluated_values
                    .values
                    .iter()
                    .find(|fact| fact.id == *value)
                    .ok_or_else(|| {
                        self.missing_context(
                            expression_span(expression),
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                Ok(assignment::EvaluatedOutputEntry {
                    value: value.clone(),
                    shape: fact.shape.clone(),
                })
            })
            .collect::<Result<Vec<_>, ReferenceAnalysisError>>()?;
        Ok(assignment::EvaluatedOutputSequence { entries })
    }

    pub(super) fn expression_value_point(
        &self,
        expression: &ScalarExpression,
        output: usize,
    ) -> Result<CfgPointId, ReferenceAnalysisError> {
        if matches!(expression, ScalarExpression::If { .. } | ScalarExpression::Block(_)) {
            self.control_output_sequence(expression)?.entries.get(output).map(|entry| entry.value.point)
                .ok_or_else(|| self.missing_context(expression_span(expression), ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence))
        } else if matches!(expression, ScalarExpression::Call { .. }) {
            self.call_output_sequence(expression)?
                .entries
                .get(output)
                .map(|entry| entry.value.point)
                .ok_or_else(|| {
                    self.missing_context(
                        expression_span(expression),
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    )
                })
        } else if output == 0 {
            Ok(self.cursor.expect("evaluated source output"))
        } else {
            Err(self.missing_context(
                expression_span(expression),
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ))
        }
    }
}
