use super::*;
use crate::scalar::{CfgPointId, ReferenceBindingId, ReferenceFlowPoint};
use wosy_syntax::ByteSpan;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ReturnBranch {
    pub(super) local: BTreeSet<usize>,
    pub(super) covered: BTreeSet<usize>,
    pub(super) joined: BTreeMap<usize, String>,
    pub(super) forwarded: BTreeMap<usize, String>,
}

pub(super) type ReturnBranches = BTreeMap<(u32, u32), ReturnBranch>;

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
    let mut branches = BTreeMap::new();
    let mut named_external_joins = Vec::new();
    let mut dynamic_count = 0;
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
        let mut external_fresh = false;
        let mut borrowed = false;
        for origin in origins {
            match &reference_cfg.facts[origin.0] {
                ReferenceOrigin::Fresh { allocation, .. }
                    if allocation.function_span == function.span
                        && allocation.source == point.source_span.source =>
                {
                    local = true
                }
                ReferenceOrigin::Fresh { .. } => external_fresh = true,
                ReferenceOrigin::BorrowedFrom { .. } => borrowed = true,
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
        if external_fresh || borrowed {
            external_outputs.insert(output);
        }
        dynamic_count += usize::from((local || external_fresh) && borrowed || local && external_fresh);
        // A named result joining a forwarded allocation and a borrowed
        // address already has an ABI owner slot. Its lexical slot must track
        // assignments just like a local-allocation/borrowed join.
        if external_fresh && borrowed && !local && !projected {
            if let Some(metadata) = function.body.final_output_values.get(output) {
                let read = &reference_cfg.points[value_point.0];
                if let (
                    ScalarExpression::Name { name, .. },
                    CfgPointKind::Read { binding },
                ) = (&metadata.value, &read.kind) {
                    if function.body.span == binding.block_span
                        && binding.function_span == function.span
                        && binding.source == point.source_span.source
                        && binding.kind == ReferenceBindingKind::Declared
                        && read.source_span.range == metadata.span
                        && read.possible_origins.as_ref() == Some(origins)
                    {
                        named_external_joins.push((output, name, binding));
                    }
                }
            }
        }
        if projected {
            projected_returns.insert(output, (point.id, value_point));
        }
    }
    if !named_external_joins.is_empty() {
        let names = exact_null_root_owner_names(function, reference_cfg);
        let mut seen = BTreeSet::new();
        // Branch materialization handles this whole sidecar cohort. Do not
        // suppress legacy forwarding for an unhandled member, or write two
        // dynamic slots from one current owner.
        if named_external_joins.len() == dynamic_count
            && named_external_joins.iter().all(|(_, name, binding)| {
                names.get(*name) == Some(binding) && seen.insert(*binding)
            })
        {
            let selected = branches.entry((function.body.span.start, function.body.span.end))
                .or_insert_with(ReturnBranch::default);
            for (output, name, _) in named_external_joins {
                selected.covered.insert(output);
                selected.joined.insert(output, name.clone());
            }
        }
    }
    let mixed = result
        .intersection(&external_outputs)
        .copied()
        .collect::<BTreeSet<_>>();
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

/// The existing storage/assignment state is keyed by spelling. Only admit
/// exact root bindings here if no nested declaration can shadow them, the
/// spelling is unique among root declarations/parameters, and the checked
/// initializer establishes the false initial sidecar without allocating.
fn exact_null_root_owner_names<'a>(
    function: &'a ScalarFunction,
    cfg: &'a ScalarReferenceCfg,
) -> BTreeMap<&'a String, &'a ReferenceBindingId> {
    let mut null_bindings = BTreeMap::new();
    for point in &cfg.points {
        let CfgPointKind::Bind { binding } = &point.kind else { continue };
        if binding.kind != ReferenceBindingKind::Declared
            || binding.function_span != function.span
            || binding.block_span != function.body.span
        {
            return BTreeMap::new();
        }
        if point.possible_origins.as_ref().is_some_and(|origins| {
            !origins.is_empty() && origins.iter().all(|id| {
                matches!(cfg.facts.get(id.0), Some(ReferenceOrigin::Null { .. }))
            })
        }) {
            let span = binding.declaration_span;
            if null_bindings.insert((span.start, span.end), binding).is_some() {
                return BTreeMap::new();
            }
        }
    }
    let mut names = BTreeMap::new();
    for name in &function.parameters {
        names.insert(name, None);
    }
    for item in &function.body.items {
        let ScalarBlockItem::LocalBinding(binding) = item else { continue };
        let mut register = |name, span: ByteSpan, eligible| {
            let owner = if eligible {
                null_bindings.get(&(span.start, span.end)).copied()
            } else {
                None
            };
            names.entry(name).and_modify(|slot| *slot = None).or_insert(owner);
        };
        let initializer_is_null = !binding.is_allocation
            && binding.fallible_assignment.is_none()
            && matches!(&binding.value, ScalarExpression::Name { name, .. } if name == "null");
        match binding.receivers.as_slice() {
            [] => register(&binding.name, binding.name_span, initializer_is_null),
            [receiver] => register(&receiver.name, receiver.name_span, initializer_is_null),
            receivers => {
                for receiver in receivers {
                    register(&receiver.name, receiver.name_span, false);
                }
            }
        }
    }
    names.into_iter().filter_map(|(name, binding)| Some((name, binding?))).collect()
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
        Some(syntax_output),
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

pub(super) fn joined_return_binding_types(
    function: &ScalarFunction,
    branches: &ReturnBranches,
) -> Result<BTreeMap<String, ScalarType>, String> {
    let mut bindings = BTreeMap::new();
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return Err("function has no callable signature".to_owned());
    };
    for branch in branches.values() {
        for (output, name) in &branch.joined {
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
                selected.covered.insert(output);
                selected.joined.insert(output, name.clone());
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
            None,
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
    projected_slot: Option<usize>,
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
    selected.covered.insert(output);
    if local && external {
        let ScalarExpression::Name { name, .. } = expression else {
            return Err(format!(
                "return branch at {span:?} has joined owners without a binding"
            ));
        };
        selected.joined.insert(output, name.clone());
    } else if local {
        selected.local.insert(output);
    } else if external && projected_slot == Some(output) {
        if let Some(name) = exact_named_call_return(reference_cfg, branch, expression, point) {
            selected.forwarded.insert(output, name.to_owned());
        }
    }
    Ok(())
}

fn exact_named_call_return<'a>(
    cfg: &ScalarReferenceCfg,
    branch: &ScalarBlock,
    expression: &'a ScalarExpression,
    point: &ReferenceFlowPoint,
) -> Option<&'a str> {
    if branch.unsafe_context
        || branch.final_output_values.len() != 2
        || branch.items.len() != branch.final_output_values.len() + 1
    {
        return None;
    }
    let ScalarBlockItem::LocalBinding(initializer) = &branch.items[0] else { return None };
    let (
        ScalarExpression::Name { name, .. },
        ScalarExpression::Call { span, .. },
        CfgPointKind::Read { binding },
    ) = (expression, &initializer.value, &point.kind) else { return None };
    if initializer.is_allocation
        || initializer.output_origin != ScalarBindingOutputOrigin::SingleExpression
        || binding.block_span != branch.span
    {
        return None;
    }
    let (slot, receiver) = initializer.receivers.iter().enumerate()
        .find(|(_, receiver)| receiver.name == *name)?;
    if !matches!(receiver.ty, ScalarType::CheckedReference { .. })
        || binding.declaration_span != receiver.name_span
    {
        return None;
    }
    let [first, second] = initializer.receivers.as_slice() else { return None };
    let sibling = if slot == 0 { second } else { first };
    if sibling.name == *name
        || !(integer_width(&sibling.ty).is_some()
            || matches!(sibling.ty, ScalarType::Bool | ScalarType::Char | ScalarType::F32 | ScalarType::F64 | ScalarType::Enum(_)))
    {
        return None;
    }
    let mut terminals = branch.final_expressions();
    let ((first, _), (second, _)) = (terminals.next()?, terminals.next()?);
    let other = if std::ptr::eq(first, expression) { second }
        else if std::ptr::eq(second, expression) { first }
        else { return None };
    if !matches!(other, ScalarExpression::Name { name: terminal, .. } if terminal == &sibling.name) {
        return None;
    }
    let mut acquired = false;
    for origin in point.possible_origins.as_ref()? {
        match cfg.facts.get(origin.0)? {
            ReferenceOrigin::Fresh { loan, .. } => {
                let ReferenceBindingKind::CallOutput { point: call, output } = loan.origin_place.binding.kind else { return None };
                let call = cfg.points.get(call.0)?;
                let CfgPointKind::Call { expression: ScalarExpression::Call { span: call_span, .. }, output: call_output, .. } = &call.kind else { return None };
                if call.id >= point.id || *call_span != *span || output != slot || *call_output != slot {
                    return None;
                }
                acquired = true;
            }
            ReferenceOrigin::BorrowedFrom { .. } | ReferenceOrigin::Null { .. } => {}
            ReferenceOrigin::Invalid { .. } => return None,
        }
    }
    acquired.then_some(name.as_str())
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
    pub(super) pointer: BasicValueEnum<'ctx>,
    pub(super) live: Option<inkwell::values::IntValue<'ctx>>,
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

pub(super) fn release_proven_local_array<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    pointer: PointerValue<'ctx>,
    live_slot: PointerValue<'ctx>,
) -> Result<(), String> {
    let live = state.builder
        .build_load(context.bool_type(), live_slot, "allocation_live")
        .map_err(builder_error)?
        .into_int_value();
    declare_core_runtime(context, state.module);
    consume_runtime_array_release(context, state, RuntimeArrayRelease {
        pointer: pointer.into(),
        live: Some(live),
    })?;
    state.builder.build_store(live_slot, context.bool_type().const_zero())
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
    retained_ancestor_owners: Option<&BTreeMap<String, bool>>,
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
                | ScalarType::Enum(_)
        ) || super::aggregate_transfer::holds_registered_return(state, expression)
        {
            return;
        }
        let mut bindings = state.return_bindings.clone();
        bindings.extend(return_bindings(block));
        let mut visited = BTreeSet::new();
        for name in escaped_runtime_array_owners(expression, &bindings, &mut visited) {
            // An ordinary block yielding a derived view does not transfer
            // storage already owned by its enclosing scope. Its caller may
            // still read the view and must clean up the owner at its own exit.
            if retained_ancestor_owners.is_some_and(|owners| owners.contains_key(&name)) {
                continue;
            }
            if state.runtime_array_owners.contains_key(&name) {
                state.runtime_array_owners.insert(name, false);
            }
        }
        return;
    }
    if super::aggregate_transfer::literal_array_leaf_return(state) {
        return;
    }
    if let ScalarExpression::Name { name, .. } = expression {
        retire_named_runtime_array_return(state, name);
    }
}

