use super::*;
use crate::scalar::{CfgPointId, ReferenceFlowPoint};

type ReturnBranches =
    BTreeMap<(u32, u32), (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>)>;

pub(super) fn fresh_return_outputs(
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
) -> Result<(BTreeSet<usize>, ReturnBranches), String> {
    let projected = matches!(&function.signature, ScalarType::Callable { outputs, .. }
        if outputs.outputs.len() > 1)
        && projected_return_arms(&function.body).is_some();
    let mut result = BTreeSet::new();
    let mut external_outputs = BTreeSet::new();
    let mut projected_returns = BTreeMap::new();
    for point in &reference_cfg.points {
        let CfgPointKind::ReturnOutput {
            output,
            checked: true,
            value_point,
        } = point.kind
        else {
            continue;
        };
        let origins = point
            .possible_origins
            .as_ref()
            .ok_or_else(|| format!("missing validated return origin at {:?}", point.source_span))?;
        let mut local = false;
        let mut external = false;
        for origin in origins {
            match &reference_cfg.facts[origin.0] {
                ReferenceOrigin::Fresh { allocation, .. }
                    if allocation.function_span == function.span
                        && allocation.source == point.source_span.source =>
                {
                    local = true
                }
                ReferenceOrigin::Fresh { .. } | ReferenceOrigin::BorrowedFrom { .. } => {
                    external = true;
                }
                ReferenceOrigin::Null { .. } => {}
                ReferenceOrigin::Invalid { .. } => {
                    return Err(format!(
                        "invalid validated return origin at {:?}",
                        point.source_span
                    ));
                }
            }
        }
        if local {
            result.insert(output);
        }
        if external {
            external_outputs.insert(output);
        }
        if projected {
            projected_returns.insert(output, (point.id, value_point));
        }
    }
    let mixed = result
        .intersection(&external_outputs)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut branches = BTreeMap::new();
    for output in &mixed {
        if projected {
            let &(return_point, value_point) = &projected_returns[output];
            record_projected_return_branches(
                function,
                reference_cfg,
                &function.body,
                return_point,
                value_point,
                *output,
                *output,
                &mut branches,
            )?;
            continue;
        }
        let final_output = function
            .body
            .final_output_values
            .get(*output)
            .ok_or_else(|| format!("missing return output {output}"))?;
        record_return_branches(
            function,
            reference_cfg,
            &final_output.value,
            *output,
            &mut branches,
        )?;
    }
    result.retain(|output| !mixed.contains(output));
    Ok((result, branches))
}

fn projected_return_arms(block: &ScalarBlock) -> Option<[&ScalarBlock; 2]> {
    if block.final_output_values.len() != 1 {
        return None;
    }
    let (expression, _) = block.final_expressions().next()?;
    return_if_arms(expression)
}

fn return_if_arms(expression: &ScalarExpression) -> Option<[&ScalarBlock; 2]> {
    let ScalarExpression::If {
        then_branch,
        else_branch,
        ..
    } = expression
    else {
        return None;
    };
    Some([then_branch, else_branch])
}

fn record_projected_return_branches(
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    block: &ScalarBlock,
    consumer: CfgPointId,
    value_point: CfgPointId,
    syntax_output: usize,
    output: usize,
    branches: &mut ReturnBranches,
) -> Result<(), String> {
    let point = reference_cfg
        .points
        .get(value_point.0)
        .filter(|point| point.id == value_point && value_point < consumer)
        .ok_or_else(|| {
            format!("invalid return output {output} value link {consumer:?} -> {value_point:?}")
        })?;
    let final_expression = block.final_expressions().nth(syntax_output);
    let conditional = projected_return_arms(block)
        .map(|arms| (arms, syntax_output))
        .or_else(|| {
            let (expression, _) = final_expression?;
            return_if_arms(expression).map(|arms| (arms, 0))
        });
    if let Some((arms, arm_output)) = conditional {
        let CfgPointKind::ConditionalOutput {
            then_point,
            else_point,
        } = point.kind
        else {
            return Err(format!(
                "return output {output} has no conditional output at {value_point:?}"
            ));
        };
        if then_point == else_point {
            return Err(format!(
                "return output {output} has identical arm links at {value_point:?}"
            ));
        }
        for (arm, arm_point) in arms.into_iter().zip([then_point, else_point]) {
            record_projected_return_branches(
                function,
                reference_cfg,
                arm,
                value_point,
                arm_point,
                arm_output,
                output,
                branches,
            )?;
        }
        return Ok(());
    }

    if matches!(point.kind, CfgPointKind::ConditionalOutput { .. }) {
        return Err(format!(
            "return branch for output {output} has an unexpected conditional output at {value_point:?}"
        ));
    }
    let (expression, metadata) =
        final_expression.ok_or_else(|| format!("return branch has no output {output}"))?;
    record_return_branch_owner(
        function,
        reference_cfg,
        block,
        expression,
        metadata.span,
        point,
        output,
        branches,
    )
}

