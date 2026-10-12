use super::*;

fn control_path_contains(
    cfg: &ScalarReferenceCfg,
    from: CfgPointId,
    to: CfgPointId,
    boundary: CfgPointId,
) -> bool {
    let mut pending = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(point) = pending.pop() {
        if point == to { return true; }
        if point == boundary || !seen.insert(point) { continue; }
        let Some(point) = cfg.points.get(point.0) else { return false; };
        pending.extend(point.successors.iter().copied());
    }
    false
}

fn validate_control_anchor(
    cfg: &ScalarReferenceCfg,
    occurrence: &SourceSpan,
    output_projection: &[usize],
    evaluation_point: CfgPointId,
    source: &OutputProductionSource,
) -> Result<(), ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: occurrence.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    let anchor = cfg.points.get(evaluation_point.0).ok_or_else(invalid)?;
    if anchor.id != evaluation_point || anchor.source_span != *occurrence {
        return Err(invalid());
    }
    let predecessors = cfg.points.iter().filter(|point| point.successors.contains(&evaluation_point))
        .collect::<Vec<_>>();
    match source {
        OutputProductionSource::Conditional { branch, inputs } => {
            let branches = cfg.conditional_points.iter().filter(|event| {
                event.point == *branch && event.role == ScalarConditionalPointRole::Branch
            }).collect::<Vec<_>>();
            let [registered] = branches.as_slice() else { return Err(invalid()); };
            let branch_point = cfg.points.get(branch.0).ok_or_else(invalid)?;
            if branch_point.id != *branch || !matches!(branch_point.kind, CfgPointKind::Branch { .. })
                || !matches!(anchor.kind, CfgPointKind::Join)
                || registered.then_block.output_projection != output_projection
                || registered.then_block.source_span.source != occurrence.source
                || inputs.len() != 2 || predecessors.len() != 2
                || branch_point.successors.len() != 2
                || inputs[0].predecessor == inputs[1].predecessor {
                return Err(invalid());
            }
            for (position, (input, role)) in inputs.iter().zip([
                ScalarConditionalPointRole::ThenScopeExit,
                ScalarConditionalPointRole::ElseScopeExit,
            ]).enumerate() {
                let events = cfg.conditional_points.iter().filter(|event| {
                    event.then_block == registered.then_block && event.role == role
                }).collect::<Vec<_>>();
                let [event] = events.as_slice() else { return Err(invalid()); };
                let predecessor = cfg.points.get(input.predecessor.0).ok_or_else(invalid)?;
                if event.point != input.predecessor || predecessor.id != input.predecessor
                    || !matches!(predecessor.kind, CfgPointKind::ScopeExit)
                    || predecessor.successors.iter().filter(|point| **point == evaluation_point).count() != 1
                    || !predecessors.iter().any(|point| point.id == input.predecessor) {
                    return Err(invalid());
                }
                let entry = branch_point.successors[position];
                let other_exit = inputs[1 - position].predecessor;
                if !control_path_contains(cfg, entry, input.predecessor, other_exit)
                    || input.outputs.windows(2).any(|pair| pair[0].point >= pair[1].point)
                    || input.outputs.iter().any(|value| {
                        !control_path_contains(cfg, entry, value.point, evaluation_point)
                            || !control_path_contains(cfg, value.point, input.predecessor, evaluation_point)
                    }) {
                    return Err(invalid());
                }
            }
        }
        OutputProductionSource::Block { occurrence: block, scope_exit, inputs } => {
            let events = cfg.block_scope_exits.iter().filter(|event| event.occurrence == *block)
                .collect::<Vec<_>>();
            let [event] = events.as_slice() else { return Err(invalid()); };
            let exit = cfg.points.get(scope_exit.0).ok_or_else(invalid)?;
            if block.source_span != *occurrence || block.output_projection != output_projection
                || event.point != *scope_exit || exit.id != *scope_exit
                || !matches!(exit.kind, CfgPointKind::ScopeExit)
                || exit.successors.iter().filter(|point| **point == evaluation_point).count() != 1
                || !matches!(predecessors.as_slice(), [point] if point.id == *scope_exit)
                || !matches!(&anchor.kind, CfgPointKind::Evaluate(ScalarExpression::Block(actual))
                    if *actual.occurrence == *block) {
                return Err(invalid());
            }
            if inputs.windows(2).any(|pair| pair[0].point >= pair[1].point)
                || inputs.iter().any(|value| !control_path_contains(cfg, value.point, *scope_exit, evaluation_point)) {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

pub(in crate::scalar) fn edge_outputs(
    tables: &EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    from: CfgPointId,
    to: CfgPointId,
) -> Result<Vec<(EvaluatedValueId, EvaluatedValueId)>, ReferenceAnalysisError> {
    let mut outputs = Vec::new();
    for production in tables.productions.iter().filter(|production| production.producer == *owner) {
        let invalid = |_site| {
            let error = ReferenceAnalysisError::MissingContext {
                source_span: production.occurrence.clone(),
                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            };
            #[cfg(test)]
            selected_source::snapshot23_observation::constructed_site(2, _site, &error);
            error
        };
        let inputs = match &production.source {
            OutputProductionSource::Conditional { inputs, .. } if production.evaluation_point == to => {
                let selected = inputs.iter().filter(|input| input.predecessor == from).collect::<Vec<_>>();
                match selected.as_slice() {
                    [] => None,
                    [input] => Some(input.outputs.as_slice()),
                    _ => return Err(invalid(0usize)),
                }
            }
            OutputProductionSource::Block { scope_exit, inputs, .. }
                if *scope_exit == from && production.evaluation_point == to => Some(inputs.as_slice()),
            _ => None,
        };
        if let Some(inputs) = inputs {
            if inputs.is_empty() && production.outputs.len() == 1 {
                let facts = tables.values.iter().filter(|fact| fact.id == production.outputs[0]).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { return Err(invalid(1usize)); };
                let shapes = tables.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
                if !matches!(shapes.as_slice(), [shape] if shape.ty == ScalarType::Unit) {
                    return Err(invalid(2usize));
                }
                continue;
            }
            if inputs.len() != production.outputs.len() { return Err(invalid(3usize)); }
            for (output, destination) in production.outputs.iter().enumerate() {
                let source = &inputs[output];
                if source.resolved.producer != *owner || destination.resolved.producer != *owner
                    || destination.resolved.output != output
                    || destination.resolved.occurrence != production.occurrence
                    || destination.resolved.output_projection != production.output_projection
                    || tables.values.iter().filter(|fact| fact.id == *source).count() != 1
                    || tables.values.iter().filter(|fact| fact.id == *destination).count() != 1
                    || outputs.iter().any(|(_, known)| known == destination) {
                    return Err(invalid(4usize));
                }
                outputs.push((source.clone(), destination.clone()));
            }
        }
    }
    Ok(outputs)
}

pub(super) fn transport_edge_values(
    cfg: &ScalarReferenceCfg,
    state: &mut ReferenceEdgeState,
    from: CfgPointId,
    to: CfgPointId,
) -> Result<(), ReferenceAnalysisError> {
    let outputs = edge_outputs(&cfg.evaluated_values, &cfg.producer_owner, from, to)?;
    for (source, destination) in outputs {
        state.point_values[destination.point.0] = state.point_values[source.point.0].clone();
        state.callable_point_values[destination.point.0] = state.callable_point_values[source.point.0].clone();
        state.callable_point_projected[destination.point.0] = state.callable_point_projected[source.point.0].clone();
        let sources = state.holder_point_sources[source.point.0].iter().map(|holder| match holder {
            CallableHolderPlace::Temporary(_) => CallableHolderPlace::Temporary(destination.point),
            holder => holder.clone(),
        }).collect::<Vec<_>>();
        state.holder_point_values[destination.point.0] = state.holder_relations.iter().filter(|relation| {
            relation.holders.iter().any(|holder| sources.contains(holder))
        }).cloned().collect();
        state.holder_point_sources[destination.point.0] = sources;
        if let Some(backings) = state.backing_point_values.get(&source.point).cloned() {
            state.backing_point_values.insert(destination.point, backings);
        } else {
            state.backing_point_values.remove(&destination.point);
        }
    }
    Ok(())
}

pub(super) fn forward_exit_holders(
    tables: &EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    successors: &[CfgPointId],
    state: &mut ReferenceEdgeState,
    exit: CfgPointId,
    actions: &mut Vec<CallableHolderAction>,
) -> Result<Vec<CallableHolderPlace>, ReferenceAnalysisError> {
    let mut preserved = Vec::new();
    for successor in successors {
        for (source, destination) in edge_outputs(tables, owner, exit, *successor)? {
            for holder in state.holder_point_sources[source.point.0].clone() {
                let relations = state.holder_relations.iter().filter(|relation| relation.holders.contains(&holder)).collect::<Vec<_>>();
                if relations.is_empty() || relations.iter().any(|relation| state.holder_provenance.lookup(&relation.environment, &holder).is_none()) {
                    return Err(ReferenceAnalysisError::MissingContext {
                        source_span: destination.resolved.occurrence.clone(),
                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    });
                }
                let target = match holder {
                    CallableHolderPlace::Temporary(_) => CallableHolderPlace::Temporary(destination.point),
                    _ => holder.clone(),
                };
                if holder != target {
                    transfer_holder(&mut state.holder_relations, &holder, target.clone(), true,
                        &destination.resolved.occurrence, actions, exit, &mut state.holder_provenance);
                }
                if !preserved.contains(&target) { preserved.push(target); }
            }
        }
    }
    Ok(preserved)
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    for production in cfg.evaluated_values.productions.iter().filter(|production| {
        production.producer == cfg.producer_owner
            && matches!(production.source, OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })
    }) {
        let invalid = || ReferenceAnalysisError::MissingContext {
            source_span: production.occurrence.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        if production.producer != cfg.producer_owner
            || cfg.evaluated_values.productions.iter().filter(|known| {
                known.producer == production.producer && known.occurrence == production.occurrence
                    && known.output_projection == production.output_projection
            }).count() != 1 {
            return Err(invalid());
        }
        validate_control_anchor(cfg, &production.occurrence, &production.output_projection,
            production.evaluation_point, &production.source)?;
        let anchor = cfg.points.get(production.evaluation_point.0).ok_or_else(invalid)?;
        let inputs = match &production.source {
            OutputProductionSource::Conditional { branch, inputs } => {
                let branch = cfg.points.get(branch.0).ok_or_else(invalid)?;
                if !matches!(anchor.kind, CfgPointKind::Join)
                    || !matches!(branch.kind, CfgPointKind::Branch { .. })
                    || inputs.len() != 2
                    || inputs[0].predecessor == inputs[1].predecessor {
                    return Err(invalid());
                }
                for input in inputs {
                    let predecessor = cfg.points.get(input.predecessor.0).ok_or_else(invalid)?;
                    if !matches!(predecessor.kind, CfgPointKind::ScopeExit)
                        || !predecessor.successors.contains(&anchor.id) {
                        return Err(invalid());
                    }
                }
                inputs.iter().map(|input| input.outputs.as_slice()).collect::<Vec<_>>()
            }
            OutputProductionSource::Block { occurrence, scope_exit, inputs } => {
                let exit = cfg.points.get(scope_exit.0).ok_or_else(invalid)?;
                if !matches!(&anchor.kind, CfgPointKind::Evaluate(ScalarExpression::Block(block)) if *block.occurrence == *occurrence)
                    || !matches!(exit.kind, CfgPointKind::ScopeExit)
                    || !exit.successors.contains(&anchor.id)
                    || !cfg.block_scope_exits.iter().any(|event| event.occurrence == *occurrence && event.point == exit.id)
                    || occurrence.source_span != production.occurrence
                    || occurrence.output_projection != production.output_projection {
                    return Err(invalid());
                }
                vec![inputs.as_slice()]
            }
            _ => unreachable!(),
        };
        for tuple in &inputs {
            if tuple.len() != production.outputs.len() {
                if !tuple.is_empty() || production.outputs.len() != 1 {
                    return Err(invalid());
                }
                let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == production.outputs[0]).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { return Err(invalid()); };
                let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
                if !matches!(shapes.as_slice(), [shape] if shape.ty == ScalarType::Unit) {
                    return Err(invalid());
                }
            }
        }
        for (output, value) in production.outputs.iter().enumerate() {
            let point = cfg.points.get(value.point.0).ok_or_else(invalid)?;
            if value.resolved.producer != cfg.producer_owner
                || value.resolved.occurrence != production.occurrence
                || value.resolved.output_projection != production.output_projection
                || value.resolved.output != output
                || point.source_span != production.occurrence
                || if production.outputs.len() == 1 { point.id != anchor.id } else {
                    !matches!(point.kind, CfgPointKind::ValueOutputProjection { production, output: actual } if production == anchor.id && actual == output)
                } {
                return Err(invalid());
            }
            let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *value).collect::<Vec<_>>();
            let [fact] = facts.as_slice() else { return Err(invalid()); };
            let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else { return Err(invalid()); };
            if shape.id.producer != cfg.producer_owner { return Err(invalid()); }
            for tuple in &inputs {
                if tuple.is_empty() && production.outputs.len() == 1 && shape.ty == ScalarType::Unit { continue; }
                if tuple.len() != production.outputs.len() { return Err(invalid()); }
                let source = &tuple[output];
                let source_facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *source).collect::<Vec<_>>();
                let [source_fact] = source_facts.as_slice() else { return Err(invalid()); };
                let source_shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == source_fact.shape).collect::<Vec<_>>();
                let [source_shape] = source_shapes.as_slice() else { return Err(invalid()); };
                if source.resolved.producer != cfg.producer_owner || !scalar_type_equal(&source_shape.ty, &shape.ty) { return Err(invalid()); }
            }
        }
    }
    for point in &cfg.points {
        if let CfgPointKind::ValueOutputProjection { production: anchor, output } = &point.kind {
            let productions = cfg.evaluated_values.productions.iter().filter(|production| {
                production.producer == cfg.producer_owner && production.evaluation_point == *anchor
                    && production.outputs.get(*output).map(|value| value.point) == Some(point.id)
            }).collect::<Vec<_>>();
            if productions.len() != 1 {
                return Err(ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                });
            }
        }
    }
    for alias in &cfg.evaluated_values.block_output_aliases {
        let invalid = || ReferenceAnalysisError::MissingContext {
            source_span: alias.selected.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        if alias.selected.source_span != alias.source.source_span
            || alias.selected.output_projection.len() != alias.source.output_projection.len() + 1
            || alias.selected.output_projection[1..] != alias.source.output_projection
            || cfg.evaluated_values.block_output_aliases.iter().filter(|known| known.selected == alias.selected).count() != 1 {
            return Err(invalid());
        }
        let mut source = &alias.source;
        loop {
            let next = cfg.evaluated_values.block_output_aliases.iter()
                .filter(|known| known.selected == *source).collect::<Vec<_>>();
            match next.as_slice() {
                [] => break,
                [next] if next.selected.source_span == next.source.source_span
                    && next.selected.output_projection.len() == next.source.output_projection.len() + 1
                    && next.selected.output_projection[1..] == next.source.output_projection => source = &next.source,
                _ => return Err(invalid()),
            }
        }
        let events = cfg.block_scope_exits.iter().filter(|event| event.occurrence == *source).collect::<Vec<_>>();
        let [event] = events.as_slice() else { return Err(invalid()); };
        let point = cfg.points.get(event.point.0).ok_or_else(invalid)?;
        if point.source_span != source.source_span || !matches!(point.kind, CfgPointKind::ScopeExit) {
            return Err(invalid());
        }
    }
    Ok(())
}

