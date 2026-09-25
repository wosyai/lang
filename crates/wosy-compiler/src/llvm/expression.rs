use super::*;

pub(super) fn emit_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            if name == "null" {
                return Ok(EmitValue::Basic(
                    pointer_integer_type(context, state.target_layout)
                        .const_zero()
                        .into(),
                ));
            }
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                    return Ok(EmitValue::Basic(slot.into()));
                }
                if matches!(ty, ScalarType::RuntimeArray { .. })
                    && state
                        .runtime_array_allocations
                        .get(name)
                        .copied()
                        .unwrap_or(false)
                {
                    return Ok(EmitValue::Basic(slot.into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            storage_type(context, &ty, state.structs, state.target_layout)?,
                            slot,
                            name,
                        )
                        .map_err(builder_error)?,
                ));
            }
            if let Some((global, ty)) = state.globals.get(name).cloned() {
                if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                    return Ok(EmitValue::Basic(global.as_pointer_value().into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            storage_type(context, &ty, state.structs, state.target_layout)?,
                            global.as_pointer_value(),
                            name,
                        )
                        .map_err(builder_error)?,
                ));
            }
            state
                .values
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown LLVM value {name}"))
        }
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            if let Some(tag) = enum_tag {
                return Ok(EmitValue::Basic(
                    context.i32_type().const_int(u64::from(*tag), false).into(),
                ));
            }
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            Err(format!("unknown LLVM member {receiver}.{name}"))
        }
        ScalarExpression::Dereference { place, .. } => emit_place_value(context, state, place),
        ScalarExpression::IndexedRead { place, .. } => emit_place_value(context, state, place),
        ScalarExpression::Integer { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(
                    i32::try_from(value.clone()).map_err(|_| "invalid i32 literal")? as u64,
                    true,
                )
                .into(),
        )),
        ScalarExpression::InvalidInteger { .. } => Err("invalid integer literal".to_owned()),
        ScalarExpression::Float { .. } => {
            Err("floating-point literal requires an expected type".to_owned())
        }
        ScalarExpression::InvalidFloat { .. } => Err("invalid floating-point literal".to_owned()),
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Char { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(u64::from(*value as u32), false)
                .into(),
        )),
        ScalarExpression::Utf8 { .. } => {
            Err("string literal requires an expected std.utf8 type".to_owned())
        }
        ScalarExpression::Unary {
            operator, operand, ..
        } => {
            let value = take_basic(emit_expression(context, state, operand)?)?.into_int_value();
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => emit_binary(context, state, operator, left, right),
        ScalarExpression::Call {
            receiver,
            name,
            span,
            type_arguments,
            arguments,
            overload_selection,
            ..
        } => {
            if state.invalidations.contains_key(&(span.start, span.end)) {
                return emit_core_invalidation(context, state, *span, arguments, None);
            }
            if receiver.as_deref() == Some("core") && name == "alloc" {
                return emit_core_alloc(context, state, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "free" {
                return emit_core_free(context, state, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "system_panic" {
                return emit_core_system_panic(context, state, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return emit_int_conversion(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "uint_to_float"
                        | "sint_to_float"
                        | "float_to_sint_trunc"
                        | "float_to_uint_trunc"
                        | "float_trunc"
                        | "float_extend"
                )
            {
                return emit_float_conversion(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return emit_bitcast(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return emit_core_pointer_cast(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "offset" {
                return emit_core_offset(context, state, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "load" {
                return emit_core_load(context, state, type_arguments, arguments);
            }
            emit_call(
                context,
                state,
                receiver.as_deref(),
                name,
                type_arguments,
                arguments,
                overload_selection.as_ref(),
            )
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => emit_if(context, state, condition, then_branch, else_branch),
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => emit_unit_if(context, state, condition, then_branch),
        ScalarExpression::Block(block) => emit_block(context, state, block),
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => Ok(EmitValue::Basic(
            state
                .builder
                .build_ptr_to_int(
                    place_pointer(context, state, place, None)?,
                    pointer_integer_type(context, state.target_layout),
                    "address",
                )
                .map_err(builder_error)?
                .into(),
        )),
        ScalarExpression::StructLiteral { .. } => {
            return Err("struct literal requires an expected struct type".to_owned())
        }
        ScalarExpression::ArrayLiteral { .. } => {
            return Err("array literal requires an expected fixed array type".to_owned())
        }
    }
}

pub(super) fn emit_project_typed_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    expected: &ScalarType,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Utf8 { value, .. } => emit_utf8_literal(context, state, value, expected),
        ScalarExpression::Unary {
            operator, operand, ..
        } if integer_width(expected).is_some() || *expected == ScalarType::Bool => {
            let value = take_basic(emit_project_typed_expression(
                context, state, operand, expected, module, modules,
            )?)?
            .into_int_value();
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Integer { value, .. } if integer_width(expected).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, expected, value)?.into()),
        ),
        ScalarExpression::Float { value, .. } => Ok(EmitValue::Basic(
            float_constant(context, expected, *value)?.into(),
        )),
        ScalarExpression::StructLiteral { fields, .. } => {
            emit_struct_literal(context, state, expected, fields)
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            emit_array_literal(context, state, expected, elements)
        }
        _ => emit_project_expression(context, state, expression, module, modules),
    }
}

pub(super) fn emit_project_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            if let Some(tag) = enum_tag {
                return Ok(EmitValue::Basic(
                    context.i32_type().const_int(u64::from(*tag), false).into(),
                ));
            }
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            let target = project_namespace_target(module, modules, receiver)?;
            let (global, ty) = project_member_global(state, target, name)?;
            if ty == ScalarType::Unit {
                return Ok(EmitValue::Unit);
            }
            let global = global.ok_or_else(|| format!("unknown LLVM member storage {name}"))?;
            if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                return Ok(EmitValue::Basic(global.as_pointer_value().into()));
            }
            Ok(EmitValue::Basic(
                state
                    .builder
                    .build_load(
                        storage_type(context, &ty, state.structs, state.target_layout)?,
                        global.as_pointer_value(),
                        name,
                    )
                    .map_err(builder_error)?,
            ))
        }
        ScalarExpression::Dereference { place, .. } => emit_place_value_in_project(
            context,
            state,
            place,
            Some(ProjectCallScope { module, modules }),
        ),
        ScalarExpression::IndexedRead { place, .. } => emit_place_value_in_project(
            context,
            state,
            place,
            Some(ProjectCallScope { module, modules }),
        ),
        ScalarExpression::Call {
            receiver,
            name,
            span,
            type_arguments,
            arguments,
            overload_selection,
            ..
        } => {
            if state.invalidations.contains_key(&(span.start, span.end)) {
                return emit_core_invalidation(
                    context,
                    state,
                    *span,
                    arguments,
                    Some(ProjectCallScope { module, modules }),
                );
            }
            if receiver.as_deref() == Some("core") && name == "alloc" {
                return emit_core_alloc_project(context, state, arguments, module, modules);
            }
            if receiver.as_deref() == Some("core") && name == "free" {
                return emit_core_free_project(context, state, arguments, module, modules);
            }
            if receiver.as_deref() == Some("core") && name == "system_panic" {
                return emit_core_system_panic(context, state, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return emit_int_conversion_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "uint_to_float"
                        | "sint_to_float"
                        | "float_to_sint_trunc"
                        | "float_to_uint_trunc"
                        | "float_trunc"
                        | "float_extend"
                )
            {
                return emit_float_conversion_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return emit_bitcast_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return emit_core_pointer_cast_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "offset" {
                return emit_core_offset_project(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "load" {
                return emit_core_load_project(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            let target = receiver.as_ref().map_or(module, |binding| {
                module
                    .namespace_bindings
                    .iter()
                    .find(|namespace| namespace.binding == *binding)
                    .and_then(|namespace| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == namespace.target)
                            .copied()
                    })
                    .unwrap_or(module)
            });
            let qualified = receiver
                .as_deref()
                .filter(|binding| {
                    target.items.iter().any(|item| {
                        matches!(
                            item,
                            ScalarItem::Extern(extern_decl)
                                if extern_decl.binding == *binding
                                    && extern_decl.functions.iter().any(|function| function.name == *name)
                        )
                    })
                })
                .map_or_else(
                    || project_function_name(&target.source, name),
                    |binding| {
                        let import_module = target
                            .items
                            .iter()
                            .find_map(|item| match item {
                                ScalarItem::Extern(extern_decl)
                                    if extern_decl.binding == binding
                                        && extern_decl.functions.iter().any(|function| function.name == *name) =>
                                {
                                    match &extern_decl.actual_module {
                                        crate::scalar::ScalarExternModule::Valid(import_module) => {
                                            Some(import_module.as_str())
                                        }
                                        crate::scalar::ScalarExternModule::Invalid { .. } => None,
                                    }
                                }
                                _ => None,
                            })
                            .expect("validated extern module");
                        project_extern_lookup_key(
                            &target.source,
                            binding,
                            import_module,
                            name,
                        )
                    },
                );
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            let target = state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified))
                .ok_or_else(|| format!("unknown LLVM callable {qualified}"))?
                .clone();
            let ScalarType::Callable { parameters, .. } = state
                .signatures
                .get(&target)
                .ok_or_else(|| format!("unknown LLVM callable signature {qualified}"))?
            else {
                return Err(format!(
                    "LLVM callable {qualified} has no callable signature"
                ));
            };
            let values = arguments
                .iter()
                .zip(parameters)
                .map(|(argument, parameter)| {
                    emit_project_typed_expression(
                        context, state, argument, parameter, module, modules,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            emit_call_values(context, state, &target, values)
        }
        ScalarExpression::Name { name, .. } => emit_expression(
            context,
            state,
            &ScalarExpression::Name {
                name: name.clone(),
                span: wosy_syntax::ByteSpan::new(0, 0),
            },
        ),
        ScalarExpression::Integer { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(
                    i32::try_from(value.clone()).map_err(|_| "invalid i32 literal")? as u64,
                    true,
                )
                .into(),
        )),
        ScalarExpression::InvalidInteger { .. } => Err("invalid integer literal".to_owned()),
        ScalarExpression::Float { .. } => {
            Err("floating-point literal requires an expected type".to_owned())
        }
        ScalarExpression::InvalidFloat { .. } => Err("invalid floating-point literal".to_owned()),
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Char { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(u64::from(*value as u32), false)
                .into(),
        )),
        ScalarExpression::Utf8 { .. } => {
            Err("string literal requires an expected std.utf8 type".to_owned())
        }
        ScalarExpression::Unary {
            operator, operand, ..
        } => {
            let value = take_basic(emit_project_expression(
                context, state, operand, module, modules,
            )?)?
            .into_int_value();
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            let unsigned = project_expression_is_unsigned(state, left, module, modules);
            let left = emit_project_expression(context, state, left, module, modules)?;
            emit_binary_with_rhs(
                context,
                state,
                operator,
                left,
                right,
                ExpressionPath::Project { module, modules },
                unsigned,
            )
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let condition = emit_project_expression(context, state, condition, module, modules)?;
            emit_project_if(
                context,
                state,
                condition,
                then_branch,
                else_branch,
                module,
                modules,
            )
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            let condition = emit_project_expression(context, state, condition, module, modules)?;
            emit_project_unit_if(context, state, condition, then_branch, module, modules)
        }
        ScalarExpression::Block(block) => {
            emit_project_block(context, state, block, module, modules)
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => Ok(EmitValue::Basic(
            state
                .builder
                .build_ptr_to_int(
                    place_pointer(
                        context,
                        state,
                        place,
                        Some(ProjectCallScope { module, modules }),
                    )?,
                    pointer_integer_type(context, state.target_layout),
                    "address",
                )
                .map_err(builder_error)?
                .into(),
        )),
        ScalarExpression::StructLiteral { .. } => {
            return Err("struct literal requires an expected struct type".to_owned())
        }
        ScalarExpression::ArrayLiteral { .. } => {
            return Err("array literal requires an expected fixed array type".to_owned())
        }
    }
}

fn emit_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    receiver: Option<&str>,
    name: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    overload_selection: Option<&ScalarOverloadSelection>,
) -> Result<EmitValue<'ctx>, String> {
    let qualified =
        receiver.map_or_else(|| name.to_owned(), |receiver| format!("{receiver}.{name}"));
    let lookup =
        overload_selection.map(|selection| selected_overload_lookup_key(&qualified, selection));
    let target = state
        .call_targets
        .get(lookup.as_ref().unwrap_or(&qualified))
        .or_else(|| {
            state
                .call_targets
                .get(&specialization_lookup_key(&qualified, type_arguments))
        })
        .or_else(|| state.call_targets.get(&qualified))
        .ok_or_else(|| format!("unknown LLVM callable {qualified}"))?
        .clone();
    let ScalarType::Callable { parameters, .. } = state
        .signatures
        .get(&target)
        .ok_or_else(|| format!("unknown LLVM callable signature {qualified}"))?
    else {
        return Err(format!(
            "LLVM callable {qualified} has no callable signature"
        ));
    };
    let values = arguments
        .iter()
        .zip(parameters)
        .map(|(argument, parameter)| emit_typed_expression(context, state, argument, parameter))
        .collect::<Result<Vec<_>, _>>()?;
    emit_call_values(context, state, &target, values)
}

fn allocation_root_name(place: &crate::ScalarPlace) -> Result<&str, String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => Ok(name),
        crate::ScalarPlace::Index { base, .. } | crate::ScalarPlace::Field { base, .. } => {
            allocation_root_name(base)
        }
        crate::ScalarPlace::Dereference { .. } => {
            Err("core allocation place requires a named owner".to_owned())
        }
    }
}

fn core_effect_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    address: inkwell::values::IntValue<'ctx>,
    candidates: &[InvalidationCandidate],
    project: Option<ProjectCallScope<'_>>,
) -> Result<Vec<(String, inkwell::values::IntValue<'ctx>)>, String> {
    let distinct = candidates
        .iter()
        .map(|candidate| &candidate.place.binding)
        .collect::<BTreeSet<_>>();
    let multiple = distinct.len() > 1;
    let mut owners = BTreeMap::new();
    for candidate in candidates {
        let name = allocation_root_name(&candidate.place.place)?.to_owned();
        let selected = if multiple {
            let pointer = place_pointer(context, state, &candidate.place.place, project)?;
            let pointer = state
                .builder
                .build_ptr_to_int(pointer, address.get_type(), "candidate_address")
                .map_err(builder_error)?;
            state
                .builder
                .build_int_compare(IntPredicate::EQ, pointer, address, "core_effect_target")
                .map_err(builder_error)?
        } else {
            context.bool_type().const_int(1, false)
        };
        if let Some((_, previous)) = owners.get_mut(&candidate.place.binding) {
            *previous = state
                .builder
                .build_or(*previous, selected, "core_effect_owner")
                .map_err(builder_error)?;
        } else {
            owners.insert(candidate.place.binding.clone(), (name, selected));
        }
    }
    Ok(owners.into_values().collect())
}

fn emit_core_invalidation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    span: wosy_syntax::ByteSpan,
    arguments: &[ScalarExpression],
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let (operation, candidates, inputs) = state
        .invalidations
        .get(&(span.start, span.end))
        .cloned()
        .ok_or_else(|| "core invalidation lacks a typed CFG event".to_owned())?;
    let address = take_basic(match project {
        Some(scope) => {
            emit_project_expression(context, state, &arguments[0], scope.module, scope.modules)?
        }
        None => emit_expression(context, state, &arguments[0])?,
    })?
    .into_int_value();
    if operation == CoreOperationId::Invalidate {
        if !matches!(inputs, InvalidationInputs::Invalidate) {
            return Err("inconsistent core.invalidate inputs".to_owned());
        }
        let owners = core_effect_owners(context, state, address, &candidates, project)?;
        let multiple = owners.len() > 1;
        for (name, selected) in owners {
            let live = *state
                .runtime_array_live
                .get(&name)
                .ok_or_else(|| format!("missing allocation state for {name}"))?;
            if multiple {
                let (_, ty) = state
                    .storage
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("missing allocation storage for {name}"))?;
                if !matches!(ty, ScalarType::RuntimeArray { .. }) {
                    return Err(format!("invalid allocation target {name}"));
                }
                let block = state
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| "missing invalidation block".to_owned())?;
                let function = block
                    .get_parent()
                    .ok_or_else(|| "missing invalidation function".to_owned())?;
                let update = context.append_basic_block(function, "invalidate.update");
                let next = context.append_basic_block(function, "invalidate.next");
                state
                    .builder
                    .build_conditional_branch(selected, update, next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(update);
                state
                    .builder
                    .build_store(live, context.bool_type().const_zero())
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_unconditional_branch(next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(next);
            } else {
                state
                    .builder
                    .build_store(live, context.bool_type().const_zero())
                    .map_err(builder_error)?;
            }
        }
        return Ok(EmitValue::Unit);
    }
    if operation == CoreOperationId::Free {
        if !matches!(inputs, InvalidationInputs::Free) || arguments.len() != 1 {
            return Err("inconsistent checked core.free inputs".to_owned());
        }
        declare_core_runtime(context, state.module);
        let deallocator = state
            .module
            .get_function(CORE_FREE_SYMBOL)
            .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
        let owners = core_effect_owners(context, state, address, &candidates, project)?;
        let multiple = owners.len() > 1;
        for (name, selected) in owners {
            let (slot, ty) = state
                .storage
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("missing checked release storage for {name}"))?;
            if !matches!(ty, ScalarType::RuntimeArray { .. })
                || state.runtime_array_allocations.get(&name) != Some(&false)
            {
                return Err(format!(
                    "checked core.free requires an allocation owner {name}"
                ));
            }
            let live_slot = *state
                .runtime_array_live
                .get(&name)
                .ok_or_else(|| format!("missing checked release owner state for {name}"))?;
            if multiple {
                let block = state
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| "missing checked release block".to_owned())?;
                let function = block
                    .get_parent()
                    .ok_or_else(|| "missing checked release function".to_owned())?;
                let update = context.append_basic_block(function, "checked_release.selected");
                let next = context.append_basic_block(function, "checked_release.next");
                state
                    .builder
                    .build_conditional_branch(selected, update, next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(update);
                let live = state
                    .builder
                    .build_load(context.bool_type(), live_slot, "checked_release_live")
                    .map_err(builder_error)?
                    .into_int_value();
                let release = context.append_basic_block(function, "checked_release.live");
                state
                    .builder
                    .build_conditional_branch(live, release, next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(release);
                let backing = state
                    .builder
                    .build_load(
                        context.ptr_type(AddressSpace::default()),
                        slot,
                        "checked_release_backing",
                    )
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_call(deallocator, &[backing.into()], "checked_release")
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_store(live_slot, context.bool_type().const_zero())
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_unconditional_branch(next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(next);
            } else {
                let backing = state
                    .builder
                    .build_load(
                        context.ptr_type(AddressSpace::default()),
                        slot,
                        "checked_release_backing",
                    )
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_call(deallocator, &[backing.into()], "checked_release")
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_store(live_slot, context.bool_type().const_zero())
                    .map_err(builder_error)?;
            }
        }
        return Ok(EmitValue::Unit);
    }
    let InvalidationInputs::Rebind {
        raw_address,
        length,
        ..
    } = inputs
    else {
        return Err("inconsistent core.rebind inputs".to_owned());
    };
    if operation != CoreOperationId::Rebind
        || arguments.len() != 3
        || arguments[1] != raw_address
        || arguments[2] != length
    {
        return Err("core.rebind inputs differ from resolved CFG event".to_owned());
    }
    let raw = take_basic(match project {
        Some(scope) => {
            emit_project_expression(context, state, &raw_address, scope.module, scope.modules)?
        }
        None => emit_expression(context, state, &raw_address)?,
    })?
    .into_int_value();
    let length = take_basic(match project {
        Some(scope) => emit_project_typed_expression(
            context,
            state,
            &length,
            &ScalarType::U64,
            scope.module,
            scope.modules,
        )?,
        None => emit_typed_expression(context, state, &length, &ScalarType::U64)?,
    })?
    .into_int_value();
    let pointer = state
        .builder
        .build_int_to_ptr(
            raw,
            context.ptr_type(AddressSpace::default()),
            "rebound_address",
        )
        .map_err(builder_error)?;
    let owners = core_effect_owners(context, state, address, &candidates, project)?;
    let multiple = owners.len() > 1;
    for (name, selected) in owners {
        let (slot, ty) = state
            .storage
            .get(&name)
            .cloned()
            .ok_or_else(|| format!("missing rebind storage for {name}"))?;
        if !matches!(ty, ScalarType::RuntimeArray { .. })
            || state.runtime_array_allocations.get(&name) != Some(&false)
        {
            return Err(format!(
                "core.rebind requires writable runtime allocation {name}"
            ));
        }
        let length_slot = *state
            .runtime_array_lengths
            .get(&name)
            .ok_or_else(|| format!("missing runtime length for {name}"))?;
        if multiple {
            let current = state
                .builder
                .get_insert_block()
                .ok_or_else(|| "missing rebind block".to_owned())?;
            let function = current
                .get_parent()
                .ok_or_else(|| "missing rebind function".to_owned())?;
            let update = context.append_basic_block(function, "rebind.update");
            let next = context.append_basic_block(function, "rebind.next");
            state
                .builder
                .build_conditional_branch(selected, update, next)
                .map_err(builder_error)?;
            state.builder.position_at_end(update);
            state
                .builder
                .build_store(slot, pointer)
                .map_err(builder_error)?;
            state
                .builder
                .build_store(length_slot, length)
                .map_err(builder_error)?;
            state
                .builder
                .build_unconditional_branch(next)
                .map_err(builder_error)?;
            state.builder.position_at_end(next);
        } else {
            state
                .builder
                .build_store(slot, pointer)
                .map_err(builder_error)?;
            state
                .builder
                .build_store(length_slot, length)
                .map_err(builder_error)?;
        }
    }
    Ok(EmitValue::Unit)
}

fn emit_core_alloc<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [size, alignment] = arguments else {
        return Err("core.alloc has invalid argument arity".to_owned());
    };
    let size = take_basic(emit_typed_expression(
        context,
        state,
        size,
        &ScalarType::U64,
    )?)?;
    let alignment = take_basic(emit_typed_expression(
        context,
        state,
        alignment,
        &ScalarType::U64,
    )?)?;
    emit_core_alloc_values(context, state, size, alignment)
}

fn emit_core_alloc_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [size, alignment] = arguments else {
        return Err("core.alloc has invalid argument arity".to_owned());
    };
    let size = take_basic(emit_project_typed_expression(
        context,
        state,
        size,
        &ScalarType::U64,
        module,
        modules,
    )?)?;
    let alignment = take_basic(emit_project_typed_expression(
        context,
        state,
        alignment,
        &ScalarType::U64,
        module,
        modules,
    )?)?;
    emit_core_alloc_values(context, state, size, alignment)
}

fn emit_core_alloc_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    size: BasicValueEnum<'ctx>,
    alignment: BasicValueEnum<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let allocator = state
        .module
        .get_function(CORE_ALLOC_SYMBOL)
        .ok_or_else(|| "core allocation runtime declaration is missing".to_owned())?;
    let pointer = state
        .builder
        .build_call(allocator, &[size.into(), alignment.into()], "allocation")
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "core allocation runtime returned no pointer".to_owned())?
        .into_pointer_value();
    let address = state
        .builder
        .build_ptr_to_int(
            pointer,
            pointer_integer_type(context, state.target_layout),
            "allocation_address",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(address.into()))
}

