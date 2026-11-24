use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CheckedForwardingProof {
    recursive: (OccurrenceOrdinal, CfgPointId),
    step: (OccurrenceOrdinal, CfgPointId),
}

impl CheckedForwardingProof {
    pub(in crate::scalar) fn recursive(self) -> OccurrenceOrdinal {
        self.recursive.0
    }

    pub(crate) fn primitive_outputs(self, cfg: &ScalarReferenceCfg) -> Option<[CfgPointId; 2]> {
        let [(recursive, self_point), (step, step_point)] = [self.recursive, self.step];
        (self_point != step_point
            && cfg.primary_call_point(recursive) == Some(self_point)
            && cfg.primary_call_point(step) == Some(step_point))
        .then_some([self_point, step_point])
    }
}

pub(in crate::scalar) fn prove(
    source: &SourceIdentity,
    function: &ScalarFunction,
    calls: &FunctionCallSites,
    cfg: &ScalarReferenceCfg,
) -> Option<CheckedForwardingProof> {
    let [view, state] = calls.boundary.parameters.as_slice() else {
        return None;
    };
    let [result] = calls.boundary.results.as_slice() else {
        return None;
    };
    if !matches!(&view.ty, ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner } if **inner == ScalarType::U8)
        || view.ordinal != 0
        || state.ordinal != 1
        || state.ty != ScalarType::I32
        || result.ty != ScalarType::I32
        || !function.overload_arms.is_empty()
        || !calls.boundary.generics.is_empty()
        || !scalar_block(&function.body, true)
        || !calls.structure.checked_types.places.is_empty()
    {
        return None;
    }
    let [recursive] = returned_self_calls(source, function, calls)[..] else {
        return None;
    };
    let argument = |call, index| match calls.structure.edges.get(&(
        OccurrenceParent::Expression(call),
        OccurrenceEdge::CallArgument(index),
    ))? {
        OccurrenceNode::Expression(ordinal) => Some(*ordinal),
        _ => None,
    };
    let step = argument(recursive, 1)?;
    let [_, _] = calls.call_occurrences.as_slice() else {
        return None;
    };
    let target = |ordinal| {
        let row = calls
            .call_occurrences
            .iter()
            .find(|row| row.ordinal == ordinal)?;
        Some(&calls.sites.get(row.site)?.target)
    };
    let recursive_target = target(recursive)?;
    let step_target = target(step)?;
    if step_target == recursive_target
        || !matches!(
            step_target,
            ResolvedCallTarget::ModuleCallable {
                concrete: ConcreteCallSelection::Declaration,
                ..
            }
        )
    {
        return None;
    }
    for (ordinal, parameters) in [
        (recursive, &[&view.ty, &state.ty][..]),
        (step, &[&view.ty][..]),
    ] {
        let signature = calls.checked_call_signatures.get(&ordinal)?;
        let [output] = signature.outputs.outputs.as_slice() else {
            return None;
        };
        if signature.overload.is_some()
            || !signature.parameters.iter().eq(parameters.iter().copied())
            || output.ty != ScalarType::I32
        {
            return None;
        }
    }
    let names = [argument(recursive, 0)?, argument(step, 0)?];
    let formal = ReferenceBindingId {
        source: source.clone(),
        function_span: function.span,
        declaration_span: view.span,
        block_span: function.body.span,
        kind: ReferenceBindingKind::Declared,
    };
    let PlainNameFacts::Prepared(identities) = &calls.plain_names else {
        return None;
    };
    let types = &calls.structure.checked_types.expressions;
    if names[0] == names[1]
        || !names.iter().all(|ordinal| {
            types.get(ordinal) == Some(&view.ty)
                && matches!(identities.get(ordinal), Some(PlainNameIdentity::Lexical(binding)) if *binding == formal)
        })
        || !types.iter().all(|(ordinal, ty)| names.contains(ordinal) || scalar_or_unit(ty))
    {
        return None;
    }
    let [ReferenceOrigin::BorrowedFrom { parameter: 0, loan }] = cfg.facts.as_slice() else {
        return None;
    };
    if loan.mode != ScalarReferenceMutability::Shared
        || loan.parent.is_some()
        || loan.origin_place.binding != formal
        || !loan.origin_place.projections.is_empty()
        || !matches!(loan.origin_place.place, ScalarPlace::Name { .. })
        || loan.creation_span != formal.declaration_span
        || loan.origin_place.source != formal.source
        || loan.origin_place.function_span != formal.function_span
        || loan.origin_place.declaration_span != formal.declaration_span
        || loan.origin_place.block_span != formal.block_span
        || !cfg.allocations.is_empty()
        || !cfg.specialized_policies.is_empty()
    {
        return None;
    }
    let self_call = cfg.primary_call_point(recursive)?;
    let step_call = cfg.primary_call_point(step)?;
    let CfgPointKind::Call {
        argument_points: self_args,
        target,
        ..
    } = &cfg.points[self_call.0].kind
    else {
        return None;
    };
    let [self_read, step_result] = self_args[..] else {
        return None;
    };
    let CfgPointKind::Call {
        argument_points: step_args,
        target: actual_step,
        ..
    } = &cfg.points[step_call.0].kind
    else {
        return None;
    };
    let [step_read] = step_args[..] else {
        return None;
    };
    if target != recursive_target || actual_step != step_target || step_result != step_call {
        return None;
    }
    let same_graph = cfg.points.len() == cfg.initial_points.len()
        && cfg
            .points
            .iter()
            .zip(&cfg.initial_points)
            .all(|(point, initial)| {
                point.id == initial.id
                    && point.kind == initial.kind
                    && point.successors == initial.successors
            });
    if !same_graph
        || ![&cfg.points, &cfg.initial_points]
            .into_iter()
            .all(|points| {
                check_graph(points, &formal, [self_read, step_read, step_call, self_call])
            })
    {
        return None;
    }
    Some(CheckedForwardingProof {
        recursive: (recursive, self_call),
        step: (step, step_call),
    })
}