pub(super) fn retire_named_runtime_array_return(state: &mut EmitState<'_, '_>, name: &str) {
    state.runtime_array_owners.insert(name.to_owned(), false);
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
        | ScalarExpression::ValueField { .. }
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
        | ScalarExpression::ValueField { .. }
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

pub(super) fn emit_standard_allocation_call<'ctx>(
    state: &EmitState<'ctx, '_>,
    size: BasicValueEnum<'ctx>,
    alignment: BasicValueEnum<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    let allocator = state
        .module
        .get_function(CORE_ALLOC_SYMBOL)
        .ok_or_else(|| "core allocation runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(allocator, &[size.into(), alignment.into()], "allocation")
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .map(BasicValueEnum::into_pointer_value)
        .ok_or_else(|| "core allocation runtime returned no pointer".to_owned())
}

pub(super) fn emit_managed_allocation_branch<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    size: inkwell::values::IntValue<'ctx>,
    alignment: u64,
    failure: inkwell::basic_block::BasicBlock<'ctx>,
    success: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    let allocation = emit_standard_allocation_call(
        state,
        size.into(),
        context.i64_type().const_int(alignment, false).into(),
    )?;
    let failed = state.builder.build_is_null(allocation, "allocation_failed").map_err(builder_error)?;
    state.builder.build_conditional_branch(failed, failure, success).map_err(builder_error)?;
    Ok(allocation)
}