fn emit_core_free<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [pointer] = arguments else {
        return Err("core.free has invalid argument arity".to_owned());
    };
    let value = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
    emit_core_free_value(context, state, value)
}

fn emit_core_free_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [pointer] = arguments else {
        return Err("core.free has invalid argument arity".to_owned());
    };
    let value = take_basic(emit_project_expression(
        context, state, pointer, module, modules,
    )?)?
    .into_int_value();
    emit_core_free_value(context, state, value)
}

fn emit_core_free_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: inkwell::values::IntValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let deallocator = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    let pointer = state
        .builder
        .build_int_to_ptr(
            value,
            context.ptr_type(AddressSpace::default()),
            "free_pointer",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_call(deallocator, &[pointer.into()], "free")
        .map_err(builder_error)?;
    Ok(EmitValue::Unit)
}

fn emit_core_system_panic<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    if !arguments.is_empty() {
        return Err("core.system_panic has invalid argument arity".to_owned());
    }
    declare_core_runtime(context, state.module);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(panic, &[], "system_panic")
        .map_err(builder_error)?;
    Ok(EmitValue::Unit)
}

fn emit_int_conversion<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = integer_conversion_source_type(state, value, None)?;
    let destination = &type_argument.ty;
    let value =
        take_basic(emit_typed_expression(context, state, value, &source)?)?.into_int_value();
    let converted = if operation == "int_trunc" {
        state
            .builder
            .build_int_truncate(value, integer_type(context, destination)?, "int_trunc")
    } else if is_signed_integer_type(&source) {
        state
            .builder
            .build_int_s_extend(value, integer_type(context, destination)?, "int_extend")
    } else {
        state
            .builder
            .build_int_z_extend(value, integer_type(context, destination)?, "int_extend")
    }
    .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted.into()))
}

