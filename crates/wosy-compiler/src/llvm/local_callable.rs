use super::*;
use crate::ScalarBinding;

pub(super) struct LocalCallableSlot<'ctx> {
    storage: PointerValue<'ctx>,
    pub(super) parameters: Vec<ScalarType>,
    pub(super) outputs: crate::ScalarOutputSequence,
}

/// A private array of the same capture-free pointers as lexical callable slots.
pub(super) struct LocalCallableArray<'ctx> {
    storage: PointerValue<'ctx>,
    llvm_type: inkwell::types::ArrayType<'ctx>,
    length: u64,
    element: ScalarType,
}

fn array_element(ty: &ScalarType) -> Option<(&ScalarType, u64)> {
    let ScalarType::Array { element, length, .. } = ty else { return None; };
    matches!(element.as_ref(), ScalarType::Callable { .. }).then_some((element.as_ref(), *length))
}

fn scalar_only_signature(ty: &ScalarType) -> Option<(&[ScalarType], &crate::ScalarOutputSequence)> {
    signature(ty).filter(|(parameters, _)| parameters.iter().all(scalar))
}

fn same_scalar_signature(left: &ScalarType, right: &ScalarType) -> bool {
    scalar_only_signature(left).zip(scalar_only_signature(right)).is_some_and(
        |((left_parameters, left_outputs), (right_parameters, right_outputs))|
            left_parameters == right_parameters
                && left_outputs.outputs[0].ty == right_outputs.outputs[0].ty)
}

fn array_binding<'a>(
    state: &EmitState<'_, '_>, binding: &'a ScalarBinding,
) -> Result<(&'a crate::scalar::ScalarOutputReceiver, &'a ScalarType, u32), String> {
    let Some((element, length)) = array_element(&binding.declared_type) else {
        return Err("missing fixed callable-array element type".into());
    };
    let [receiver] = binding.receivers.as_slice() else { return Err("fixed callable array requires one receiver".into()); };
    if !state.prepared_render || !state.require_release_checked_access || binding.is_allocation
        || binding.fallible_assignment.is_some() || !array_element(&receiver.ty)
            .is_some_and(|(ty, actual)| actual == length && same_scalar_signature(ty, element))
        || state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding)).is_none()
        || scalar_only_signature(element).is_none()
    {
        return Err("fixed callable array requires a prepared lexical scalar-only declaration".into());
    }
    let bytes = length.checked_mul(state.target_layout.pointer_size).ok_or("fixed callable array size overflows")?;
    memory::check_fixed_address_domain(state.target_layout, bytes)?;
    Ok((receiver, element, u32::try_from(length).map_err(|_| "fixed callable array exceeds LLVM array count")?))
}

fn selected_array<'a, 'ctx>(
    state: &'a EmitState<'ctx, '_>, place: &crate::ScalarPlace,
) -> Option<&'a LocalCallableArray<'ctx>> {
    let nodes = state.body_call_nodes?;
    let root = match place {
        crate::ScalarPlace::Index { base, .. } => base,
        other => other,
    };
    state.local_callable_arrays.get(&nodes.lexical_place_declaration(root)?)
}

fn selected_pointer<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<PointerValue<'ctx>, String> {
    let crate::ScalarPlace::Index { base, index, .. } = place else {
        return Err("fixed callable array requires a direct indexed element".into());
    };
    let nodes = state.body_call_nodes.ok_or("fixed callable element lacks prepared nodes")?;
    let selected = selected_array(state, place).ok_or("fixed callable element lacks its lexical array")?;
    if !matches!(base.as_ref(), crate::ScalarPlace::Name { .. })
        || nodes.place_ordinal(place).is_none()
        || !nodes.checked_place_type(base).and_then(array_element)
            .is_some_and(|(element, length)| same_scalar_signature(element, &selected.element) && length == selected.length)
        || !nodes.checked_place_type(place).is_some_and(|ty| same_scalar_signature(ty, &selected.element))
    {
        return Err("fixed callable array changed its checked place or element".into());
    }
    let (storage, array, length) = (selected.storage, selected.llvm_type, selected.length);
    let selected = memory::emit_place_index(context, state, index, project)?;
    memory::check_fixed_index(context, state, selected, length, None)?;
    memory::fixed_array_storage_element_pointer(context, state, array.into(), storage, selected)
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

pub(crate) fn closed_scalar_target(function: &ScalarFunction, cfg: &ScalarReferenceCfg) -> bool {
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
    parameters: &[ScalarType], outputs: &crate::ScalarOutputSequence, allow_reader: bool,
) -> Result<PointerValue<'ctx>, String> {
    let symbol = (if allow_reader { state.closed_local_callable_symbol(expression) }
        else { state.closed_local_scalar_callable_symbol(expression) })
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