struct GuardedScalarSite {
    scope: ByteSpan,
    ty: ScalarType,
    runtime_literal_length: Option<u64>,
    runtime_returned: bool,
    movable: bool,
}

pub(super) struct GuardedScalarSites<'ctx> {
    sites: Box<[GuardedScalarSite]>,
    declarations: HashMap<(u32, u32), usize>,
    conditions: HashMap<usize, (usize, bool)>,
    aliases: HashMap<usize, usize>,
    runtime_moves: HashMap<(u32, u32), (usize, usize, usize)>,
    runtime_replacements: HashMap<usize, (usize, usize, usize)>,
    releases: HashMap<(u32, u32), usize>,
    active: Vec<Option<GuardedScalarOwner<'ctx>>>,
    lexical: Vec<usize>,
}

impl<'ctx> GuardedScalarSites<'ctx> {
    pub(super) fn from_cfg(cfg: &ScalarReferenceCfg) -> Option<Box<Self>> {
        let proofs = cfg.guarded_scalar_allocations();
        if proofs.is_empty() { return None; }
        let mut declarations = HashMap::with_capacity(proofs.len());
        let mut conditions = HashMap::new();
        let mut aliases = HashMap::new();
        let mut releases = HashMap::new();
        for (index, proof) in proofs.iter().enumerate() {
            for span in proof.releases() {
                if releases.insert((span.start, span.end), index).is_some() { return None; }
            }
            let declaration = proof.binding().declaration_span;
            if declarations.insert((declaration.start, declaration.end), index).is_some() { return None; }
            for guard in proof.guards() {
                if conditions.insert(guard.condition, (index, guard.present_on_then)).is_some() { return None; }
            }
            for &alias in proof.alias_accesses() {
                if aliases.insert(alias, index).is_some() { return None; }
            }
        }
        let mut sites = proofs.iter().map(|proof| GuardedScalarSite {
            scope: proof.binding().block_span, ty: proof.ty().clone(),
            runtime_literal_length: proof.runtime_literal_length(),
            runtime_returned: proof.runtime_returned(), movable: !proof.runtime_moves().is_empty() || !proof.releases().is_empty(),
        }).collect::<Vec<_>>();
        let mut runtime_moves = HashMap::new();
        for (source, proof) in proofs.iter().enumerate() {
            for receipt in proof.runtime_moves() {
                if &receipt.source != proof.binding() { return None; }
                if receipt.replacement.is_some() { continue; }
                let span = receipt.target.declaration_span;
                let declaration = (span.start, span.end);
                let target = sites.len();
                if declarations.insert(declaration, target).is_some()
                    || runtime_moves.insert(declaration, (source, target, receipt.rhs)).is_some()
                { return None; }
                sites.push(GuardedScalarSite { scope: receipt.target.block_span, ty: receipt.target_ty.clone(),
                    runtime_literal_length: None, runtime_returned: true, movable: false });
            }
        }
        let mut runtime_replacements = HashMap::new();
        for (source, proof) in proofs.iter().enumerate() {
            for receipt in proof.runtime_moves() {
                let Some(destination) = receipt.replacement else { continue; };
                let span = receipt.target.declaration_span;
                let &target = declarations.get(&(span.start, span.end))?;
                if sites[target].movable || sites[target].ty != receipt.target_ty
                    || runtime_replacements.insert(destination, (source, target, receipt.rhs)).is_some()
                { return None; }
            }
        }
        let active = vec![None; sites.len()];
        Some(Box::new(Self {
            sites: sites.into_boxed_slice(), declarations, conditions, aliases, runtime_moves, runtime_replacements, releases,
            active, lexical: Vec::new(),
        }))
    }
}

#[derive(Clone, Copy)]
struct GuardedScalarOwner<'ctx> {
    backing: PointerValue<'ctx>,
    present: inkwell::values::IntValue<'ctx>,
    runtime_extent: Option<inkwell::values::IntValue<'ctx>>,
    live_slot: Option<PointerValue<'ctx>>,
}