fn is_bitcast_scalar_type(ty: &ScalarType) -> bool {
    integer_width(ty).is_some()
        || matches!(
            ty,
            ScalarType::Char | ScalarType::Bool | ScalarType::F32 | ScalarType::F64
        )
}

fn bitcast_source_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    project_module: Option<&ScalarModule>,
) -> Result<ScalarType, String> {
    let ty = match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone())),
        ScalarExpression::Member { receiver, name, .. } => {
            match struct_member_place(state, receiver, name)? {
                Some(crate::ScalarPlace::Field {
                    field: ScalarFieldReference::Resolved(field),
                    ..
                }) => structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == field)
                    .map(|field| field.ty.clone()),
                Some(_) | None => None,
            }
        }
        ScalarExpression::Dereference { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Integer { .. } => Some(ScalarType::I32),
        ScalarExpression::Char { .. } => Some(ScalarType::Char),
        ScalarExpression::Boolean { .. } => Some(ScalarType::Bool),
        ScalarExpression::Float { .. } => Some(ScalarType::F64),
        ScalarExpression::Unary { operand, .. } => {
            bitcast_source_type(state, operand, project_module).ok()
        }
        ScalarExpression::Binary { left, .. } => {
            bitcast_source_type(state, left, project_module).ok()
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } if receiver.as_deref() == Some("core")
            && matches!(
                name.as_str(),
                "int_trunc"
                    | "int_extend"
                    | "uint_to_float"
                    | "sint_to_float"
                    | "float_to_sint_trunc"
                    | "float_to_uint_trunc"
                    | "float_trunc"
                    | "float_extend"
                    | "bitcast"
            ) =>
        {
            type_arguments.first().map(|argument| argument.ty.clone())
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver.as_ref().map_or_else(
                || {
                    project_module.map_or_else(
                        || name.clone(),
                        |module| project_function_name(&module.source, name),
                    )
                },
                |receiver| format!("{receiver}.{name}"),
            );
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified))
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .map(|output| output.ty.clone())
        }
        _ => None,
    };
    ty.filter(|ty| is_bitcast_scalar_type(ty))
        .ok_or_else(|| "core bitcast has an unknown source type".to_owned())
}