pub(super) fn dynamic_return_outputs(
    function_span: wosy_syntax::ByteSpan,
    reference_cfg: &ScalarReferenceCfg,
) -> BTreeSet<usize> {
    reference_cfg
        .points
        .iter()
        .filter_map(|point| {
            let CfgPointKind::ReturnOutput {
                output,
                checked: true,
                ..
            } = point.kind
            else {
                return None;
            };
            let Some(origins) = &point.possible_origins else {
                return None;
            };
            let fresh = origins.iter().any(|id| {
                matches!(
                    reference_cfg.facts[id.0],
                    ReferenceOrigin::Fresh { .. }
                )
            });
            let borrowed = origins.iter().any(|id| matches!(reference_cfg.facts[id.0], ReferenceOrigin::BorrowedFrom { .. }));
            let local = origins.iter().any(|id| matches!(&reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span == function_span && allocation.source == point.source_span.source));
            let external_fresh = origins.iter().any(|id| matches!(&reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span != function_span || allocation.source != point.source_span.source));
            (fresh && (borrowed || (local && external_fresh))).then_some(output)
        })
        .collect()
}

pub(super) fn dynamic_return_owner(
    function_span: wosy_syntax::ByteSpan,
    reference_cfg: &ScalarReferenceCfg,
) -> bool {
    !dynamic_return_outputs(function_span, reference_cfg).is_empty()
}

pub(super) fn joined_return_binding_types(
    function: &ScalarFunction,
    branches: &BTreeMap<(u32, u32), (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>)>,
) -> Result<BTreeMap<String, ScalarType>, String> {
    let mut bindings = BTreeMap::new();
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return Err("function has no callable signature".to_owned());
    };
    for (_, _, joined) in branches.values() {
        for (output, name) in joined {
            let ty = outputs
                .outputs
                .get(*output)
                .ok_or_else(|| format!("missing joined return output {output}"))?
                .ty
                .clone();
            bindings.insert(name.clone(), ty);
        }
    }
    Ok(bindings)
}

pub(super) fn joined_assignment_owners(
    function_span: wosy_syntax::ByteSpan,
    reference_cfg: &ScalarReferenceCfg,
    bindings: &BTreeMap<String, ScalarType>,
) -> Result<BTreeMap<(u32, u32), CheckedAssignmentOwner>, String> {
    let mut assignments = BTreeMap::new();
    for point in &reference_cfg.points {
        let CfgPointKind::Assign { target, .. } = &point.kind else {
            continue;
        };
        let crate::ScalarPlace::Name { name, .. } = &target.place else {
            continue;
        };
        if !bindings.contains_key(name) {
            continue;
        }
        let origins = point.possible_origins.as_ref().ok_or_else(|| {
            format!(
                "missing validated assignment owner at {:?}",
                point.source_span
            )
        })?;
        let local = origins.iter().any(|id| matches!(&reference_cfg.facts[id.0],
            ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span == function_span && allocation.source == point.source_span.source));
        let external = origins.iter().any(|id| matches!(&reference_cfg.facts[id.0],
            ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span != function_span || allocation.source != point.source_span.source));
        let owner = match (local, external) {
            (true, false) => CheckedAssignmentOwner::Local,
            (false, true) => CheckedAssignmentOwner::External,
            (false, false) => CheckedAssignmentOwner::Borrowed,
            (true, true) => {
                return Err(format!(
                    "assignment at {:?} combines local and external owners",
                    point.source_span
                ))
            }
        };
        assignments.insert(
            (point.source_span.range.start, point.source_span.range.end),
            owner,
        );
    }
    Ok(assignments)
}

pub(super) fn initialize_joined_return_bindings<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    bindings: BTreeMap<String, ScalarType>,
) -> Result<(), String> {
    for (name, ty) in bindings {
        let slot = entry_alloca(
            state,
            context.bool_type().into(),
            &format!("{name}_return_owner"),
        )?;
        state
            .builder
            .build_store(slot, context.bool_type().const_zero())
            .map_err(builder_error)?;
        state.joined_return_bindings.insert(name, (slot, ty));
    }
    Ok(())
}