impl ReferenceFlowBuilder {
    pub(super) fn grouped_output_block(
        &mut self,
        block: &ScalarBlock,
    ) -> Result<Option<ScalarBlock>, ReferenceAnalysisError> {
        let Some(group) = group_conditional_final_outputs(&self.source, block)? else {
            return Ok(None);
        };
        for correspondence in &group.blocks {
            let known = self.cfg.evaluated_values.block_output_aliases.iter()
                .filter(|known| known.selected == correspondence.selected).collect::<Vec<_>>();
            if known.len() > 1 || known.iter().any(|known| known.source != correspondence.source)
                || group.blocks.iter().filter(|other| other.selected == correspondence.selected).count() != 1 {
                return Err(self.missing_context(block.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
            }
        }
        for correspondence in group.blocks {
            let alias = BlockOutputAlias {
                selected: correspondence.selected,
                source: correspondence.source,
            };
            if let Some(known) = self.cfg.evaluated_values.block_output_aliases.iter()
                .find(|known| known.selected == alias.selected) {
                if known != &alias {
                    return Err(self.missing_context(block.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
                }
            } else {
                self.cfg.evaluated_values.block_output_aliases.push(alias);
            }
        }
        let mut source = block.clone();
        let start = source.items.len() - source.final_output_values.len();
        source.items.truncate(start);
        source.terminated_items.truncate(start);
        source.items.push(ScalarBlockItem::Expression(group.expression.clone()));
        source.terminated_items.push(false);
        let mut output = source.final_output_values[0].clone();
        output.value = group.expression;
        source.final_output_values = vec![output];
        source.expressions = source.items.iter().filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        }).collect();
        Ok(Some(source))
    }

    pub(super) fn complete_control_outputs(
        &mut self,
        occurrence: SourceSpan,
        output_projection: Vec<usize>,
        evaluation_point: CfgPointId,
        scope: ReferenceScopeId,
        source: OutputProductionSource,
    ) -> Result<(), ReferenceAnalysisError> {
        if self.cfg.evaluated_values.productions.iter().any(|known| {
            known.producer == self.cfg.producer_owner && known.occurrence == occurrence
                && known.output_projection == output_projection
        }) || self.cfg.evaluated_values.values.iter().any(|known| {
            known.id.resolved.producer == self.cfg.producer_owner
                && known.id.resolved.occurrence == occurrence
                && known.id.resolved.output_projection == output_projection
        }) {
            return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
        }
        validate_control_anchor(&self.cfg, &occurrence, &output_projection, evaluation_point, &source)?;
        let inputs = match &source {
            OutputProductionSource::Conditional { inputs, .. } => inputs
                .iter()
                .map(|input| input.outputs.as_slice())
                .collect::<Vec<_>>(),
            OutputProductionSource::Block { inputs, .. } => vec![inputs.as_slice()],
            _ => unreachable!(),
        };
        let resolved = self.value_types.expressions.iter().filter(|fact| {
            fact.source_span == occurrence
        }).collect::<Vec<_>>();
        let [resolved] = resolved.as_slice() else {
            return Err(self.missing_context(
                occurrence.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        };
        let arity = resolved.outputs.outputs.len();
        let empty_unit = resolved.outputs.outputs.as_slice().iter().all(|output| output.ty == ScalarType::Unit)
            && arity == 1 && inputs.iter().all(|input| input.is_empty());
        if !empty_unit && inputs.iter().any(|input| input.len() != arity) {
            return Err(self.missing_context(
                occurrence.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        }
        for input in &inputs {
            for (output, value) in input.iter().enumerate() {
                let facts = self.cfg.evaluated_values.values.iter().filter(|fact| fact.id == *value).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else {
                    return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
                };
                let shapes = self.cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
                let [shape] = shapes.as_slice() else {
                    return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
                };
                let point = self.cfg.points.get(value.point.0)
                    .ok_or_else(|| self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence))?;
                if value.resolved.producer != self.cfg.producer_owner || shape.id.producer != self.cfg.producer_owner
                    || point.id != value.point || point.source_span != value.resolved.occurrence
                    || input.iter().filter(|known| **known == *value).count() != 1 {
                    return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
                }
                let ty = self.value_types.selected_output(&occurrence, output)
                    .map_err(|source_span| ReferenceAnalysisError::MissingContext {
                        source_span,
                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    })?;
                if !scalar_type_equal(&shape.ty, ty) {
                    return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
                }
            }
        }
        let mut staged_shapes = self.cfg.evaluated_values.shapes.clone();
        let mut output_shapes = Vec::new();
        for output in 0..arity {
            let ty = self.value_types.selected_output(&occurrence, output)
                .map_err(|source_span| ReferenceAnalysisError::MissingContext {
                    source_span,
                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                })?;
            output_shapes.push(intern_value_shape(
                &mut staged_shapes, &self.cfg.producer_owner, ty, &self.structs, &self.enums,
                &self.generic_scope, &self.generic_declarations,
            ).map_err(|_| self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::MissingResolvedBinding))?);
        }
        self.cfg.evaluated_values.shapes = staged_shapes;
        let mut outputs = Vec::new();
        for output in 0..arity {
            let point = if arity == 1 {
                evaluation_point
            } else {
                self.point_with_origins(
                    CfgPointKind::ValueOutputProjection { production: evaluation_point, output },
                    occurrence.range,
                    scope,
                    None,
                )
            };
            let value = bind_evaluated_value_id(&self.cfg, ResolvedValueOutputId {
                producer: self.cfg.producer_owner.clone(),
                occurrence: occurrence.clone(),
                output_projection: output_projection.clone(),
                output,
            }, point).map_err(|_| self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence))?;
            self.cfg.evaluated_values.values.push(EvaluatedValueFact {
                id: value.clone(),
                shape: output_shapes[output].clone(),
                cases: Vec::new(),
            });
            outputs.push(value);
        }
        let production = EvaluatedOutputProduction {
            producer: self.cfg.producer_owner.clone(),
            occurrence,
            output_projection,
            evaluation_point,
            outputs,
            source,
        };
        self.cfg.evaluated_values.productions.push(production);
        Ok(())
    }

    pub(in crate::scalar) fn control_output_sequence(
        &self,
        expression: &ScalarExpression,
    ) -> Result<assignment::EvaluatedOutputSequence, ReferenceAnalysisError> {
        let occurrence = SourceSpan::new(self.source.clone(), expression_span(expression));
        let projection = match expression {
            ScalarExpression::If { then_branch, .. } => &then_branch.occurrence.output_projection,
            ScalarExpression::Block(block) => &block.occurrence.output_projection,
            _ => unreachable!(),
        };
        let productions = self.cfg.evaluated_values.productions.iter().filter(|production| {
            production.producer == self.cfg.producer_owner && production.occurrence == occurrence
                && production.output_projection == *projection
        }).collect::<Vec<_>>();
        let [production] = productions.as_slice() else {
            return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
        };
        let entries = production.outputs.iter().map(|value| {
            let facts = self.cfg.evaluated_values.values.iter().filter(|fact| fact.id == *value).collect::<Vec<_>>();
            let [fact] = facts.as_slice() else {
                return Err(self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
            };
            Ok(assignment::EvaluatedOutputEntry { value: value.clone(), shape: fact.shape.clone() })
        }).collect::<Result<Vec<_>, ReferenceAnalysisError>>()?;
        Ok(assignment::EvaluatedOutputSequence { entries })
    }
}

pub(in crate::scalar) fn validate_original_control_anchor(original: &ScalarReferenceCfg, production: &EvaluatedOutputProduction) -> Result<(), ReferenceAnalysisError> {
    let mut isolated = original.clone();
    isolated.points = original.initial_points.clone();
    validate_control_anchor(&isolated, &production.occurrence, &production.output_projection, production.evaluation_point, &production.source)
}