fn emit_bitcast<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = bitcast_source_type(state, value, None)?;
    let destination = &type_argument.ty;
    let value = take_basic(emit_typed_expression(context, state, value, &source)?)?;
    let converted = state
        .builder
        .build_bit_cast(
            value,
            basic_type(context, destination, state.target_layout)?,
            "bitcast",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted))
}
fn emit_int_conversion_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = integer_conversion_source_type(state, value, Some(module))?;
    let destination = &type_argument.ty;
    let value = take_basic(emit_project_typed_expression(
        context, state, value, &source, module, modules,
    )?)?
    .into_int_value();
    let converted = if operation == "int_trunc" {
        state
            .builder
            .build_int_truncate(value, integer_type(context, destination)?, "int_trunc")
    } else if is_signed_integer_type(&source) {
        state
            .builder
            .build_int_s_extend(value, integer_type(context, destination)?, "int_extend")
    } else {
        state
            .builder
            .build_int_z_extend(value, integer_type(context, destination)?, "int_extend")
    }
    .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted.into()))
}

fn emit_bitcast_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = bitcast_source_type(state, value, Some(module))?;
    let destination = &type_argument.ty;
    let value = take_basic(emit_project_typed_expression(
        context, state, value, &source, module, modules,
    )?)?;
    let converted = state
        .builder
        .build_bit_cast(
            value,
            basic_type(context, destination, state.target_layout)?,
            "bitcast",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted))
}
fn emit_core_pointer_cast<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let value = take_basic(emit_expression(context, state, value)?)?;
    emit_core_pointer_cast_values(context, state, value, &type_argument.ty)
}

