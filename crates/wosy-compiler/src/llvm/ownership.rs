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
        ) || super::aggregate_transfer::holds_registered_return(state)
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

pub(super) struct GuardedScalarSite {
    declaration: ByteSpan,
    scope: ByteSpan,
    ty: ScalarType,
    condition: usize,
    present_on_then: bool,
}

impl GuardedScalarSite {
    pub(super) fn all_from_cfg(cfg: &ScalarReferenceCfg) -> Option<Box<[Self]>> {
        let proofs = cfg.guarded_scalar_allocations();
        if proofs.is_empty() {
            return None;
        }
        proofs.iter().map(|proof| {
            let point = cfg.points.get(proof.branch().0)?;
            if !matches!(point.kind, crate::scalar::CfgPointKind::Branch { .. }) {
                return None;
            }
            Some(Self {
                declaration: proof.binding().declaration_span,
                scope: proof.binding().block_span,
                ty: proof.ty().clone(),
                condition: proof.condition(),
                present_on_then: proof.present_on_then(),
            })
        }).collect()
    }
}

pub(super) struct GuardedScalarOwner<'ctx> {
    site: usize,
    backing: PointerValue<'ctx>,
    present: inkwell::values::IntValue<'ctx>,
}

pub(super) fn bind_guarded_scalar<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &crate::ScalarBinding,
    value: Option<BasicValueEnum<'ctx>>,
) -> Result<(), String> {
    let [receiver] = binding.receivers.as_slice() else {
        return Err("fallible scalar allocation requires a single receiver".into());
    };
    let (index, site) = state.guarded_scalar.as_deref().unwrap_or_default().iter().enumerate()
        .find(|(_, site)| site.declaration == receiver.name_span)
        .ok_or_else(|| format!(
            "fallible scalar allocation has no prepared ownership for {} at {:?}", receiver.name, receiver.name_span,
        ))?;
    if receiver.ty != site.ty || state.guarded_scalar_owner.is_some() {
        return Err("invalid guarded scalar allocation binding".into());
    }
    let aggregate = match (&receiver.ty, &binding.value, value) {
        (ScalarType::Struct(id), ScalarExpression::StructLiteral { fields, .. }, None) =>
            Some(memory::prepare_struct_literal(context, state, id, fields)?),
        (ty, _, Some(BasicValueEnum::IntValue(value)))
            if memory::integer_width(ty) == Some(value.get_type().get_bit_width()) => None,
        _ => return Err("invalid guarded allocation initializer".into()),
    };
    declare_core_runtime(context, state.module);
    let (size, align) = allocation_layout(&receiver.ty, state.layouts, state.target_layout)?;
    let parent = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or_else(|| "missing guarded scalar allocation function".to_owned())?;
    let initialized = context.append_basic_block(parent, "fallible.initialize");
    let continuation = context.append_basic_block(parent, "fallible.continue");
    let backing = emit_standard_allocation_call(
        state, context.i64_type().const_int(size, false).into(),
        context.i64_type().const_int(align, false).into(),
    )?;
    let present = state.builder.build_is_not_null(backing, "fallible.present").map_err(builder_error)?;
    state.builder.build_conditional_branch(present, initialized, continuation).map_err(builder_error)?;
    state.builder.position_at_end(initialized);
    if let Some(values) = aggregate {
        memory::write_struct_literal(context, state, values, backing)?;
    } else {
        state.builder.build_store(backing, value.expect("checked integer initializer")).map_err(builder_error)?;
    }
    state.builder.build_unconditional_branch(continuation).map_err(builder_error)?;
    state.builder.position_at_end(continuation);
    state.storage.insert(receiver.name.clone(), (backing, receiver.ty.clone()));
    state.guarded_scalar_owner = Some(GuardedScalarOwner { site: index, backing, present });
    Ok(())
}

pub(super) fn guarded_scalar_condition<'ctx>(
    state: &EmitState<'ctx, '_>,
    expression: &ScalarExpression,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(sites) = &state.guarded_scalar else { return Ok(None) };
    let condition = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression));
    let Some((index, site)) = sites.iter().enumerate().find(|(_, site)| condition == Some(site.condition))
        else { return Ok(None) };
    let owner = state.guarded_scalar_owner.as_ref()
        .filter(|owner| owner.site == index)
        .ok_or_else(|| "guarded scalar condition precedes its allocation".to_owned())?;
    let condition = if site.present_on_then {
        owner.present
    } else {
        state.builder.build_not(owner.present, "fallible.absent").map_err(builder_error)?
    };
    Ok(Some(EmitValue::Basic(condition.into())))
}

pub(super) fn release_guarded_scalar<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    scope: ByteSpan,
) -> Result<(), String> {
    let Some(owner) = state.guarded_scalar_owner.as_ref() else { return Ok(()) };
    if state.guarded_scalar.as_deref().and_then(|sites| sites.get(owner.site)).map(|site| site.scope) != Some(scope) {
        return Ok(());
    }
    let owner = state.guarded_scalar_owner.take().expect("matched guarded ownership");
    let parent = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or_else(|| "missing guarded scalar release function".to_owned())?;
    let release = state.module.get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    let present = context.append_basic_block(parent, "fallible.release");
    let next = context.append_basic_block(parent, "fallible.released");
    state.builder.build_conditional_branch(owner.present, present, next).map_err(builder_error)?;
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
    declare_core_runtime(context, state.module);
    let (element_size, alignment) = allocation_layout(element, state.layouts, state.target_layout)?;
    let function = state
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .ok_or_else(|| "runtime allocation has no containing function".to_owned())?;
    let panic_block = context.append_basic_block(function, "allocation_panic");
    let integer = context.i64_type();
    // The checked extent also guarantees the target's signed LLVM pointer
    // index domain. Reuse the constructor's existing size-overflow branch
    // rather than repeating a size/range check at each access.
    let max_bytes = if checked_extent {
        match state.target_layout.pointer_size {
            4 => i32::MAX as u64,
            8 => i64::MAX as u64,
            _ => return Err("unsupported runtime-array pointer index width".to_owned()),
        }
    } else {
        u64::MAX
    };
    if element_size != 0 {
        let limit = max_bytes / element_size;
        if limit != u64::MAX && !length.get_zero_extended_constant().is_some_and(|constant| constant <= limit) {
            let overflow = state.builder.build_int_compare(
                IntPredicate::UGT, length, integer.const_int(limit, false), "allocation_size_overflow",
            ).map_err(builder_error)?;
            let allocate = context.append_basic_block(function, "allocation_size_valid");
            state.builder.build_conditional_branch(overflow, panic_block, allocate)
                .map_err(builder_error)?;
            state.builder.position_at_end(allocate);
        }
    }
    let size = match element_size {
        0 => integer.const_zero(),
        1 => length,
        _ => state.builder.build_int_nuw_mul(
            length, integer.const_int(element_size, false), "allocation_size",
        ).map_err(builder_error)?,
    };
    let continue_block = context.append_basic_block(function, "allocation_continue");
    let allocation = emit_managed_allocation_branch(
        context, state, size, alignment, panic_block, continue_block,
    )?;
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