fn enroll_guarded_owner<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    index: usize,
    mut owner: GuardedScalarOwner<'ctx>,
) -> Result<(), String> {
    if state.guarded_scalar.as_ref().ok_or("missing guarded owner sites")?.sites[index].movable {
        let slot = entry_alloca(state, context.bool_type().into(), "fallible.owner_live")?;
        state.builder.build_store(slot, owner.present).map_err(builder_error)?;
        owner.live_slot = Some(slot);
    }
    let sites = state.guarded_scalar.as_mut().expect("prepared guarded sites");
    sites.active[index] = Some(owner);
    sites.lexical.push(index);
    Ok(())
}

pub(super) fn move_guarded_runtime_owner<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &crate::ScalarBinding,
) -> Result<bool, String> {
    let Some(sites) = state.guarded_scalar.as_ref().filter(|sites| !sites.runtime_moves.is_empty()) else { return Ok(false); };
    let [receiver] = binding.receivers.as_slice() else { return Ok(false); };
    let span = receiver.name_span;
    let Some(&(source, target, rhs)) = sites.runtime_moves.get(&(span.start, span.end)) else { return Ok(false); };
    let ScalarExpression::Name { name, .. } = &binding.value else { return Err("guarded move lost its exact lexical source".into()); };
    let nodes = state.body_call_nodes.ok_or("guarded move lacks exact prepared binding")?;
    let declaration = nodes.lexical_expression_declaration(&binding.value)
        .map(|span| (span.start, span.end));
    let owner = sites.active[source].ok_or("guarded move has no source receipt")?;
    if nodes.expression_ordinal(&binding.value) != Some(rhs) || nodes.binding_position(binding).is_none()
        || declaration.and_then(|span| sites.declarations.get(&span)) != Some(&source)
        || sites.active[target].is_some()
        || receiver.ty != sites.sites[target].ty
        || !matches!(state.storage.get(name), Some((backing, ty)) if *backing == owner.backing && *ty == sites.sites[source].ty)
    { return Err("guarded move differs from its checked operands".into()); }
    transfer_guarded_runtime_owner(context, state, source, target, &receiver.name, false)?;
    Ok(true)
}

pub(super) fn replace_guarded_runtime_owner<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &crate::ScalarAssignment,
) -> Result<bool, String> {
    let Some(sites) = state.guarded_scalar.as_ref().filter(|sites| !sites.runtime_replacements.is_empty()) else { return Ok(false); };
    let ([destination], [value]) = (assignment.targets.as_slice(), assignment.values.as_slice()) else { return Ok(false); };
    let nodes = state.body_call_nodes.ok_or("guarded replacement lacks prepared operands")?;
    let Some(&(source, target, rhs)) = nodes.place_ordinal(&destination.place)
        .and_then(|occurrence| sites.runtime_replacements.get(&occurrence)) else { return Ok(false); };
    let (crate::ScalarPlace::Name { name: destination_name, .. }, ScalarExpression::Name { name: source_name, .. }) =
        (&destination.place, value) else { return Err("guarded replacement changed its lexical places".into()); };
    let exact = |span: Option<ByteSpan>| span.and_then(|span| sites.declarations.get(&(span.start, span.end)));
    let valid = |index: usize, name: &str| sites.active.get(index).copied().flatten().is_some_and(|owner: GuardedScalarOwner<'_>|
        matches!(state.storage.get(name), Some((backing, ty)) if *backing == owner.backing && *ty == sites.sites[index].ty));
    if nodes.expression_ordinal(value) != Some(rhs)
        || exact(nodes.lexical_expression_declaration(value)) != Some(&source)
        || exact(nodes.lexical_existing_destination_span(&destination.place)) != Some(&target)
        || !valid(source, source_name) || !valid(target, destination_name)
    { return Err("guarded replacement differs from its checked operands".into()); }
    transfer_guarded_runtime_owner(context, state, source, target, destination_name, true)?;
    Ok(true)
}

fn transfer_guarded_runtime_owner<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    source: usize,
    target: usize,
    name: &str,
    replacement: bool,
) -> Result<(), String> {
    let sites = state.guarded_scalar.as_ref().ok_or("missing guarded transfer sites")?;
    let owner = sites.active[source].ok_or("guarded transfer has no source receipt")?;
    let previous = sites.active[target];
    if previous.is_some() != replacement || previous.is_some_and(|previous| previous.backing == owner.backing) {
        return Err("guarded transfer has no distinct destination receipt".into());
    }
    let extent = if sites.sites[source].runtime_returned { owner.runtime_extent }
        else { sites.sites[source].runtime_literal_length.map(|length| context.i64_type().const_int(length, false)) }
        .ok_or("guarded transfer lost the source extent")?;
    let live_slot = owner.live_slot.ok_or("guarded transfer lacks source lifetime state")?;
    let receiver_ty = sites.sites[target].ty.clone();
    // The exact lexical destination is retained once, and its previous
    // owner is consumed only after the already available descriptor installs.
    state.builder.build_store(live_slot, context.bool_type().const_zero()).map_err(builder_error)?;
    state.storage.insert(name.into(), (owner.backing, receiver_ty));
    let transferred = GuardedScalarOwner { backing: owner.backing,
        present: context.bool_type().const_all_ones(), runtime_extent: Some(extent), live_slot: None };
    if let Some(previous) = previous {
        state.guarded_scalar.as_mut().expect("prepared guarded sites").active[target] = Some(transferred);
        release_guarded_scalar_owner(context, state, previous)?;
    } else {
        enroll_guarded_owner(context, state, target, transferred)?;
    }
    Ok(())
}