fn emit_core_pointer_cast_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let value = take_basic(emit_project_expression(
        context, state, value, module, modules,
    )?)?;
    emit_core_pointer_cast_values(context, state, value, &type_argument.ty)
}

fn emit_core_pointer_cast_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: BasicValueEnum<'ctx>,
    destination: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let converted = state
        .builder
        .build_bit_cast(
            value,
            basic_type(context, destination, state.target_layout)?,
            "pointer_cast",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted))
}
fn emit_float_conversion<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let destination = &type_argument.ty;
    match operation {
        "uint_to_float" | "sint_to_float" => {
            let source = integer_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_int_value();
            let converted = if is_signed_integer_type(&source) {
                state.builder.build_signed_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_unsigned_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_to_sint_trunc" | "float_to_uint_trunc" => {
            let source = float_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_float_value();
            let converted = if is_signed_integer_type(destination) {
                state.builder.build_float_to_signed_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_float_to_unsigned_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_trunc" => {
            let source = float_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_float_value();
            let converted = state
                .builder
                .build_float_trunc(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_extend" => {
            let source = float_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_float_value();
            let converted = state
                .builder
                .build_float_ext(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        _ => Err(format!(
            "core.{operation} is not a floating-point conversion"
        )),
    }
}

fn emit_float_conversion_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let destination = &type_argument.ty;
    match operation {
        "uint_to_float" | "sint_to_float" => {
            let source = integer_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_int_value();
            let converted = if is_signed_integer_type(&source) {
                state.builder.build_signed_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_unsigned_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_to_sint_trunc" | "float_to_uint_trunc" => {
            let source = float_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_float_value();
            let converted = if is_signed_integer_type(destination) {
                state.builder.build_float_to_signed_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_float_to_unsigned_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_trunc" => {
            let source = float_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_float_value();
            let converted = state
                .builder
                .build_float_trunc(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_extend" => {
            let source = float_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_float_value();
            let converted = state
                .builder
                .build_float_ext(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        _ => Err(format!(
            "core.{operation} is not a floating-point conversion"
        )),
    }
}

fn emit_core_offset<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.offset has invalid type argument arity".to_owned());
    };
    let [pointer, count] = arguments else {
        return Err("core.offset has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
    let count = take_basic(emit_typed_expression(
        context,
        state,
        count,
        &ScalarType::I64,
    )?)?
    .into_int_value();
    emit_core_offset_values(context, state, pointer, count, &type_argument.ty)
}

fn emit_core_offset_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.offset has invalid type argument arity".to_owned());
    };
    let [pointer, count] = arguments else {
        return Err("core.offset has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_project_expression(
        context, state, pointer, module, modules,
    )?)?
    .into_int_value();
    let count = take_basic(emit_project_typed_expression(
        context,
        state,
        count,
        &ScalarType::I64,
        module,
        modules,
    )?)?
    .into_int_value();
    emit_core_offset_values(context, state, pointer, count, &type_argument.ty)
}

fn emit_core_offset_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    pointer: inkwell::values::IntValue<'ctx>,
    count: inkwell::values::IntValue<'ctx>,
    pointee: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let element = storage_type(context, pointee, state.structs, state.target_layout)?;
    let base = state
        .builder
        .build_int_to_ptr(
            pointer,
            context.ptr_type(AddressSpace::default()),
            "offset_base",
        )
        .map_err(builder_error)?;
    let advanced = unsafe {
        state
            .builder
            .build_in_bounds_gep(element, base, &[count], "offset")
    }
    .map_err(builder_error)?;
    let address = state
        .builder
        .build_ptr_to_int(
            advanced,
            pointer_integer_type(context, state.target_layout),
            "offset_address",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(address.into()))
}

fn emit_core_load<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.load has invalid type argument arity".to_owned());
    };
    let [pointer] = arguments else {
        return Err("core.load has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
    emit_core_load_values(context, state, pointer, &type_argument.ty)
}

fn emit_core_load_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.load has invalid type argument arity".to_owned());
    };
    let [pointer] = arguments else {
        return Err("core.load has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_project_expression(
        context, state, pointer, module, modules,
    )?)?
    .into_int_value();
    emit_core_load_values(context, state, pointer, &type_argument.ty)
}

fn emit_core_load_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    pointer: inkwell::values::IntValue<'ctx>,
    pointee: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let base = state
        .builder
        .build_int_to_ptr(
            pointer,
            context.ptr_type(AddressSpace::default()),
            "load_base",
        )
        .map_err(builder_error)?;
    if matches!(
        pointee,
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
    ) {
        return Ok(EmitValue::Basic(base.into()));
    }
    let value = state
        .builder
        .build_load(
            basic_type(context, pointee, state.target_layout)?,
            base,
            "load",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(value))
}

fn is_signed_integer_type(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128
    )
}

fn is_float_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::F32 | ScalarType::F64)
}