pub(super) fn binding_value<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, binding: &ScalarBinding,
    project: Option<ProjectCallScope<'_>>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    if array_element(&binding.declared_type).is_some() {
        let (_, element, length) = array_binding(state, binding)?;
        let ScalarExpression::ArrayLiteral { elements, .. } = &binding.value else {
            return Err("fixed callable array requires complete literal construction".into());
        };
        if elements.len() != length as usize {
            return Err("fixed callable array literal count differs from its extent".into());
        }
        let (parameters, outputs) = scalar_only_signature(element).ok_or("invalid fixed callable array signature")?;
        let resolved = elements.iter().map(|expression| target(state, expression, parameters, outputs, false))
            .collect::<Result<Vec<_>, _>>()?;
        let array = context.ptr_type(AddressSpace::default()).const_array(&resolved);
        return Ok(Some(EmitValue::Basic(array.into())));
    }
    if !state.prepared_render || state.body_call_nodes.is_none()
        || !matches!(binding.declared_type, ScalarType::Callable { .. })
    {
        return Ok(None);
    }
    let (parameters, outputs) = signature(&binding.declared_type)
        .ok_or_else(|| "unsupported local callable slot signature".to_owned())?;
    if let ScalarExpression::IndexedRead { place, .. } = &binding.value {
        let nodes = state.body_call_nodes.expect("prepared local callable receiver");
        if binding.fallible_assignment.is_some() || binding.receivers.len() != 1
            || nodes.binding_position(binding).is_none()
            || !nodes.checked_place_type(place).is_some_and(|ty| same_scalar_signature(ty, &binding.declared_type))
        {
            return Err("indexed callable requires one exact typed lexical receiver".into());
        }
        let pointer = selected_pointer(context, state, place, project)?;
        let value = state.builder.build_load(context.ptr_type(AddressSpace::default()), pointer, "selected_callable")
            .map_err(builder_error)?;
        return Ok(Some(EmitValue::Basic(value)));
    }
    Ok(Some(EmitValue::Basic(target(state, &binding.value, parameters, outputs, true)?.into())))
}

pub(super) fn store_binding<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding, value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    if array_element(&binding.declared_type).is_some() {
        let (receiver, element, length) = array_binding(state, binding)?;
        let EmitValue::Basic(BasicValueEnum::ArrayValue(array)) = value else {
            return Err("fixed callable array lacks fully initialized storage".into());
        };
        let llvm_type = context.ptr_type(AddressSpace::default()).array_type(length);
        if array.get_type() != llvm_type { return Err("fixed callable array changed its physical extent".into()); }
        let storage = entry_alloca(state, llvm_type.into(), "local_callable_array")?;
        state.builder.build_store(storage, *array).map_err(builder_error)?;
        if state.local_callable_arrays.insert(receiver.name_span, LocalCallableArray {
            storage, llvm_type, length: length.into(), element: element.clone(),
        }).is_some() { return Err("duplicate fixed callable array declaration".into()); }
        return Ok(true);
    }
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

pub(super) fn replace<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, assignment: &ScalarAssignment,
    project: Option<ProjectCallScope<'_>>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    if !state.local_callable_arrays.is_empty() {
        if let Some(destination) = assignment.targets.iter()
            .find(|destination| selected_array(state, &destination.place).is_some())
        {
            let [expression] = assignment.values.as_slice() else {
                return Err("fixed callable array update requires one direct target".into());
            };
            if assignment.targets.len() != 1 || !matches!(destination.place, crate::ScalarPlace::Index { .. }) {
                return Err("fixed callable array cannot be rebound or included in a compound write".into());
            }
            let nodes = state.body_call_nodes.ok_or("fixed callable update lacks prepared nodes")?;
            if !nodes.is_callable_array_statement(assignment)
                || assignment.receiver != destination.receiver || assignment.receiver_span != destination.receiver_span
                || project.is_some_and(|project| assignment.receiver.as_ref().is_some_and(|receiver|
                    state.project_names.is_some_and(|names| names.for_module(project.module)
                        .namespaces.contains_key(receiver.as_str()))))
            {
                return Err("fixed callable array update requires an ordinary lexical statement without a previous result".into());
            }
            let array = selected_array(state, &destination.place).expect("authenticated callable array");
            let (parameters, outputs) = scalar_only_signature(&array.element).ok_or("invalid fixed callable array signature")?;
            let value = target(state, expression, parameters, outputs, false)?;
            let pointer = selected_pointer(context, state, &destination.place, project)?;
            state.builder.build_store(pointer, value).map_err(builder_error)?;
            return Ok(Some(EmitValue::Unit));
        }
    }
    let Some(nodes) = state.body_call_nodes.filter(|_| !state.local_callable_slots.is_empty()) else {
        return Ok(None);
    };
    let selected = assignment.targets.iter().find_map(|target| {
        let span = nodes.lexical_existing_destination_span(&target.place)?;
        state.local_callable_slots.get(&(span.start, span.end))
    });
    let Some(slot) = selected else { return Ok(None) };
    let [expression] = assignment.values.as_slice() else {
        return Err("local callable replacement requires one direct value".into());
    };
    if assignment.targets.len() != 1 || !matches!(assignment.targets[0].place, crate::ScalarPlace::Name { .. }) {
        return Err("local callable replacement requires one lexical destination".into());
    }
    let pointer = target(state, expression, &slot.parameters, &slot.outputs, true)?;
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