fn incoming_origin(point: &ReferenceFlowPoint) -> bool {
    point
        .possible_origins
        .as_ref()
        .is_some_and(|origins| origins.len() == 1 && origins.contains(&ReferenceOriginId(0)))
}

fn check_graph(
    points: &[ReferenceFlowPoint],
    formal: &ReferenceBindingId,
    [self_read, step_read, step_call, self_call]: [CfgPointId; 4],
) -> bool {
    let successor = |id: CfgPointId| {
        let [next] = points.get(id.0)?.successors[..] else {
            return None;
        };
        Some(next)
    };
    // The two source-ordered arguments copy the same incoming shared loan.
    // Even one intervening affine move or write invalidates this proof.
    let Some(bind) = successor(CfgPointId(0)) else {
        return false;
    };
    if successor(self_read) != Some(step_read)
        || successor(step_read) != Some(step_call)
        || successor(step_call) != Some(self_call)
        || !matches!(points.get(bind.0), Some(ReferenceFlowPoint { kind: CfgPointKind::Bind { binding }, .. }) if binding == formal)
        || !matches!(points[0].kind, CfgPointKind::Entry)
        || ![self_read, step_read]
            .into_iter()
            .all(|id| matches!(points[id.0].kind, CfgPointKind::Read { .. }))
    {
        return false;
    }
    if !points.iter().enumerate().all(|(index, point)| {
        let id = point.id;
        if id.0 != index
            || point
                .successors
                .iter()
                .any(|next| next.0 <= index || next.0 >= points.len())
            || (id != bind
                && id != self_read
                && id != step_read
                && id != self_call
                && id != step_call
                && point
                    .possible_origins
                    .as_ref()
                    .is_some_and(|origins| !origins.is_empty()))
        {
            return false;
        }
        match &point.kind {
            CfgPointKind::Entry => index == 0,
            CfgPointKind::Bind { binding } => {
                id == bind && binding == formal && incoming_origin(point)
            }
            CfgPointKind::Read { binding } if id == self_read || id == step_read => {
                binding == formal && incoming_origin(point)
            }
            CfgPointKind::Read { binding } => binding != formal,
            CfgPointKind::Call { output: 0, .. } => {
                (id == step_call || id == self_call)
                    && (point
                        .possible_origins
                        .as_ref()
                        .is_none_or(BTreeSet::is_empty)
                        || incoming_origin(point))
            }
            CfgPointKind::ScopeEnter
            | CfgPointKind::ScopeExit
            | CfgPointKind::Evaluate(_)
            | CfgPointKind::Branch { .. }
            | CfgPointKind::Join
            | CfgPointKind::ReturnOutput {
                output: 0,
                checked: false,
                ..
            }
            | CfgPointKind::Return => true,
            _ => false,
        }
    }) {
        return false;
    }
    let Some(mut next) = successor(self_call) else {
        return false;
    };
    let mut value = self_call;
    let mut returned = false;
    loop {
        match points[next.0].kind {
            CfgPointKind::ScopeExit => {}
            CfgPointKind::Join if !returned => value = next,
            CfgPointKind::ReturnOutput { value_point, .. } if !returned && value_point == value => {
                returned = true
            }
            CfgPointKind::Return => return returned && points[next.0].successors.is_empty(),
            _ => return false,
        }
        let Some(successor) = successor(next) else {
            return false;
        };
        next = successor;
    }
}

fn scalar_block(block: &ScalarBlock, returned: bool) -> bool {
    let outputs = block.final_output_values.len();
    outputs <= 1
        && (!returned || outputs == 1)
        && block.items.iter().enumerate().all(|(index, item)| {
            matches!(item, ScalarBlockItem::Expression(expression)
                if scalar_expression(expression, returned && index + outputs >= block.items.len()))
        })
}

fn scalar_expression(expression: &ScalarExpression, returned: bool) -> bool {
    match expression {
        ScalarExpression::Name { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. } => true,
        ScalarExpression::Unary { operand, .. } => scalar_expression(operand, false),
        ScalarExpression::Binary { left, right, .. } => {
            scalar_expression(left, false) && scalar_expression(right, false)
        }
        ScalarExpression::Call { arguments, .. } => arguments
            .iter()
            .all(|argument| scalar_expression(argument, false)),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            scalar_expression(condition, false)
                && scalar_block(then_branch, returned)
                && scalar_block(else_branch, returned)
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => scalar_expression(condition, false) && scalar_block(then_branch, false),
        ScalarExpression::Block(block) => scalar_block(block, returned),
        _ => false,
    }
}