enum GuardedAggregateInitializer<'ctx, 'module, 'literal> {
    Struct(memory::PreparedStructLiteral<'ctx, 'module>),
    Array(Vec<EmitValue<'ctx>>),
    Inline(Vec<memory::GuardedInlineWrite<'ctx, 'literal>>),
}

pub(super) fn bind_guarded_scalar<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &crate::ScalarBinding,
    value: Option<BasicValueEnum<'ctx>>,
    path: super::expression::ExpressionPath<'_>,
) -> Result<(), String> {
    let [receiver] = binding.receivers.as_slice() else {
        return Err("fallible scalar allocation requires a single receiver".into());
    };
    let sites = state.guarded_scalar.as_ref().ok_or("missing guarded scalar sites")?;
    let declaration = receiver.name_span;
    let &index = sites.declarations.get(&(declaration.start, declaration.end))
        .ok_or_else(|| format!("fallible scalar allocation has no prepared ownership for {} at {:?}",
            receiver.name, declaration))?;
    if receiver.ty != sites.sites[index].ty
        || sites.active[index].is_some()
    {
        return Err("invalid guarded scalar allocation binding".into());
    }
    if sites.sites[index].runtime_returned {
        let Some(BasicValueEnum::StructValue(result)) = value else { return Err("missing guarded owned array descriptor".into()); };
        if result.get_type() != super::aggregate_transfer::array_descriptor_type(context)
            || !state.aggregate.as_ref().is_some_and(|aggregate| state.body_call_nodes
                .and_then(|nodes| nodes.expression_ordinal(&binding.value))
                .is_some_and(|ordinal| aggregate.plan().array_descriptor_calls.contains(&ordinal)
                    || aggregate.plan().guarded_array_descriptor_choices.contains(&ordinal)))
        { return Err("invalid guarded owned array descriptor".into()); }
        let backing = state.builder.build_extract_value(result, 0, "guarded_array_backing")
            .map_err(builder_error)?.into_pointer_value();
        let extent = state.builder.build_extract_value(result, 1, "guarded_array_extent")
            .map_err(builder_error)?.into_int_value();
        let present = state.builder.build_is_not_null(backing, "fallible.present").map_err(builder_error)?;
        state.storage.insert(receiver.name.clone(), (backing, receiver.ty.clone()));
        return enroll_guarded_owner(context, state, index, GuardedScalarOwner {
            backing, present, runtime_extent: Some(extent), live_slot: None,
        });
    }
    let runtime_layout = match (&receiver.ty, sites.sites[index].runtime_literal_length) {
        (ScalarType::RuntimeArray { element, .. }, Some(length)) => {
            let (_, values, span) = crate::scalar::guarded_runtime_literal(binding)
                .ok_or("missing guarded runtime literal")?;
            if values.len() as u64 != length { return Err("guarded runtime literal lost its prepared extent".into()); }
            Some(ScalarType::Array { element: element.clone(), length, length_span: span, span })
        }
        (ScalarType::RuntimeArray { .. }, None) | (_, Some(_)) =>
            return Err("invalid guarded runtime extent receipt".into()),
        _ => None,
    };
    let materialized_ty = runtime_layout.as_ref().unwrap_or(&receiver.ty);
    let rhs = if super::aggregate_transfer::fallible_allocation_call_at(state, &binding.value)
        || super::aggregate_transfer::fallible_allocation_expression_at(state, &binding.value)
    {
        let Some(BasicValueEnum::StructValue(pair)) = value else {
            return Err("missing sealed fallible RHS result".into());
        };
        if pair.get_type() != super::aggregate_transfer::fallible_allocation_result_type(context, &receiver.ty)? {
            return Err("invalid sealed fallible RHS result".into());
        }
        Some(pair)
    } else { None };
    let parent = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or_else(|| "missing guarded scalar allocation function".to_owned())?;
    let mut member_failure = None;
    let mut emit_member = |state: &mut EmitState<'ctx, '_>, member: &ScalarExpression, ty: &ScalarType| {
        use super::expression::{ExpressionPath, emit_fallible_numeric_to};
        if super::aggregate_transfer::fallible_allocation_expression_at(state, member) {
            let failure = *member_failure.get_or_insert_with(||
                context.append_basic_block(parent, "fallible.field_absence"));
            let (value, _) = emit_fallible_numeric_to(context, state, member, ty, path, failure)?;
            Ok(EmitValue::Basic(value))
        } else {
            match (path, member) {
                (ExpressionPath::Project { module, modules },
                    ScalarExpression::Call { .. } | ScalarExpression::Binary { .. } | ScalarExpression::If { .. }
                | ScalarExpression::Unary { .. }) =>
                    super::expression::emit_project_typed_expression(context, state, member, ty, module, modules),
                _ => memory::emit_typed_expression(context, state, member, ty),
            }
        }
    };
    let aggregate = match (materialized_ty, &binding.value, value) {
        (ScalarType::Struct(id), ScalarExpression::StructLiteral { fields, .. }, None) => {
            if crate::scalar::has_inline_aggregate_field(fields)
            {
                Some(GuardedAggregateInitializer::Inline(
                    memory::stage_inline_fields(state, id, fields, &mut emit_member)?,
                ))
            } else {
                Some(GuardedAggregateInitializer::Struct(
                    memory::prepare_struct_literal_with(state, id, fields, emit_member)?,
                ))
            }
        }
        (ScalarType::Array { element, length, .. }, ScalarExpression::ArrayLiteral { elements, .. }, None)
            if *length == elements.len() as u64 && matches!(element.as_ref(), ScalarType::U64 | ScalarType::I32) =>
            Some(GuardedAggregateInitializer::Array(elements.iter().map(|value|
                emit_member(state, value, element)).collect::<Result<Vec<_>, _>>()?)),
        (_, _, Some(BasicValueEnum::StructValue(_))) if rhs.is_some() => None,
        (ty, _, Some(BasicValueEnum::IntValue(value)))
            if memory::integer_width(ty) == Some(value.get_type().get_bit_width())
                || (*ty == ScalarType::Bool && value.get_type().get_bit_width() == 1) => None,
        _ => return Err("invalid guarded allocation initializer".into()),
    };
    declare_core_runtime(context, state.module);
    let (size, align) = allocation_layout(materialized_ty, state.layouts, state.target_layout)?;
    let initialized = context.append_basic_block(parent, "fallible.initialize");
    let continuation = context.append_basic_block(parent, "fallible.continue");
    let skipped = if let Some(rhs) = rhs {
        let here = state.builder.get_insert_block().ok_or("missing fallible RHS block")?;
        let succeeded = state.builder.build_extract_value(rhs, 0, "fallible.rhs_succeeded")
            .map_err(builder_error)?.into_int_value();
        let allocate = context.append_basic_block(parent, "fallible.destination");
        state.builder.build_conditional_branch(succeeded, allocate, continuation).map_err(builder_error)?;
        state.builder.position_at_end(allocate);
        Some(here)
    } else if let Some(failure) = member_failure {
        let success = state.builder.get_insert_block().ok_or("missing successful aggregate members block")?;
        state.builder.position_at_end(failure);
        state.builder.build_unconditional_branch(continuation).map_err(builder_error)?;
        state.builder.position_at_end(success);
        Some(failure)
    } else { None };
    let allocation_end = state.builder.get_insert_block().ok_or("missing fallible allocation block")?;
    let backing = emit_standard_allocation_call(
        state, context.i64_type().const_int(size, false).into(),
        context.i64_type().const_int(align, false).into(),
    )?;
    let present = state.builder.build_is_not_null(backing, "fallible.present").map_err(builder_error)?;
    state.builder.build_conditional_branch(present, initialized, continuation).map_err(builder_error)?;
    state.builder.position_at_end(initialized);
    if let Some(values) = aggregate {
        match values {
            GuardedAggregateInitializer::Struct(values) =>
                memory::write_struct_literal(context, state, values, backing)?,
            GuardedAggregateInitializer::Array(values) =>
                memory::write_fixed_array_literal(context, state, materialized_ty, values, backing)?,
            GuardedAggregateInitializer::Inline(values) =>
                memory::write_guarded_inline(context, state, values, backing)?,
        }
    } else {
        let value = if let Some(rhs) = rhs {
            state.builder.build_extract_value(rhs, 1, "fallible.rhs_payload").map_err(builder_error)?
        } else {
            value.expect("checked integer initializer")
        };
        state.builder.build_store(backing, value).map_err(builder_error)?;
    }
    state.builder.build_unconditional_branch(continuation).map_err(builder_error)?;
    state.builder.position_at_end(continuation);
    let (backing, present) = if let Some(skipped) = skipped {
        let address = state.builder.build_phi(backing.get_type(), "fallible.backing").map_err(builder_error)?;
        address.add_incoming(&[(&backing, allocation_end), (&backing, initialized),
            (&backing.get_type().const_null(), skipped)]);
        let availability = state.builder.build_phi(context.bool_type(), "fallible.available").map_err(builder_error)?;
        availability.add_incoming(&[(&context.bool_type().const_zero(), allocation_end),
            (&context.bool_type().const_all_ones(), initialized), (&context.bool_type().const_zero(), skipped)]);
        (address.as_basic_value().into_pointer_value(), availability.as_basic_value().into_int_value())
    } else { (backing, present) };
    state.storage.insert(receiver.name.clone(), (backing, receiver.ty.clone()));
    enroll_guarded_owner(context, state, index, GuardedScalarOwner {
        backing, present, runtime_extent: None, live_slot: None,
    })
}