fn record_return_branches(
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    expression: &ScalarExpression,
    output: usize,
    branches: &mut ReturnBranches,
) -> Result<(), String> {
    let ScalarExpression::If {
        then_branch,
        else_branch,
        ..
    } = expression
    else {
        if let ScalarExpression::Name { name, .. } = expression {
            let point = reference_cfg
                .points
                .iter()
                .find(|point| {
                    matches!(point.kind,
                CfgPointKind::ReturnOutput { output: index, checked: true, .. } if index == output)
                })
                .ok_or_else(|| format!("missing checked return output {output}"))?;
            let origins = point
                .possible_origins
                .as_ref()
                .ok_or_else(|| format!("missing checked return origins for output {output}"))?;
            let local = origins.iter().any(|id| matches!(&reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span == function.span && allocation.source == point.source_span.source));
            let external = origins.iter().any(|id| match &reference_cfg.facts[id.0] {
                ReferenceOrigin::BorrowedFrom { .. } => true,
                ReferenceOrigin::Fresh { allocation, .. } => {
                    allocation.function_span != function.span
                        || allocation.source != point.source_span.source
                }
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
            });
            if local && external {
                let selected = branches
                    .entry((function.body.span.start, function.body.span.end))
                    .or_default();
                selected.1.insert(output);
                selected.2.insert(output, name.clone());
                return Ok(());
            }
        }
        return Err(format!(
            "return output {output} has no typed branch for its joined owners"
        ));
    };
    for branch in [then_branch, else_branch] {
        let [value] = branch.final_output_values.as_slice() else {
            return Err(format!("return branch has no unique output {output}"));
        };
        if matches!(value.value, ScalarExpression::If { .. }) {
            record_return_branches(function, reference_cfg, &value.value, output, branches)?;
            continue;
        }
        let points = reference_cfg
            .points
            .iter()
            .filter(|point| {
                point.source_span.range == value.span
                    && match &value.value {
                        ScalarExpression::CheckedAddress { .. } => {
                            matches!(point.kind, CfgPointKind::Borrow { .. })
                        }
                        ScalarExpression::Name { name, .. } if name == "null" => {
                            matches!(point.kind, CfgPointKind::Evaluate(_))
                        }
                        ScalarExpression::Name { .. } => {
                            matches!(point.kind, CfgPointKind::Read { .. })
                        }
                        ScalarExpression::Call { .. } => {
                            matches!(point.kind, CfgPointKind::Call { output: 0, .. })
                        }
                        _ => matches!(point.kind, CfgPointKind::Evaluate(_)),
                    }
                    && point
                        .possible_origins
                        .as_ref()
                        .is_some_and(|origins| !origins.is_empty())
            })
            .collect::<Vec<_>>();
        let [point] = points.as_slice() else {
            return Err(format!(
                "return branch at {:?} has {} typed origin points",
                value.span,
                points.len()
            ));
        };
        record_return_branch_owner(
            function,
            reference_cfg,
            branch,
            &value.value,
            value.span,
            point,
            output,
            branches,
        )?;
    }
    Ok(())
}