fn float_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<inkwell::types::FloatType<'ctx>, String> {
    match ty {
        ScalarType::F32 => Ok(context.f32_type()),
        ScalarType::F64 => Ok(context.f64_type()),
        _ => Err("expected floating-point type".to_owned()),
    }
}

fn integer_conversion_source_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    project_module: Option<&ScalarModule>,
) -> Result<ScalarType, String> {
    let ty = match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone())),
        ScalarExpression::Member { receiver, name, .. } => {
            match struct_member_place(state, receiver, name)? {
                Some(crate::ScalarPlace::Field {
                    field: ScalarFieldReference::Resolved(field),
                    ..
                }) => structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == field)
                    .map(|field| field.ty.clone()),
                Some(_) | None => None,
            }
        }
        ScalarExpression::Dereference { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Integer { .. } => Some(ScalarType::I32),
        ScalarExpression::Unary { operand, .. } => {
            integer_conversion_source_type(state, operand, project_module).ok()
        }
        ScalarExpression::Binary { left, .. } => {
            integer_conversion_source_type(state, left, project_module).ok()
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } if receiver.as_deref() == Some("core")
            && matches!(
                name.as_str(),
                "int_trunc"
                    | "int_extend"
                    | "uint_to_float"
                    | "sint_to_float"
                    | "float_to_sint_trunc"
                    | "float_to_uint_trunc"
                    | "float_trunc"
                    | "float_extend"
            ) =>
        {
            type_arguments.first().map(|argument| argument.ty.clone())
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver.as_ref().map_or_else(
                || {
                    project_module.map_or_else(
                        || name.clone(),
                        |module| project_function_name(&module.source, name),
                    )
                },
                |receiver| format!("{receiver}.{name}"),
            );
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified))
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .map(|output| output.ty.clone())
        }
        _ => None,
    };
    ty.filter(|ty| integer_width(ty).is_some())
        .ok_or_else(|| "core integer conversion has an unknown source type".to_owned())
}

fn float_conversion_source_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    project_module: Option<&ScalarModule>,
) -> Result<ScalarType, String> {
    let ty = match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone())),
        ScalarExpression::Member { receiver, name, .. } => {
            match struct_member_place(state, receiver, name)? {
                Some(crate::ScalarPlace::Field {
                    field: ScalarFieldReference::Resolved(field),
                    ..
                }) => structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == field)
                    .map(|field| field.ty.clone()),
                Some(_) | None => None,
            }
        }
        ScalarExpression::Dereference { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Integer { .. } => Some(ScalarType::I32),
        ScalarExpression::Float { .. } => Some(ScalarType::F64),
        ScalarExpression::Unary { operand, .. } => {
            float_conversion_source_type(state, operand, project_module).ok()
        }
        ScalarExpression::Binary { left, .. } => {
            float_conversion_source_type(state, left, project_module).ok()
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } if receiver.as_deref() == Some("core")
            && matches!(
                name.as_str(),
                "int_trunc"
                    | "int_extend"
                    | "uint_to_float"
                    | "sint_to_float"
                    | "float_to_sint_trunc"
                    | "float_to_uint_trunc"
                    | "float_trunc"
                    | "float_extend"
            ) =>
        {
            type_arguments.first().map(|argument| argument.ty.clone())
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver.as_ref().map_or_else(
                || {
                    project_module.map_or_else(
                        || name.clone(),
                        |module| project_function_name(&module.source, name),
                    )
                },
                |receiver| format!("{receiver}.{name}"),
            );
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified))
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .map(|output| output.ty.clone())
        }
        _ => None,
    };
    ty.filter(|ty| integer_width(ty).is_some() || is_float_type(ty))
        .ok_or_else(|| "core float conversion has an unknown source type".to_owned())
}

fn emit_call_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
    values: Vec<EmitValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let function = *state
        .functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM callable {name}"))?;
    let mut arguments = values
        .into_iter()
        .map(|value| match value {
            EmitValue::Basic(value) => Ok(BasicMetadataValueEnum::from(value)),
            EmitValue::Unit => Err("unit argument is invalid".into()),
            EmitValue::Aggregate { .. } => Err("aggregate argument is invalid".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ScalarType::Callable { parameters, .. } = state
        .signatures
        .get(name)
        .ok_or_else(|| format!("unknown LLVM callable signature {name}"))?
    else {
        return Err(format!("LLVM callable {name} has no callable signature"));
    };
    let owner_outputs = if function.count_params() as usize > parameters.len() {
        state
            .dynamic_owner_outputs
            .get(name)
            .ok_or_else(|| format!("missing owner signature for {name}"))?
    } else {
        &BTreeSet::new()
    };
    let mut owner_slots = BTreeMap::new();
    for output in owner_outputs {
        let slot = entry_alloca(
            state,
            context.bool_type().into(),
            &format!("checked_return_owner_{output}"),
        )?;
        arguments.push(slot.into());
        owner_slots.insert(*output, slot);
    }
    let call = state
        .builder
        .build_call(function, &arguments, "call")
        .map_err(builder_error)?;
    state.call_owner_flags = owner_slots
        .into_iter()
        .map(|(output, slot)| {
            state
                .builder
                .build_load(
                    context.bool_type(),
                    slot,
                    &format!("checked_return_owned_{output}"),
                )
                .map(|value| (output, value.into_int_value()))
                .map_err(builder_error)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let outputs = match state.signatures.get(name) {
        Some(ScalarType::Callable { outputs, .. }) => outputs,
        _ => return Err(format!("unknown LLVM callable signature {name}")),
    };
    match (call.try_as_basic_value(), outputs.outputs.as_slice()) {
        (ValueKind::Basic(value), [output]) => {
            if output.ty == ScalarType::Unit {
                return Err("unit callable produced a value".to_owned());
            }
            Ok(EmitValue::Basic(value))
        }
        (ValueKind::Basic(value), outputs) if outputs.len() > 1 => Ok(EmitValue::Aggregate {
            value: value.into_struct_value(),
            outputs: outputs.iter().map(|output| output.ty.clone()).collect(),
        }),
        (ValueKind::Instruction(_), []) | (ValueKind::Instruction(_), [_]) => Ok(EmitValue::Unit),
        (ValueKind::Instruction(_), _) => Err("aggregate callable produced no value".to_owned()),
        (ValueKind::Basic(_), []) | (ValueKind::Basic(_), &[_, _, ..]) => {
            Err("LLVM callable result shape does not match its signature".to_owned())
        }
    }
}

fn emit_binary<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: &ScalarExpression,
    right: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    let unsigned = expression_is_unsigned(state, left);
    let left = emit_expression(context, state, left)?;
    emit_binary_with_rhs(
        context,
        state,
        operator,
        left,
        right,
        ExpressionPath::Single,
        unsigned,
    )
}

fn expression_is_unsigned<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
) -> bool {
    match expression {
        ScalarExpression::Member { receiver, name, .. } => {
            if let Ok(Some(crate::ScalarPlace::Field { field, .. })) =
                struct_member_place(state, receiver, name)
            {
                if let ScalarFieldReference::Resolved(field) = field {
                    if let Ok(structure) = structure(state.structs, field.structure.clone()) {
                        if let Some(field) = structure.fields.get(field.index) {
                            return matches!(
                                &field.ty,
                                ScalarType::U8
                                    | ScalarType::U16
                                    | ScalarType::U32
                                    | ScalarType::U64
                                    | ScalarType::U128
                            );
                        }
                    }
                }
            }
            false
        }
        ScalarExpression::Name { name, .. } => match state.storage.get(name) {
            Some((_, ty)) => matches!(
                ty,
                ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
            ),
            None => match state.globals.get(name) {
                Some((_, ty)) => matches!(
                    ty,
                    ScalarType::U8
                        | ScalarType::U16
                        | ScalarType::U32
                        | ScalarType::U64
                        | ScalarType::U128
                ),
                None => false,
            },
        },
        ScalarExpression::Unary { operand, .. } => expression_is_unsigned(state, operand),
        ScalarExpression::Binary { left, .. } => expression_is_unsigned(state, left),
        _ => false,
    }
}

fn project_expression_is_unsigned<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> bool {
    match expression {
        ScalarExpression::Member { receiver, name, .. } => {
            if let Ok(target) = project_namespace_target(module, modules, receiver) {
                return target.items.iter().any(|item| {
                    matches!(
                        item,
                        ScalarItem::Binding(binding)
                            if binding.name == *name && matches!(
                                &binding.declared_type,
                                ScalarType::U8
                                    | ScalarType::U16
                                    | ScalarType::U32
                                    | ScalarType::U64
                                    | ScalarType::U128
                            )
                    )
                });
            }
            expression_is_unsigned(state, expression)
        }
        ScalarExpression::Unary { operand, .. } => {
            project_expression_is_unsigned(state, operand, module, modules)
        }
        ScalarExpression::Binary { left, .. } => {
            project_expression_is_unsigned(state, left, module, modules)
        }
        _ => expression_is_unsigned(state, expression),
    }
}

enum ExpressionPath<'a> {
    Single,
    Project {
        module: &'a ScalarModule,
        modules: &'a [&'a ScalarModule],
    },
}