pub(super) fn guarded_array_extent<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    base: &crate::ScalarPlace,
    backing: PointerValue<'ctx>,
    ty: &ScalarType,
) -> Result<Option<inkwell::values::IntValue<'ctx>>, String> {
    let Some(sites) = &state.guarded_scalar else { return Ok(None); };
    let Some(declaration) = state.body_call_nodes.and_then(|nodes| nodes.lexical_place_declaration(base))
    else { return Ok(None); };
    let Some(&index) = sites.declarations.get(&(declaration.start, declaration.end)) else { return Ok(None); };
    let site = &sites.sites[index];
    if !matches!(site.ty, ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }) { return Ok(None); }
    let owner = sites.active[index].filter(|owner| owner.backing == backing && &site.ty == ty)
        .ok_or("guarded array access lost its exact prepared owner")?;
    let extent = match &site.ty {
        ScalarType::Array { length, .. } => context.i64_type().const_int(*length, false),
        ScalarType::RuntimeArray { .. } if site.runtime_returned =>
            owner.runtime_extent.ok_or("missing guarded returned array extent")?,
        ScalarType::RuntimeArray { .. } => context.i64_type()
            .const_int(site.runtime_literal_length.ok_or("missing guarded runtime literal extent")?, false),
        _ => unreachable!("checked guarded array type"),
    };
    Ok(Some(extent))
}

