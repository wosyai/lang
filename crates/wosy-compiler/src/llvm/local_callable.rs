use super::*;
use crate::ScalarBinding;

pub(super) struct LocalCallableSlot<'ctx> {
    storage: PointerValue<'ctx>,
    pub(super) parameters: Vec<ScalarType>,
    pub(super) outputs: crate::ScalarOutputSequence,
}

fn scalar(ty: &ScalarType) -> bool {
    integer_width(ty).is_some()
        || matches!(ty, ScalarType::Bool | ScalarType::Char | ScalarType::Enum(_)
            | ScalarType::F32 | ScalarType::F64)
}

fn signature(ty: &ScalarType) -> Option<(&[ScalarType], &crate::ScalarOutputSequence)> {
    let ScalarType::Callable { parameters, outputs } = ty else { return None };
    let [output] = outputs.outputs.as_slice() else { return None };
    let scalar_parameters = parameters.iter().all(scalar);
    let shared_reader = matches!(parameters.as_slice(), [ScalarType::CheckedReference {
        mutability: crate::scalar::ScalarReferenceMutability::Shared, inner,
    }] if scalar(inner) && &output.ty == inner.as_ref());
    (scalar(&output.ty) && (scalar_parameters || shared_reader)).then_some((parameters, outputs))
}

pub(super) fn closed_scalar_target(function: &ScalarFunction, cfg: &ScalarReferenceCfg) -> bool {
    if signature(&function.signature).is_none_or(|(parameters, _)| !parameters.iter().all(scalar))
        || !cfg.allocations.is_empty()
        || !matches!(cfg.points.first().map(|point| &point.kind), Some(CfgPointKind::Entry))
    {
        return false;
    }
    // No call, external place or reference effect is admitted through the
    // local indirect route. Scalar computation only touches its own inputs.
    let parameter_spans: std::collections::HashSet<_> = function.parameter_spans.iter().copied().collect();
    let parameter = |binding: &crate::scalar::ReferenceBindingId| {
        binding.function_span == function.span && parameter_spans.contains(&binding.declaration_span)
    };
    let mut returned = false;
    cfg.points.iter().all(|point| match &point.kind {
        CfgPointKind::Entry | CfgPointKind::ScopeEnter | CfgPointKind::ScopeExit
        | CfgPointKind::Return | CfgPointKind::Branch { .. } | CfgPointKind::Join => true,
        CfgPointKind::ReturnOutput { output: 0, checked: false, .. } if !returned => {
            returned = true;
            true
        }
        CfgPointKind::Bind { binding } | CfgPointKind::Read { binding } => parameter(binding),
        CfgPointKind::Evaluate(expression) => matches!(expression,
            ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }
            | ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }
            | ScalarExpression::Binary { operator: BinaryOperator::Add | BinaryOperator::Subtract
                | BinaryOperator::Multiply | BinaryOperator::Equal | BinaryOperator::NotEqual, .. }
            | ScalarExpression::Unary { .. }),
        _ => false,
    }) && returned
}

fn target<'ctx>(
    state: &EmitState<'ctx, '_>, expression: &ScalarExpression,
    parameters: &[ScalarType], outputs: &crate::ScalarOutputSequence,
) -> Result<PointerValue<'ctx>, String> {
    let symbol = state.closed_local_callable_symbol(expression)
        .ok_or_else(|| "local callable slot requires a proved direct scalar target".to_owned())?;
    let definition = state.functions.get(symbol).ok_or_else(|| state.missing_prepared_signature(symbol))?;
    let (actual_parameters, actual_outputs) = state.signatures.get(symbol).and_then(signature)
        .ok_or_else(|| state.missing_prepared_signature(symbol))?;
    if actual_parameters != parameters
        || actual_outputs.outputs[0].ty != outputs.outputs[0].ty
        || definition.count_params() as usize != parameters.len()
        || state.dynamic_owner_outputs.get(symbol).is_some_and(|outputs| !outputs.is_empty())
    {
        return Err("local callable target changes the selected scalar ABI".into());
    }
    Ok(definition.as_global_value().as_pointer_value())
}