fn record_return_branch_owner(
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    branch: &ScalarBlock,
    expression: &ScalarExpression,
    span: wosy_syntax::ByteSpan,
    point: &ReferenceFlowPoint,
    output: usize,
    branches: &mut ReturnBranches,
) -> Result<(), String> {
    let origins = point
        .possible_origins
        .as_ref()
        .filter(|origins| !origins.is_empty())
        .ok_or_else(|| format!("return branch at {span:?} has 0 typed origin points"))?;
    let local = origins.iter().any(|id| matches!(
        &reference_cfg.facts[id.0],
        ReferenceOrigin::Fresh { allocation, .. }
            if allocation.function_span == function.span && allocation.source == point.source_span.source
    ));
    let external = origins.iter().any(|id| match &reference_cfg.facts[id.0] {
        ReferenceOrigin::BorrowedFrom { .. } => true,
        ReferenceOrigin::Fresh { allocation, .. } => {
            allocation.function_span != function.span
                || allocation.source != point.source_span.source
        }
        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
    });
    let selected = branches
        .entry((branch.span.start, branch.span.end))
        .or_default();
    selected.1.insert(output);
    if local && external {
        let ScalarExpression::Name { name, .. } = expression else {
            return Err(format!(
                "return branch at {span:?} has joined owners without a binding"
            ));
        };
        selected.2.insert(output, name.clone());
    } else if local {
        selected.0.insert(output);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub(super) fn checked_call_owners(
    reference_cfg: &ScalarReferenceCfg,
) -> Result<BTreeMap<(u32, u32, usize), CheckedCallOwner>, String> {
    let mut owners = BTreeMap::new();
    for point in &reference_cfg.points {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { span, .. },
            output,
            ..
        } = &point.kind
        else {
            continue;
        };
        let Some(origins) = &point.possible_origins else {
            continue;
        };
        let owned = |id: &crate::scalar::ReferenceOriginId| {
            matches!(
                &reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { loan, .. }
                    if matches!(loan.origin_place.binding.kind, ReferenceBindingKind::CallOutput { point: origin, output: call_output } if origin == point.id && call_output == *output)
            )
        };
        let fresh = origins.iter().any(owned);
        let borrowed = origins.iter().any(|id| match &reference_cfg.facts[id.0] {
            ReferenceOrigin::Null { .. } => false,
            _ => !owned(id),
        });
        let owner = match (fresh, borrowed) {
            (true, true) => CheckedCallOwner::Mixed,
            (true, false) => CheckedCallOwner::Fresh,
            (false, _) => CheckedCallOwner::Borrowed,
        };
        owners.insert((span.start, span.end, *output), owner);
    }
    Ok(owners)
}

pub(super) fn function_runtime_array_result_provenance(
    function: &ScalarFunction,
) -> Option<RuntimeArrayResultProvenance> {
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return None;
    };
    if !matches!(outputs.outputs.first()?.ty, ScalarType::RuntimeArray { .. }) {
        return None;
    }
    let expression = function.body.final_output_values.first()?.value.clone();
    match expression {
        ScalarExpression::Call { .. } => Some(RuntimeArrayResultProvenance::FreshCall),
        ScalarExpression::Name { name, .. } => {
            if function.parameters.contains(&name) {
                return Some(RuntimeArrayResultProvenance::ParameterAlias);
            }
            function.body.items.iter().find_map(|item| match item {
                ScalarBlockItem::LocalBinding(binding)
                    if binding_receivers(binding)
                        .iter()
                        .any(|(receiver, _)| receiver == &name) =>
                {
                    if binding.is_allocation {
                        Some(RuntimeArrayResultProvenance::LocalOwner)
                    } else if matches!(binding.declared_type, ScalarType::Array { .. }) {
                        Some(RuntimeArrayResultProvenance::FixedWidening)
                    } else if matches!(binding.value, ScalarExpression::Call { .. }) {
                        Some(RuntimeArrayResultProvenance::FreshCall)
                    } else {
                        Some(RuntimeArrayResultProvenance::ParameterAlias)
                    }
                }
                _ => None,
            })
        }
        _ => Some(RuntimeArrayResultProvenance::FixedWidening),
    }
}

pub(super) struct RuntimeArrayRelease<'ctx> {
    pointer: BasicValueEnum<'ctx>,
    live: Option<inkwell::values::IntValue<'ctx>>,
}

pub(super) fn capture_runtime_array_release<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
) -> Result<Option<RuntimeArrayRelease<'ctx>>, String> {
    if !state
        .runtime_array_owners
        .get(name)
        .copied()
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let (slot, ty) = state
        .storage
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing runtime-array storage for {name}"))?;
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Err(format!("runtime-array owner {name} has an invalid type"));
    }
    declare_core_runtime(context, state.module);
    let pointer = if state
        .runtime_array_allocations
        .get(name)
        .copied()
        .unwrap_or(false)
    {
        slot.into()
    } else {
        state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                slot,
                "runtime_array_owner",
            )
            .map_err(builder_error)?
    };
    let live = if let Some(live) = state.runtime_array_live.get(name) {
        Some(
            state
                .builder
                .build_load(context.bool_type(), *live, "allocation_live")
                .map_err(builder_error)?
                .into_int_value(),
        )
    } else {
        None
    };
    Ok(Some(RuntimeArrayRelease { pointer, live }))
}