pub(super) fn guarded_alias_has_non_null_proof(
    state: &EmitState<'_, '_>,
    pointer: &ScalarExpression,
) -> bool {
    let Some(sites) = &state.guarded_scalar else { return false };
    let Some(operand) = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(pointer))
        else { return false };
    sites.aliases.get(&operand).and_then(|index| sites.active.get(*index))
        .is_some_and(Option::is_some)
}

pub(super) fn guarded_scalar_condition<'ctx>(
    state: &EmitState<'ctx, '_>,
    expression: &ScalarExpression,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(sites) = &state.guarded_scalar else { return Ok(None) };
    let Some(condition) = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
        else { return Ok(None) };
    let Some(&(index, present_on_then)) = sites.conditions.get(&condition) else { return Ok(None) };
    let owner = sites.active.get(index).and_then(Option::as_ref)
        .ok_or_else(|| "guarded scalar condition precedes its allocation".to_owned())?;
    let condition = if present_on_then {
        owner.present
    } else {
        state.builder.build_not(owner.present, "fallible.absent").map_err(builder_error)?
    };
    Ok(Some(EmitValue::Basic(condition.into())))
}

pub(super) fn release_guarded_owner_at<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    span: ByteSpan,
    arguments: &[ScalarExpression],
    candidates: &[crate::scalar::InvalidationCandidate],
) -> Result<bool, String> {
    let Some(sites) = state.guarded_scalar.as_ref().filter(|sites| !sites.releases.is_empty()) else { return Ok(false); };
    let Some(&index) = sites.releases.get(&(span.start, span.end)) else { return Ok(false); };
    let [ScalarExpression::CheckedAddress { place: place @ crate::ScalarPlace::Name { name, .. }, .. }] = arguments
        else { return Err("guarded release lost its exact lexical address".into()); };
    let [candidate] = candidates else { return Err("guarded release has no singular owner".into()); };
    let declaration = state.body_call_nodes.and_then(|nodes| nodes.lexical_place_declaration(place))
        .ok_or("guarded release has no prepared lexical root")?;
    let owner = sites.active[index].ok_or("guarded release has no active owner")?;
    if candidate.place.binding.declaration_span != declaration || !candidate.place.projections.is_empty()
        || sites.declarations.get(&(declaration.start, declaration.end)) != Some(&index)
        || !matches!(state.storage.get(name), Some((backing, ty))
            if *backing == owner.backing && *ty == sites.sites[index].ty)
    { return Err("guarded release differs from its checked backing".into()); }
    let live = owner.live_slot.ok_or("guarded release lacks its lifetime slot")?;
    // Presence and all conflicting loans were resolved on this exact edge.
    // The explicit release consumes the original owner, without a second check.
    state.builder.build_store(live, context.bool_type().const_zero()).map_err(builder_error)?;
    consume_runtime_array_release(context, state,
        RuntimeArrayRelease { pointer: owner.backing.into(), live: None })?;
    Ok(true)
}

pub(super) fn release_guarded_scalar<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    scope: ByteSpan,
) -> Result<(), String> {
    loop {
        let Some(sites) = state.guarded_scalar.as_mut() else { break };
        let Some(&index) = sites.lexical.last() else { break };
        if sites.sites[index].scope != scope { break; }
        sites.lexical.pop();
        let owner = sites.active[index].take().ok_or("missing active fallible cleanup owner")?;
        release_guarded_scalar_owner(context, state, owner)?;
    }
    Ok(())
}

fn release_guarded_scalar_owner<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    owner: GuardedScalarOwner<'ctx>,
) -> Result<(), String> {
    let live = if let Some(slot) = owner.live_slot {
        state.builder.build_load(context.bool_type(), slot, "fallible.owner_live")
            .map_err(builder_error)?.into_int_value()
    } else { owner.present };
    if live.get_zero_extended_constant() == Some(1) {
        return consume_runtime_array_release(context, state, RuntimeArrayRelease { pointer: owner.backing.into(), live: None });
    }
    let parent = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or_else(|| "missing guarded scalar release function".to_owned())?;
    let release = state.module.get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    let present = context.append_basic_block(parent, "fallible.release");
    let next = context.append_basic_block(parent, "fallible.released");
    state.builder.build_conditional_branch(live, present, next).map_err(builder_error)?;
    state.builder.position_at_end(present);
    state.builder.build_call(release, &[owner.backing.into()], "").map_err(builder_error)?;
    state.builder.build_unconditional_branch(next).map_err(builder_error)?;
    state.builder.position_at_end(next);
    Ok(())
}

pub(super) fn emit_runtime_array_allocation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    emit_runtime_array_allocation_with_domain(context, state, element, length, false)
}

pub(super) fn emit_runtime_array_allocation_with_domain<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
    checked_extent: bool,
) -> Result<PointerValue<'ctx>, String> {
    emit_runtime_array_allocation_with_absence(context, state, element, length, checked_extent, None)
        .map(|allocation| allocation.backing)
}

pub(super) struct RuntimeArrayConstruction<'ctx> {
    pub(super) backing: PointerValue<'ctx>,
    pub(super) length: inkwell::values::IntValue<'ctx>,
    pub(super) signed: Option<super::memory::SignedRuntimeAddressDomain>,
}