pub(super) fn binding_value<'ctx>(
    state: &EmitState<'ctx, '_>, binding: &ScalarBinding,
) -> Result<Option<EmitValue<'ctx>>, String> {
    if !state.prepared_render || state.body_call_nodes.is_none()
        || !matches!(binding.declared_type, ScalarType::Callable { .. })
    {
        return Ok(None);
    }
    let (parameters, outputs) = signature(&binding.declared_type)
        .ok_or_else(|| "unsupported local callable slot signature".to_owned())?;
    Ok(Some(EmitValue::Basic(target(state, &binding.value, parameters, outputs)?.into())))
}

pub(super) fn store_binding<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding, value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    if !state.prepared_render || state.body_call_nodes.is_none()
        || !matches!(binding.declared_type, ScalarType::Callable { .. })
    {
        return Ok(false);
    }
    let [receiver] = binding.receivers.as_slice() else {
        return Err("local callable slot requires one declared receiver".into());
    };
    let (parameters, outputs) = signature(&receiver.ty)
        .ok_or_else(|| "unsupported local callable slot signature".to_owned())?;
    let EmitValue::Basic(BasicValueEnum::PointerValue(required)) = value else {
        return Err("missing proved local callable initializer".into());
    };
    let storage = entry_alloca(state, context.ptr_type(AddressSpace::default()).into(), "local_callable")?;
    state.builder.build_store(storage, *required).map_err(builder_error)?;
    let key = (receiver.name_span.start, receiver.name_span.end);
    if state.local_callable_slots.insert(key, LocalCallableSlot {
        storage, parameters: parameters.to_vec(), outputs: outputs.clone(),
    }).is_some() {
        return Err("duplicate local callable slot".into());
    }
    Ok(true)
}

pub(super) fn replace<'ctx>(
    state: &mut EmitState<'ctx, '_>, assignment: &ScalarAssignment,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(nodes) = state.body_call_nodes.filter(|_| !state.local_callable_slots.is_empty()) else {
        return Ok(None);
    };
    let selected = assignment.targets.iter().find_map(|target| {
        let span = nodes.local_callable_destination_span(&target.place)?;
        state.local_callable_slots.get(&(span.start, span.end))
    });
    let Some(slot) = selected else { return Ok(None) };
    let [expression] = assignment.values.as_slice() else {
        return Err("local callable replacement requires one direct value".into());
    };
    if assignment.targets.len() != 1 || !matches!(assignment.targets[0].place, crate::ScalarPlace::Name { .. }) {
        return Err("local callable replacement requires one lexical destination".into());
    }
    let pointer = target(state, expression, &slot.parameters, &slot.outputs)?;
    state.builder.build_store(slot.storage, pointer).map_err(builder_error)?;
    Ok(Some(EmitValue::Unit))
}

pub(super) fn invoke<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, key: (u32, u32),
    arguments: &[ScalarExpression], project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let slot = state.local_callable_slots.get(&key).ok_or_else(|| "missing local callable slot".to_owned())?;
    let (storage, parameters, outputs) = (slot.storage, slot.parameters.clone(), slot.outputs.clone());
    if arguments.len() != parameters.len() {
        return Err("local callable slot argument count differs from its signature".into());
    }
    let types = parameters.iter().map(|ty| basic_type(context, ty, state.target_layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum<'ctx>>, _>>()?;
    let ty = basic_type(context, &outputs.outputs[0].ty, state.target_layout)?.fn_type(&types, false);
    let pointer = state.builder.build_load(context.ptr_type(AddressSpace::default()), storage, "local_callable_target")
        .map_err(builder_error)?.into_pointer_value();
    let values = super::expression::emit_contextual_arguments(context, state, arguments, &parameters, project)?
        .into_iter().map(|value| take_basic(value).map(Into::into)).collect::<Result<Vec<_>, _>>()?;
    let call = state.builder.build_indirect_call(ty, pointer, &values, "local_callable_call").map_err(builder_error)?;
    state.call_owner_flags.clear();
    super::expression::ordinary_call_result(call.try_as_basic_value(), &outputs)
}