pub(super) fn consume_runtime_array_release<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    handle: RuntimeArrayRelease<'ctx>,
) -> Result<(), String> {
    let RuntimeArrayRelease { pointer, live } = handle;
    let release = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    if let Some(live) = live {
        let block = state
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing allocation release block".to_owned())?;
        let function = block
            .get_parent()
            .ok_or_else(|| "missing allocation release function".to_owned())?;
        let free = context.append_basic_block(function, "release.live");
        let next = context.append_basic_block(function, "release.next");
        state
            .builder
            .build_conditional_branch(live, free, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(free);
        state
            .builder
            .build_call(release, &[pointer.into()], "runtime_array_release")
            .map_err(builder_error)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
        return Ok(());
    }
    state
        .builder
        .build_call(release, &[pointer.into()], "runtime_array_release")
        .map_err(builder_error)?;
    Ok(())
}

pub(super) fn release_runtime_array_owner<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
) -> Result<(), String> {
    if let Some(handle) = capture_runtime_array_release(context, state, name)? {
        consume_runtime_array_release(context, state, handle)?;
        state.runtime_array_owners.insert(name.to_owned(), false);
    }
    Ok(())
}

pub(super) fn release_scope_runtime_array_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outer_owners: &BTreeMap<String, bool>,
) -> Result<(), String> {
    let owners = state
        .runtime_array_owners
        .keys()
        .filter(|name| !outer_owners.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    for name in owners {
        release_runtime_array_owner(context, state, &name)?;
    }
    Ok(())
}

pub(super) fn transfer_runtime_array_return(
    state: &mut EmitState<'_, '_>,
    block: &ScalarBlock,
    expression: &ScalarExpression,
    ty: &ScalarType,
) {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        if matches!(
            ty,
            ScalarType::Unit
                | ScalarType::Bool
                | ScalarType::I8
                | ScalarType::I16
                | ScalarType::I32
                | ScalarType::I64
                | ScalarType::I128
                | ScalarType::U8
                | ScalarType::U16
                | ScalarType::U32
                | ScalarType::U64
                | ScalarType::U128
                | ScalarType::F32
                | ScalarType::F64
                | ScalarType::Char
        ) || super::aggregate_transfer::holds_registered_return(state)
        {
            return;
        }
        let mut bindings = state.return_bindings.clone();
        bindings.extend(return_bindings(block));
        let mut visited = BTreeSet::new();
        for name in escaped_runtime_array_owners(expression, &bindings, &mut visited) {
            if state.runtime_array_owners.contains_key(&name) {
                state.runtime_array_owners.insert(name, false);
            }
        }
        return;
    }
    if let ScalarExpression::Name { name, .. } = expression {
        state.runtime_array_owners.insert(name.clone(), false);
    }
}

pub(super) fn return_bindings(block: &ScalarBlock) -> BTreeMap<String, ScalarExpression> {
    let mut bindings = BTreeMap::new();
    for item in &block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let receivers = binding_receivers(binding);
                if receivers.len() == 1 {
                    bindings.insert(receivers[0].0.clone(), binding.value.clone());
                } else if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions
                {
                    for ((name, _), output) in receivers.iter().zip(&binding.output_values) {
                        bindings.insert(name.clone(), output.value.clone());
                    }
                }
            }
            ScalarBlockItem::Assignment(assignment) => {
                if assignment.targets.len() == 1 {
                    bindings.insert(assignment.target.clone(), assignment.value.clone());
                }
                for (target, value) in assignment.targets.iter().zip(&assignment.values) {
                    if let crate::ScalarPlace::Name { name, .. } = &target.place {
                        bindings.insert(name.clone(), value.clone());
                    }
                }
            }
            ScalarBlockItem::Expression(expression) => {
                extend_return_bindings_from_expression(&mut bindings, expression);
            }
        }
    }
    bindings
}

fn extend_return_bindings_from_expression(
    bindings: &mut BTreeMap<String, ScalarExpression>,
    expression: &ScalarExpression,
) {
    match expression {
        ScalarExpression::Block(block) => bindings.extend(return_bindings(block)),
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => {
            bindings.extend(return_bindings(then_branch));
            bindings.extend(return_bindings(else_branch));
        }
        ScalarExpression::UnitIf { then_branch, .. } => {
            bindings.extend(return_bindings(then_branch));
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. }
        | ScalarExpression::RawAddress { .. }
        | ScalarExpression::CheckedAddress { .. }
        | ScalarExpression::Dereference { .. }
        | ScalarExpression::IndexedRead { .. }
        | ScalarExpression::StructLiteral { .. }
        | ScalarExpression::ArrayLiteral { .. }
        | ScalarExpression::Binary { .. }
        | ScalarExpression::Unary { .. }
        | ScalarExpression::Call { .. } => {}
    }
}