pub(super) fn emit_runtime_array_allocation_with_absence<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
    checked_extent: bool,
    relay_unwind: Option<inkwell::basic_block::BasicBlock<'ctx>>,
) -> Result<RuntimeArrayConstruction<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let (element_size, alignment) = allocation_layout(element, state.layouts, state.target_layout)?;
    let function = state
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .ok_or_else(|| "runtime allocation has no containing function".to_owned())?;
    let guarded_array = state.aggregate.as_ref().is_some_and(|aggregate| aggregate.plan().guarded_array_descriptor_leaf);
    let recover = relay_unwind.is_none()
        && (guarded_array || state.aggregate.as_ref().is_some_and(|aggregate| aggregate.plan().fallible_allocation_leaf));
    let integer = context.i64_type();
    // The checked extent also guarantees the target's signed LLVM pointer
    // index domain. Reuse the constructor's existing size-overflow branch
    // rather than repeating a size/range check at each access.
    let signed = checked_extent.then(|| super::memory::SignedRuntimeAddressDomain::for_construction(
        state.target_layout, element,
    )).transpose()?;
    let max_bytes = signed.map_or(u64::MAX, |domain| domain.max_bytes());
    let limit = (element_size != 0).then(|| max_bytes / element_size).filter(|&limit|
        limit != u64::MAX && !length.get_zero_extended_constant().is_some_and(|constant| constant <= limit));
    let panic_block = ((!recover && relay_unwind.is_none()) || limit.is_some()).then(|| context.append_basic_block(function, "allocation_panic"));
    if let Some(limit) = limit {
        let overflow = state.builder.build_int_compare(
            IntPredicate::UGT, length, integer.const_int(limit, false), "allocation_size_overflow",
        ).map_err(builder_error)?;
        let allocate = context.append_basic_block(function, "allocation_size_valid");
        state.builder.build_conditional_branch(overflow, panic_block.expect("overflow panic block"), allocate)
            .map_err(builder_error)?;
        state.builder.position_at_end(allocate);
    }
    let size = match element_size {
        0 => integer.const_zero(),
        1 => length,
        _ => state.builder.build_int_nuw_mul(
            length, integer.const_int(element_size, false), "allocation_size",
        ).map_err(builder_error)?,
    };
    let continue_block = context.append_basic_block(function, "allocation_continue");
    let allocation_failure = if let Some(unwind) = relay_unwind {
        unwind
    } else if recover {
        context.append_basic_block(function, "allocation_absent")
    } else {
        panic_block.expect("ordinary allocation panic block")
    };
    let allocation = emit_managed_allocation_branch(
        context, state, size, alignment, allocation_failure, continue_block,
    )?;
    if recover {
        state.builder.position_at_end(allocation_failure);
        let absent = if guarded_array { super::aggregate_transfer::array_descriptor_type(context).const_zero() }
            else { sealed_absence_value(context, state)? };
        state.builder.build_return(Some(&absent)).map_err(builder_error)?;
    }
    if let Some(panic_block) = panic_block {
        state.builder.position_at_end(panic_block);
        let panic = state.module.get_function(CORE_SYSTEM_PANIC_SYMBOL)
            .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
        state.builder.build_call(panic, &[], "system_panic").map_err(builder_error)?;
        state.builder.build_unreachable().map_err(builder_error)?;
    }
    state.builder.position_at_end(continue_block);
    Ok(RuntimeArrayConstruction { backing: allocation, length, signed })
}

fn relay_failure_block<'ctx>(
    context: &'ctx Context, state: &EmitState<'ctx, '_>, name: &str,
) -> Result<(inkwell::basic_block::BasicBlock<'ctx>, inkwell::basic_block::BasicBlock<'ctx>), String> {
    let continuation = state.builder.get_insert_block().ok_or("missing sealed relay continuation")?;
    let function = continuation.get_parent().ok_or("missing sealed relay function")?;
    Ok((continuation, context.append_basic_block(function, name)))
}

fn sealed_absence_value<'ctx>(
    context: &'ctx Context, state: &EmitState<'ctx, '_>,
) -> Result<inkwell::values::StructValue<'ctx>, String> {
    let payload = state.aggregate.as_ref().and_then(|aggregate| aggregate.plan().fallible_allocation_payload.as_ref())
        .ok_or("missing sealed fallible allocation absence payload")?;
    Ok(super::aggregate_transfer::fallible_allocation_result_type(context, payload)?.const_zero())
}

pub(super) fn emit_relay_absence<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
) -> Result<inkwell::basic_block::BasicBlock<'ctx>, String> {
    let (continuation, absent) = relay_failure_block(context, state, "allocation_absent")?;
    state.builder.position_at_end(absent);
    let value = sealed_absence_value(context, state)?;
    state.builder.build_return(Some(&value)).map_err(builder_error)?;
    state.builder.position_at_end(continuation);
    Ok(absent)
}

pub(super) fn emit_relay_unwind<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>, pointer: PointerValue<'ctx>,
    prior: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<inkwell::basic_block::BasicBlock<'ctx>, String> {
    let (continuation, unwind) = relay_failure_block(context, state, "allocation_unwind")?;
    state.builder.position_at_end(unwind);
    consume_runtime_array_release(context, state, RuntimeArrayRelease { pointer: pointer.into(), live: None })?;
    state.builder.build_unconditional_branch(prior).map_err(builder_error)?;
    state.builder.position_at_end(continuation);
    Ok(unwind)
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