fn emit_binary_with_rhs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: EmitValue<'ctx>,
    right: &ScalarExpression,
    path: ExpressionPath<'_>,
    unsigned: bool,
) -> Result<EmitValue<'ctx>, String> {
    if !matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
        let right = match path {
            ExpressionPath::Single => match right {
                ScalarExpression::Integer { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::IntValue(value)) => {
                        let ty = integer_scalar_type(value.get_type().get_bit_width())?;
                        emit_typed_expression(context, state, right, &ty)?
                    }
                    _ => emit_expression(context, state, right)?,
                },
                ScalarExpression::Float { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::FloatValue(value)) => {
                        let ty = if value.get_type() == context.f32_type() {
                            ScalarType::F32
                        } else {
                            ScalarType::F64
                        };
                        emit_typed_expression(context, state, right, &ty)?
                    }
                    _ => emit_expression(context, state, right)?,
                },
                _ => emit_expression(context, state, right)?,
            },
            ExpressionPath::Project { module, modules } => match right {
                ScalarExpression::Integer { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::IntValue(value)) => {
                        let ty = integer_scalar_type(value.get_type().get_bit_width())?;
                        emit_project_typed_expression(context, state, right, &ty, module, modules)?
                    }
                    _ => emit_project_expression(context, state, right, module, modules)?,
                },
                ScalarExpression::Float { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::FloatValue(value)) => {
                        let ty = if value.get_type() == context.f32_type() {
                            ScalarType::F32
                        } else {
                            ScalarType::F64
                        };
                        emit_project_typed_expression(context, state, right, &ty, module, modules)?
                    }
                    _ => emit_project_expression(context, state, right, module, modules)?,
                },
                _ => emit_project_expression(context, state, right, module, modules)?,
            },
        };
        return emit_binary_values(state, operator, left, right, unsigned);
    }

    let left = take_basic(left)?.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let rhs_block = context.append_basic_block(function, "short_circuit.rhs");
    let merge = context.append_basic_block(function, "short_circuit.merge");
    let left_block = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    let (true_block, false_block) = match operator {
        BinaryOperator::And => (rhs_block, merge),
        BinaryOperator::Or => (merge, rhs_block),
        _ => return Err("invalid short-circuit operator".to_owned()),
    };
    state
        .builder
        .build_conditional_branch(left, true_block, false_block)
        .map_err(builder_error)?;

    state.builder.position_at_end(rhs_block);
    let right = match path {
        ExpressionPath::Single => emit_expression(context, state, right)?,
        ExpressionPath::Project { module, modules } => {
            emit_project_expression(context, state, right, module, modules)?
        }
    };
    let right = take_basic(right)?.into_int_value();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let rhs_end = state.builder.get_insert_block().expect("RHS block");
    state.builder.position_at_end(merge);

    let skipped = left
        .get_type()
        .const_int(u64::from(matches!(operator, BinaryOperator::Or)), false);
    let phi = state
        .builder
        .build_phi(left.get_type(), "short_circuit")
        .map_err(builder_error)?;
    phi.add_incoming(&[(&skipped, left_block), (&right, rhs_end)]);
    Ok(EmitValue::Basic(phi.as_basic_value()))
}

