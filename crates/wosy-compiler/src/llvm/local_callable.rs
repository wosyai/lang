use super::*;
use crate::ScalarBinding;

pub(super) struct LocalCallableSlot<'ctx> {
    storage: PointerValue<'ctx>,
    generated: Option<bool>,
    pub(super) parameters: Vec<ScalarType>,
    pub(super) outputs: crate::ScalarOutputSequence,
}

/// A private array of the same capture-free pointers as lexical callable slots.
pub(super) struct LocalCallableArray<'ctx> {
    storage: PointerValue<'ctx>,
    generated: Option<bool>,
    geometry: CallableArrayGeometry<'ctx>,
    element: ScalarType,
}

#[derive(Clone, Copy)]
enum CallableArrayGeometry<'ctx> {
    Fixed { ty: inkwell::types::ArrayType<'ctx>, length: u64 },
    GeneratedRuntime(inkwell::values::IntValue<'ctx>),
}

pub(super) fn generated_entry_layout(target: ScalarTargetLayout) -> Result<(u64, u64), String> {
    match target {
        ScalarTargetLayout::NATIVE64 | ScalarTargetLayout::WASM32 => Ok((16, 8)),
        _ => Err("unsupported private generated callable entry geometry".into()),
    }
}

pub(super) fn generated_entry_type(context: &Context) -> inkwell::types::StructType<'_> {
    context.struct_type(&[context.ptr_type(AddressSpace::default()).into(), context.i64_type().into()], false)
}

pub(super) fn emit_generated_factory<'ctx>(
    context: &'ctx Context, factory: FunctionValue<'ctx>, child: FunctionValue<'ctx>, captures: bool,
) -> Result<(), String> {
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(factory, "entry"));
    let entry = generated_entry_type(context).get_undef();
    let entry = builder.build_insert_value(entry, child.as_global_value().as_pointer_value(), 0, "generated_code")
        .map_err(builder_error)?.into_struct_value();
    let env = if captures { factory.get_first_param().ok_or("generated factory missing index")?.into_int_value() }
        else { context.i64_type().const_zero() };
    let entry = builder.build_insert_value(entry, env, 1, "generated_capture")
        .map_err(builder_error)?.into_struct_value();
    builder.build_return(Some(&entry)).map_err(builder_error)?;
    Ok(())
}

pub(super) fn prepared_generated_factory<'a>(state: &'a EmitState<'_, '_>, binding: &ScalarBinding) -> Option<(&'a str, bool)> {
    let (name, captures) = state.initializer_symbols?.generated_array_factories.get(&std::ptr::from_ref(&binding.value))?;
    Some((name, *captures))
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
    // {ptr, i64} is naturally aligned to 8 and occupies 16 bytes on both supported targets.
    let stride = if prepared_generated_factory(state, binding).is_some() { generated_entry_layout(state.target_layout)?.0 }
        else { state.target_layout.pointer_size };
    let bytes = length.checked_mul(stride).ok_or("fixed callable array size overflows")?;
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

pub(super) fn generated_fixed_extent(state: &EmitState<'_, '_>, place: &crate::ScalarPlace) -> Option<u64> {
    let array = selected_array(state, place)?;
    let nodes = state.body_call_nodes?;
    let CallableArrayGeometry::Fixed { length, .. } = array.geometry else { return None; };
    if array.generated.is_none() || !matches!(place, crate::ScalarPlace::Name { .. })
        || !nodes.checked_place_type(place).and_then(array_element)
            .is_some_and(|(element, actual)| same_scalar_signature(element, &array.element) && actual == length)
    { return None; }
    Some(length)
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
        || !nodes.checked_place_type(place).is_some_and(|ty| same_scalar_signature(ty, &selected.element))
    {
        return Err("callable array changed its checked place or element".into());
    }
    let (storage, geometry) = (selected.storage, selected.geometry);
    let base_ty = nodes.checked_place_type(base);
    match geometry {
        CallableArrayGeometry::Fixed { ty, length } => {
            if !base_ty.and_then(array_element).is_some_and(|(element, actual)|
                same_scalar_signature(element, &selected.element) && actual == length)
            { return Err("fixed callable array changed its extent or element".into()); }
            let selected = memory::emit_place_index(context, state, index, project)?;
            memory::check_fixed_index(context, state, selected, length, None)?;
            memory::fixed_array_storage_element_pointer(context, state, ty.into(), storage, selected)
        }
        CallableArrayGeometry::GeneratedRuntime(count) => {
            if !matches!(base_ty, Some(ScalarType::RuntimeArray { element, .. })
                if same_scalar_signature(element, &selected.element))
            { return Err("generated runtime callable changed its extent authority".into()); }
            let selected = memory::emit_place_index(context, state, index, project)?;
            memory::check_array_extent(context, state, selected, count, None)?;
            let selected = memory::array_address_index(context, state, selected)?;
            // The exact constructor checked byte capacity and installed the
            // entire private layout before registering this owner/backing.
            unsafe { state.builder.build_in_bounds_gep(generated_entry_type(context), storage, &[selected], "generated_callable_selected") }
                .map_err(builder_error)
        }
    }
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
    target_pointer(state, symbol, parameters, outputs)
}

fn target_pointer<'ctx>(
    state: &EmitState<'ctx, '_>, symbol: &str,
    parameters: &[ScalarType], outputs: &crate::ScalarOutputSequence,
) -> Result<PointerValue<'ctx>, String> {
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