fn escaped_runtime_array_owners(
    expression: &ScalarExpression,
    bindings: &BTreeMap<String, ScalarExpression>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            if !visited.insert(name.clone()) {
                return BTreeSet::new();
            }
            let owners = match bindings.get(name) {
                Some(value) => escaped_runtime_array_owners(value, bindings, visited),
                None => BTreeSet::new(),
            };
            visited.remove(name);
            owners
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => {
            escaped_runtime_array_owners_in_place(place, bindings, visited)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            fields.iter().fold(BTreeSet::new(), |mut owners, field| {
                owners.extend(escaped_runtime_array_owners(
                    &field.value,
                    bindings,
                    visited,
                ));
                owners
            })
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            elements
                .iter()
                .fold(BTreeSet::new(), |mut owners, element| {
                    owners.extend(escaped_runtime_array_owners(element, bindings, visited));
                    owners
                })
        }
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => {
            let mut owners = escaped_runtime_array_owners_in_block(then_branch, bindings, visited);
            owners.extend(escaped_runtime_array_owners_in_block(
                else_branch,
                bindings,
                visited,
            ));
            owners
        }
        ScalarExpression::Block(block) => {
            escaped_runtime_array_owners_in_block(block, bindings, visited)
        }
        ScalarExpression::Dereference { place, .. }
        | ScalarExpression::IndexedRead { place, .. } => {
            escaped_runtime_array_owners_in_place(place, bindings, visited)
        }
        ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. }
        | ScalarExpression::Binary { .. }
        | ScalarExpression::Unary { .. }
        | ScalarExpression::Call { .. }
        | ScalarExpression::UnitIf { .. } => BTreeSet::new(),
    }
}

fn escaped_runtime_array_owners_in_place(
    place: &crate::ScalarPlace,
    bindings: &BTreeMap<String, ScalarExpression>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => {
            let mut owners = BTreeSet::from([name.clone()]);
            if let Some(value) = bindings.get(name) {
                owners.extend(escaped_runtime_array_owners(value, bindings, visited));
            }
            owners
        }
        crate::ScalarPlace::Field { base, .. } | crate::ScalarPlace::Index { base, .. } => {
            escaped_runtime_array_owners_in_place(base, bindings, visited)
        }
        crate::ScalarPlace::Dereference { pointer, .. } => {
            escaped_runtime_array_owners(pointer, bindings, visited)
        }
    }
}

fn escaped_runtime_array_owners_in_block(
    block: &ScalarBlock,
    bindings: &BTreeMap<String, ScalarExpression>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    let mut local_bindings = bindings.clone();
    local_bindings.extend(return_bindings(block));
    block
        .final_output_values
        .iter()
        .fold(BTreeSet::new(), |mut owners, output| {
            owners.extend(escaped_runtime_array_owners(
                &output.value,
                &local_bindings,
                visited,
            ));
            owners
        })
}

pub(super) fn emit_runtime_array_allocation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let (element_size, alignment) = allocation_layout(element, state.layouts, state.target_layout)?;
    let size = state
        .builder
        .build_int_mul(
            length,
            context.i64_type().const_int(element_size, false),
            "allocation_size",
        )
        .map_err(builder_error)?;
    let allocator = state
        .module
        .get_function(CORE_ALLOC_SYMBOL)
        .ok_or_else(|| "core allocation runtime declaration is missing".to_owned())?;
    let allocation = state
        .builder
        .build_call(
            allocator,
            &[
                size.into(),
                context.i64_type().const_int(alignment, false).into(),
            ],
            "allocation",
        )
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "core allocation runtime returned no pointer".to_owned())?
        .into_pointer_value();
    let failed = state
        .builder
        .build_is_null(allocation, "allocation_failed")
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .ok_or_else(|| "runtime allocation has no containing function".to_owned())?;
    let panic_block = context.append_basic_block(function, "allocation_panic");
    let continue_block = context.append_basic_block(function, "allocation_continue");
    state
        .builder
        .build_conditional_branch(failed, panic_block, continue_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(panic_block);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(panic, &[], "system_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(continue_block);
    Ok(allocation)
}

pub(super) fn binding_receivers(binding: &crate::ScalarBinding) -> Vec<(String, ScalarType)> {
    if binding.receivers.is_empty() {
        vec![(binding.name.clone(), binding.declared_type.clone())]
    } else {
        binding
            .receivers
            .iter()
            .map(|receiver| (receiver.name.clone(), receiver.ty.clone()))
            .collect()
    }
}