fn emit_binary_values<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: EmitValue<'ctx>,
    right: EmitValue<'ctx>,
    unsigned: bool,
) -> Result<EmitValue<'ctx>, String> {
    let left = take_basic(left)?;
    let right = take_basic(right)?;
    if let (BasicValueEnum::FloatValue(left), BasicValueEnum::FloatValue(right)) = (left, right) {
        if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
            return Err("short-circuit operators require expression lowering".to_owned());
        }
        return match operator {
            BinaryOperator::Add => state
                .builder
                .build_float_add(left, right, "add")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Subtract => state
                .builder
                .build_float_sub(left, right, "sub")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Multiply => state
                .builder
                .build_float_mul(left, right, "mul")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Divide => state
                .builder
                .build_float_div(left, right, "div")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Remainder => state
                .builder
                .build_float_rem(left, right, "rem")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Equal => state
                .builder
                .build_float_compare(FloatPredicate::OEQ, left, right, "eq")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::NotEqual => state
                .builder
                .build_float_compare(FloatPredicate::ONE, left, right, "ne")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Less => state
                .builder
                .build_float_compare(FloatPredicate::OLT, left, right, "lt")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::LessEqual => state
                .builder
                .build_float_compare(FloatPredicate::OLE, left, right, "le")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Greater => state
                .builder
                .build_float_compare(FloatPredicate::OGT, left, right, "gt")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::GreaterEqual => state
                .builder
                .build_float_compare(FloatPredicate::OGE, left, right, "ge")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::And
            | BinaryOperator::Or
            | BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight => {
                return Err("integer operator used with floating-point values".to_owned())
            }
        }
        .map_err(builder_error);
    }
    let left = left.into_int_value();
    let right = right.into_int_value();
    let value = match operator {
        BinaryOperator::Add => state
            .builder
            .build_int_add(left, right, "add")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Subtract => state
            .builder
            .build_int_sub(left, right, "sub")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Multiply => state
            .builder
            .build_int_mul(left, right, "mul")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Divide => if unsigned {
            state.builder.build_int_unsigned_div(left, right, "div")
        } else {
            state.builder.build_int_signed_div(left, right, "div")
        }
        .map_err(builder_error)?
        .into(),
        BinaryOperator::Remainder => if unsigned {
            state.builder.build_int_unsigned_rem(left, right, "rem")
        } else {
            state.builder.build_int_signed_rem(left, right, "rem")
        }
        .map_err(builder_error)?
        .into(),
        BinaryOperator::And | BinaryOperator::Or => {
            return Err("short-circuit operators require expression lowering".to_owned())
        }
        BinaryOperator::Equal => state
            .builder
            .build_int_compare(IntPredicate::EQ, left, right, "eq")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::NotEqual => state
            .builder
            .build_int_compare(IntPredicate::NE, left, right, "ne")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Less => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::ULT
                } else {
                    IntPredicate::SLT
                },
                left,
                right,
                "lt",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::LessEqual => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::ULE
                } else {
                    IntPredicate::SLE
                },
                left,
                right,
                "le",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Greater => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::UGT
                } else {
                    IntPredicate::SGT
                },
                left,
                right,
                "gt",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::GreaterEqual => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::UGE
                } else {
                    IntPredicate::SGE
                },
                left,
                right,
                "ge",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::BitAnd => state
            .builder
            .build_and(left, right, "and")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::BitOr => state
            .builder
            .build_or(left, right, "or")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::BitXor => state
            .builder
            .build_xor(left, right, "xor")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::ShiftLeft => state
            .builder
            .build_left_shift(left, right, "shl")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::ShiftRight => state
            .builder
            .build_right_shift(left, right, !unsigned, "shr")
            .map_err(builder_error)?
            .into(),
    };
    Ok(EmitValue::Basic(value))
}

pub(super) fn emit_unary_value<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    operator: &crate::scalar::UnaryOperator,
    value: inkwell::values::IntValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    match operator {
        crate::scalar::UnaryOperator::LogicalNot | crate::scalar::UnaryOperator::BitwiseNot => {
            state
                .builder
                .build_not(value, "not")
                .map(|value| EmitValue::Basic(value.into()))
                .map_err(builder_error)
        }
        crate::scalar::UnaryOperator::Negate => state
            .builder
            .build_int_sub(value.get_type().const_zero(), value, "neg")
            .map(|value| EmitValue::Basic(value.into()))
            .map_err(builder_error),
    }
}

fn emit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: &ScalarExpression,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(emit_expression(context, state, condition)?)?.into_int_value();
    emit_if_value(
        context,
        state,
        condition,
        then_branch,
        else_branch,
        |state, block| emit_block(context, state, block),
    )
}
fn emit_project_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: EmitValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(condition)?.into_int_value();
    emit_if_value(
        context,
        state,
        condition,
        then_branch,
        else_branch,
        |state, block| emit_project_block(context, state, block, module, modules),
    )
}

fn emit_unit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: &ScalarExpression,
    then_branch: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(emit_expression(context, state, condition)?)?.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    emit_block(context, state, then_branch)?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge);
    Ok(EmitValue::Unit)
}

fn emit_project_unit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: EmitValue<'ctx>,
    then_branch: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(condition)?.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    emit_project_block(context, state, then_branch, module, modules)?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge);
    Ok(EmitValue::Unit)
}

pub(super) fn emit_if_value<'ctx, 'module, F>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: inkwell::values::IntValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    emit: F,
) -> Result<EmitValue<'ctx>, String>
where
    F: Fn(&mut EmitState<'ctx, 'module>, &ScalarBlock) -> Result<EmitValue<'ctx>, String>,
{
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let else_block = context.append_basic_block(function, "if.else");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, else_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    let incoming_owners = state.return_owner_flags.clone();
    state.return_owner_flags = incoming_owners.clone();
    let then_value = emit(state, then_branch)?;
    let then_owners = state.return_owner_flags.clone();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let then_end = state.builder.get_insert_block().expect("then block");
    state.builder.position_at_end(else_block);
    state.return_owner_flags = incoming_owners.clone();
    let else_value = emit(state, else_branch)?;
    let else_owners = state.return_owner_flags.clone();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let else_end = state.builder.get_insert_block().expect("else block");
    state.builder.position_at_end(merge);
    state.return_owner_flags = incoming_owners;
    for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
        match (then_owners.get(&output), else_owners.get(&output)) {
            (Some(then_owner), Some(else_owner)) => {
                let phi = state
                    .builder
                    .build_phi(context.bool_type(), &format!("return_owner_{output}"))
                    .map_err(builder_error)?;
                phi.add_incoming(&[(then_owner, then_end), (else_owner, else_end)]);
                state
                    .return_owner_flags
                    .insert(output, phi.as_basic_value().into_int_value());
            }
            (None, None) => {}
            _ => {
                return Err(format!(
                    "conditional return lacks owner for output {output}"
                ))
            }
        }
    }
    match (then_value, else_value) {
        (EmitValue::Unit, EmitValue::Unit) => Ok(EmitValue::Unit),
        (EmitValue::Basic(then_value), EmitValue::Basic(else_value)) => {
            let phi = state
                .builder
                .build_phi(then_value.get_type(), "if")
                .map_err(builder_error)?;
            phi.add_incoming(&[(&then_value, then_end), (&else_value, else_end)]);
            Ok(EmitValue::Basic(phi.as_basic_value()))
        }
        (
            EmitValue::Aggregate {
                value: then_value,
                outputs,
            },
            EmitValue::Aggregate {
                value: else_value,
                outputs: else_outputs,
            },
        ) if outputs == else_outputs => {
            let phi = state
                .builder
                .build_phi(then_value.get_type(), "if")
                .map_err(builder_error)?;
            phi.add_incoming(&[(&then_value, then_end), (&else_value, else_end)]);
            Ok(EmitValue::Aggregate {
                value: phi.as_basic_value().into_struct_value(),
                outputs,
            })
        }
        _ => Err("conditional branches have different LLVM values".into()),
    }
}