pub(super) fn install_generated_runtime<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, receiver: &crate::scalar::ScalarOutputReceiver,
    callee: FunctionValue<'ctx>, backing: PointerValue<'ctx>, count: inkwell::values::IntValue<'ctx>, captures: bool,
) -> Result<(), String> {
    let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
        return Err("generated callable construction lost its runtime receiver".into());
    };
    let entry = generated_entry_type(context);
    memory::emit_generated_elements(context, state, callee, count, |state, index, result| {
        if result.get_type() != entry.into() { return Err("generated runtime factory changed entry type".into()); }
        let index = memory::array_address_index(context, state, index)?;
        let pointer = unsafe { state.builder.build_in_bounds_gep(entry, backing, &[index], "generated_callable_cell") }
            .map_err(builder_error)?;
        state.builder.build_store(pointer, result).map_err(builder_error)?;
        Ok(())
    })?;
    if state.local_callable_arrays.insert(receiver.name_span, LocalCallableArray {
        storage: backing, generated: Some(captures),
        geometry: CallableArrayGeometry::GeneratedRuntime(count), element: element.as_ref().clone(),
    }).is_some() { return Err("duplicate generated runtime callable declaration".into()); }
    Ok(())
}

pub(super) fn binding_value<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, binding: &ScalarBinding,
    project: Option<ProjectCallScope<'_>>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    if array_element(&binding.declared_type).is_some() {
        let (_, element, length) = array_binding(state, binding)?;
        if let Some((name, _)) = prepared_generated_factory(state, binding) {
            let callee = *state.functions.get(name).ok_or("missing private generated factory")?;
            let entry = generated_entry_type(context);
            let array = entry.array_type(length);
            let storage = entry_alloca(state, array.into(), "generated_callable_array")?;
            if length != 0 {
                let count = context.i64_type().const_int(length.into(), false);
                memory::emit_generated_elements(context, state, callee, count, |state, index, result| {
                    if result.get_type() != entry.into() { return Err("generated factory changed entry type".into()); }
                    let slot = memory::fixed_array_storage_element_pointer(context, state, array.into(), storage, index)?;
                    state.builder.build_store(slot, result).map_err(builder_error)?;
                    Ok(())
                })?;
            }
            return Ok(Some(EmitValue::Basic(storage.into())));
        }
        let ScalarExpression::ArrayLiteral { elements, .. } = &binding.value else {
            return Err("fixed callable array requires complete literal construction".into());
        };
        if elements.len() != length as usize {
            return Err("fixed callable array literal count differs from its extent".into());
        }
        let (parameters, outputs) = scalar_only_signature(element).ok_or("invalid fixed callable array signature")?;
        let resolved = elements.iter().map(|expression| {
            if matches!(expression, ScalarExpression::AnonymousCallable { .. }) {
                let symbol = state.inline_array_arm_symbol(expression)
                    .ok_or("unprepared inline callable array element")?;
                target_pointer(state, symbol, parameters, outputs)
            } else {
                target(state, expression, parameters, outputs, false)
            }
        }).collect::<Result<Vec<_>, _>>()?;
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
        let generated = selected_array(state, place).and_then(|array| array.generated);
        let pointer = selected_pointer(context, state, place, project)?;
        let ty: BasicTypeEnum<'ctx> = if generated.is_some() { generated_entry_type(context).into() }
            else { context.ptr_type(AddressSpace::default()).into() };
        let value = state.builder.build_load(ty, pointer, "selected_callable")
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
        let generated = prepared_generated_factory(state, binding).map(|(_, captures)| captures);
        let (storage, llvm_type) = if generated.is_some() {
            let EmitValue::Basic(BasicValueEnum::PointerValue(storage)) = value else {
                return Err("generated callable array lacks fully initialized storage".into());
            };
            (*storage, generated_entry_type(context).array_type(length))
        } else {
            let EmitValue::Basic(BasicValueEnum::ArrayValue(array)) = value else {
                return Err("fixed callable array lacks fully initialized storage".into());
            };
            let ty = context.ptr_type(AddressSpace::default()).array_type(length);
            if array.get_type() != ty { return Err("fixed callable array changed its physical extent".into()); }
            let storage = entry_alloca(state, ty.into(), "local_callable_array")?;
            state.builder.build_store(storage, *array).map_err(builder_error)?;
            (storage, ty)
        };
        if state.local_callable_arrays.insert(receiver.name_span, LocalCallableArray {
            storage, generated, geometry: CallableArrayGeometry::Fixed { ty: llvm_type, length: length.into() }, element: element.clone(),
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
    let generated = match &binding.value {
        ScalarExpression::IndexedRead { place, .. } => selected_array(state, place).and_then(|array| array.generated),
        _ => None,
    };
    let EmitValue::Basic(required) = value else { return Err("missing proved local callable initializer".into()); };
    let ty: BasicTypeEnum<'ctx> = if generated.is_some() { generated_entry_type(context).into() }
        else { context.ptr_type(AddressSpace::default()).into() };
    if required.get_type() != ty { return Err("local callable initializer changes its private layout".into()); }
    let storage = entry_alloca(state, ty, "local_callable")?;
    state.builder.build_store(storage, *required).map_err(builder_error)?;
    let key = (receiver.name_span.start, receiver.name_span.end);
    if state.local_callable_slots.insert(key, LocalCallableSlot {
        storage, generated, parameters: parameters.to_vec(), outputs: outputs.clone(),
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
            if array.generated.is_some() { return Err("generated callable array is not an updateable slot".into()); }
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
    if slot.generated.is_some() { return Err("generated callable snapshot is not an updateable slot".into()); }
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
    let (storage, parameters, outputs, generated) =
        (slot.storage, slot.parameters.clone(), slot.outputs.clone(), slot.generated);
    invoke_from_storage(context, state, storage, generated, &parameters, &outputs, arguments, project)
}

pub(super) fn invoke_indexed<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, selected: &ScalarExpression,
    arguments: &[ScalarExpression], project: ProjectCallScope<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let ScalarExpression::IndexedRead { place, .. } = selected else {
        return Err("indexed invocation lost its canonical selection".into());
    };
    let nodes = state.body_call_nodes.ok_or("indexed invocation lacks prepared selection nodes")?;
    let array = selected_array(state, place).ok_or("indexed invocation requires its exact lexical callable array")?;
    let element = nodes.checked_expression_type(selected)
        .filter(|ty| same_scalar_signature(ty, &array.element))
        .ok_or("indexed invocation changed its prepared element signature")?;
    let (parameters, outputs) = scalar_only_signature(element).ok_or("indexed invocation requires a closed scalar signature")?;
    let (parameters, outputs, generated) = (parameters.to_vec(), outputs.clone(), array.generated);
    let storage = selected_pointer(context, state, place, Some(project))?;
    invoke_from_storage(context, state, storage, generated, &parameters, &outputs, arguments, Some(project))
}

fn invoke_from_storage<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>, storage: PointerValue<'ctx>,
    generated: Option<bool>, parameters: &[ScalarType], outputs: &crate::ScalarOutputSequence,
    arguments: &[ScalarExpression], project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    if arguments.len() != parameters.len() {
        return Err("local callable slot argument count differs from its signature".into());
    }
    let mut types = parameters.iter().map(|ty| basic_type(context, ty, state.target_layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum<'ctx>>, _>>()?;
    let (pointer, capture) = if let Some(captures) = generated {
        let entry = state.builder.build_load(generated_entry_type(context), storage, "generated_callable_snapshot")
            .map_err(builder_error)?.into_struct_value();
        let pointer = state.builder.build_extract_value(entry, 0, "generated_callable_code")
            .map_err(builder_error)?.into_pointer_value();
        let capture = if captures {
            types.insert(0, context.i64_type().into());
            Some(state.builder.build_extract_value(entry, 1, "generated_callable_environment")
                .map_err(builder_error)?)
        } else { None };
        (pointer, capture)
    } else {
        (state.builder.build_load(context.ptr_type(AddressSpace::default()), storage, "local_callable_target")
            .map_err(builder_error)?.into_pointer_value(), None)
    };
    let ty = basic_type(context, &outputs.outputs[0].ty, state.target_layout)?.fn_type(&types, false);
    let mut values = super::expression::emit_contextual_arguments(context, state, arguments, parameters, project)?
        .into_iter().map(|value| take_basic(value).map(Into::into)).collect::<Result<Vec<_>, _>>()?;
    if let Some(capture) = capture { values.insert(0, capture.into()); }
    let call = state.builder.build_indirect_call(ty, pointer, &values, "local_callable_call").map_err(builder_error)?;
    state.call_owner_flags.clear();
    super::expression::ordinary_call_result(call.try_as_basic_value(), outputs)
}
