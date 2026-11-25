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
    let [first, second] = calls.boundary.parameters.as_slice() else {
        return None;
    };
    let (view, state, mode) = match (&first.ty, &second.ty) {
        (ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner }, ScalarType::I32)
            if **inner == ScalarType::U8 => (first, second, ScalarReferenceMutability::Shared),
        (ScalarType::I32, ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner })
            if **inner == ScalarType::U8 => (second, first, ScalarReferenceMutability::Mutable),
        _ => return None,
    };
    let mutable = mode == ScalarReferenceMutability::Mutable;
    let [result] = calls.boundary.results.as_slice() else {
        return None;
    };
    if first.ordinal != 0
        || second.ordinal != 1
        || result.ty != ScalarType::I32
        || !function.overload_arms.is_empty()
        || !calls.boundary.generics.is_empty()
        || !scalar_block(&function.body, true)
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
    let step = argument(recursive, state.ordinal)?;
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
        (recursive, &[&first.ty, &second.ty][..]),
        (step, &[&view.ty][..]),
    ] {
        let signature = calls.checked_call_signatures.get(&ordinal)?;
        let [output] = signature.outputs.outputs.as_slice() else {
            return None;
        };
        if (mutable && calls.declared_unsafe_calls.contains(&ordinal.0))
            || signature.overload.is_some()
            || !signature.parameters.iter().eq(parameters.iter().copied())
            || output.ty != ScalarType::I32
        {
            return None;
        }
    }
    let direct = argument(recursive, view.ordinal)?;
    let step_argument = argument(step, 0)?;
    let (step_name, reborrow_place) = if mutable {
        let OccurrenceNode::Place(place) = calls.structure.edges.get(&(
            OccurrenceParent::Expression(step_argument),
            OccurrenceEdge::ExpressionPlace,
        ))? else { return None };
        let OccurrenceNode::Expression(pointer) = calls.structure.edges.get(&(
            OccurrenceParent::Place(*place),
            OccurrenceEdge::DereferencePointer,
        ))? else { return None };
        (*pointer, Some(*place))
    } else {
        (step_argument, None)
    };
    let place_types = &calls.structure.checked_types.places;
    if match reborrow_place {
        Some(place) => place_types.len() != 1 || place_types.get(&place) != Some(&ScalarType::U8),
        None => !place_types.is_empty(),
    } {
        return None;
    }
    let names = [direct, step_name];
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
        || types.get(&step_argument) != Some(&view.ty)
        || !types.iter().all(|(ordinal, ty)| {
            names.contains(ordinal) || (mutable && *ordinal == step_argument) || scalar_or_unit(ty)
        })
    {
        return None;
    }
    let (loan, child) = match cfg.facts.as_slice() {
        [ReferenceOrigin::BorrowedFrom { parameter, loan }] if !mutable && *parameter == view.ordinal => (loan, None),
        [ReferenceOrigin::BorrowedFrom { parameter, loan }, ReferenceOrigin::BorrowedFrom { parameter: p, loan: child }]
            if mutable && *parameter == view.ordinal && p == parameter
                && child.parent.as_deref() == Some(loan)
                && child.mode == mode
                && child.origin_place.binding == formal
                && matches!(child.origin_place.projections.as_slice(),
                    [ReferencePlaceProjection::Dereference(parent)] if parent.as_ref() == loan)
                && matches!(&child.origin_place.place, ScalarPlace::Dereference { .. }) =>
                    (loan, Some(child)),
        _ => return None,
    };
    if loan.mode != mode
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
    let [first_arg, second_arg] = self_args[..] else {
        return None;
    };
    let (self_read, step_result) = if mutable { (second_arg, first_arg) } else { (first_arg, second_arg) };
    let CfgPointKind::Call {
        argument_points: step_args,
        target: actual_step,
        ..
    } = &cfg.points[step_call.0].kind
    else {
        return None;
    };
    let [step_value] = step_args[..] else {
        return None;
    };
    let step_read = if let Some(child) = child {
        if !matches!(&cfg.points.get(step_value.0)?.kind, CfgPointKind::Borrow { loan, place }
            if loan == child && place == &child.origin_place)
        {
            return None;
        }
        CfgPointId(step_value.0.checked_sub(1)?)
    } else {
        step_value
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
                check_graph(points, &formal, &view.ty,
                    [self_read, step_read, step_call, self_call], child.map(|_| step_value))
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
    view_ty: &ScalarType,
    [self_read, step_read, step_call, self_call]: [CfgPointId; 4],
    reborrow: Option<CfgPointId>,
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
    let movement = if reborrow.is_some() { successor(self_read) } else { None };
    let corridor = match (reborrow, movement) {
        (Some(child), Some(moved)) =>
            successor(step_read) == Some(child)
                && successor(child) == Some(step_call)
                && successor(step_call) == Some(self_read)
                && points.get(moved.0).is_some_and(|point| matches!(point.kind, CfgPointKind::Move { .. }))
                && successor(moved) == Some(self_call),
        (None, None) => successor(self_read) == Some(step_read)
                && successor(step_read) == Some(step_call)
                && successor(step_call) == Some(self_call),
        _ => false,
    };
    if !corridor
        || !matches!(points.get(bind.0), Some(ReferenceFlowPoint { kind: CfgPointKind::Bind { binding }, .. }) if binding == formal)
        || !matches!(points[0].kind, CfgPointKind::Entry)
        || ![self_read, step_read]
            .into_iter()
            .all(|id| matches!(points[id.0].kind, CfgPointKind::Read { .. }))
    {
        return false;
    }
    let exclusive_edges = reborrow.zip(movement).map(|(child, moved)| [
        (child, step_read),
        (step_call, child),
        (self_read, step_call),
        (moved, self_read),
        (self_call, moved),
    ]);
    if !points.iter().enumerate().all(|(index, point)| {
        let id = point.id;
        if id.0 != index
            || point
                .successors
                .iter()
                .any(|next| next.0 <= index || next.0 >= points.len())
            || exclusive_edges.as_ref().is_some_and(|edges| {
                point.successors.iter().any(|next| {
                    edges.iter().any(|(target, predecessor)| next == target && id != *predecessor)
                })
            })
            || (id != bind
                && id != self_read
                && id != step_read
                && id != self_call
                && id != step_call
                && Some(id) != reborrow
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
            CfgPointKind::Move { place, ty, owner } => Some(id) == movement
                && ty == view_ty && owner == formal && place.binding == *formal
                && place.projections.is_empty() && matches!(place.place, ScalarPlace::Name { .. }),
            CfgPointKind::Borrow { .. } => Some(id) == reborrow
                && point.possible_origins.as_ref().is_some_and(|origins| {
                    origins.len() == 1 && origins.contains(&ReferenceOriginId(1))
                }),
            CfgPointKind::Call { output: 0, .. } => {
                (id == step_call || id == self_call)
                    && (point
                        .possible_origins
                        .as_ref()
                        .is_none_or(BTreeSet::is_empty)
                        || incoming_origin(point)
                        || (id == step_call && reborrow.is_some()
                            && point.possible_origins.as_ref().is_some_and(|origins| {
                                origins.len() == 1 && origins.contains(&ReferenceOriginId(1))
                            })))
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
    !block.unsafe_context
        && outputs <= 1
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
        ScalarExpression::CheckedAddress { mutability: ScalarReferenceMutability::Mutable, place, .. } =>
            matches!(place, ScalarPlace::Dereference { pointer, .. }
                if matches!(pointer.as_ref(), ScalarExpression::Name { .. })),
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
