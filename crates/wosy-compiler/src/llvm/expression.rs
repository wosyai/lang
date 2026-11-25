use super::memory::{load_place_value, resolved_field_pointer};
use super::*;
use crate::scalar::control::{BoundInitializerMember, InitializationMember, InitializationStorage};

fn prepared_initializer_member<'a>(
    state: &EmitState<'_, 'a>,
    expression: &ScalarExpression,
) -> Result<Option<BoundInitializerMember<'a>>, String> {
    state
        .initializer_nodes
        .filter(|_| state.prepared_render)
        .map(|nodes| nodes.member(expression))
        .transpose()
}

fn emit_initializer_member<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    member: BoundInitializerMember<'_>,
    expression: &ScalarExpression,
    project_path: bool,
) -> Result<EmitValue<'ctx>, String> {
    match member.identity {
        InitializationMember::Enum { tag, .. } => Ok(EmitValue::Basic(
            context.i32_type().const_int(u64::from(*tag), false).into(),
        )),
        InitializationMember::Field {
            receiver,
            receiver_name,
            field,
            through_raw_pointer,
        } => {
            if *through_raw_pointer {
                if project_path {
                    return Err(format!("unknown namespace binding {receiver_name}"));
                }
                let ScalarExpression::Member { name, .. } = expression else {
                    unreachable!()
                };
                return Err(format!("unknown LLVM member {receiver_name}.{name}"));
            }
            let base = match receiver {
                InitializationStorage::Lexical(_) => {
                    state.storage.get(receiver_name).map(|(slot, _)| *slot)
                }
                InitializationStorage::Module(_) => state
                    .globals
                    .get(receiver_name)
                    .map(|(global, _)| global.as_pointer_value()),
            }
            .ok_or_else(|| format!("unknown LLVM place {receiver_name}"))?;
            let pointer = resolved_field_pointer(context, state, base, field)?;
            load_place_value(context, state, pointer, member.ty)
        }
        InitializationMember::Module {
            declaration, name, ..
        } => {
            if *member.ty == ScalarType::Unit {
                return Ok(EmitValue::Unit);
            }
            let key = project_global_name(&declaration.source, name);
            let (global, _) = state.all_globals.get(&key).ok_or_else(|| {
                if matches!(member.ty, ScalarType::Callable { .. }) {
                    format!("unknown LLVM member {name}")
                } else {
                    format!("unknown LLVM member storage {name}")
                }
            })?;
            let address = global.as_pointer_value();
            if matches!(member.ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                return Ok(EmitValue::Basic(address.into()));
            }
            Ok(EmitValue::Basic(
                state
                    .builder
                    .build_load(
                        storage_type(context, member.ty, state.layouts, state.target_layout)?,
                        address,
                        name,
                    )
                    .map_err(builder_error)?,
            ))
        }
    }
}

pub(super) fn emit_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::AnonymousCallable { .. } => Err("unprepared inline callable".into()),
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
                            storage_type(context, &ty, state.layouts, state.target_layout)?,
                            slot,
                            name,
                        )
                        .map_err(builder_error)?,
                ));
            }
            if let Some((global, ty)) = state.globals.get(name) {
                if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                    return Ok(EmitValue::Basic(global.as_pointer_value().into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            storage_type(context, ty, state.layouts, state.target_layout)?,
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
                .or_else(|| state.unit_values.contains(name.as_str()).then_some(EmitValue::Unit))
                .ok_or_else(|| format!("unknown LLVM value {name}"))
        }
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            if let Some(member) = prepared_initializer_member(state, expression)? {
                return emit_initializer_member(context, state, member, expression, false);
            }
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
        ScalarExpression::ValueField { receiver, .. } =>
            emit_value_field(context, state, expression, receiver, None),
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
        ScalarExpression::Float { value, .. } => {
            let ty = checked_float_type(state, expression)
                .ok_or_else(|| "floating-point literal requires an expected type".to_owned())?;
            Ok(EmitValue::Basic(float_constant(context, ty, *value)?.into()))
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
            let value = take_basic(emit_expression(context, state, operand)?)?;
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => match super::ownership::guarded_scalar_condition(state, expression)? {
            Some(value) => Ok(value),
            None => emit_binary(context, state, operator, left, right),
        },
        ScalarExpression::Call {
            receiver,
            name,
            span,
            type_arguments,
            arguments,
            ..
        } => {
            if let Some(value) =
                aggregate_transfer::emit_call(context, state, expression, arguments, None)?
            {
                return Ok(value);
            }
            let selected = state.prepared_call(expression)?;
            let (receiver, name) = match &selected {
                Some(PreparedCall::Core(operation)) => {
                    (Some("core"), initializer_core_name(*operation))
                }
                Some(target) => {
                    return emit_prepared_call(
                        context,
                        state,
                        target,
                        expression,
                        receiver.as_deref(),
                        name,
                        arguments,
                        None,
                    );
                }
                None => (receiver.as_deref(), name.as_str()),
            };
            if matches!(selected, Some(PreparedCall::Core(CoreOperationId::ArrayLen))) {
                return super::memory::emit_array_len(context, state, expression, arguments, None);
            }
            if uses_invalidation_event(state, &selected, *span)? {
                return emit_core_invalidation(context, state, *span, arguments, None);
            }
            if receiver == Some("core") && name == "alloc" {
                return emit_core_alloc(context, state, arguments);
            }
            if receiver == Some("core") && name == "free" {
                return emit_core_free(context, state, arguments);
            }
            if receiver == Some("core") && name == "system_panic" {
                return emit_core_system_panic(context, state, arguments);
            }
            if receiver == Some("core") && matches!(name, "int_trunc" | "int_extend") {
                return emit_int_conversion(context, state, name, type_arguments, arguments);
            }
            if receiver == Some("core")
                && matches!(
                    name,
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
            if receiver == Some("core") && name == "bitcast" {
                return emit_bitcast(context, state, name, type_arguments, arguments);
            }
            if receiver == Some("core") && name == "pointer_cast" {
                return emit_core_pointer_cast(context, state, name, type_arguments, arguments);
            }
            if receiver == Some("core") && name == "offset" {
                return emit_core_offset(context, state, type_arguments, arguments);
            }
            if receiver == Some("core") && name == "load" {
                return emit_core_load(context, state, type_arguments, arguments);
            }
            if selected.is_some() {
                return Err(format!("unknown LLVM callable core.{name}"));
            }
            emit_call(
                context,
                state,
                receiver,
                name,
                type_arguments,
                arguments,
                expression,
            )
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => emit_if(context, state, expression, condition, then_branch, else_branch),
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => emit_unit_if(context, state, expression, condition, then_branch),
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
    if aggregate_transfer::fallible_allocation_expression_at(state, expression)
        || aggregate_transfer::ordinary_fallible_allocation_expression_at(state, expression)
    {
        return emit_contextual_numeric(context, state, expression, expected, ExpressionPath::Project { module, modules });
    }
    match expression {
        ScalarExpression::Utf8 { value, .. } => emit_utf8_literal(context, state, value, expected),
        ScalarExpression::Unary {
            operator, operand, ..
        } if integer_width(expected).is_some() || *expected == ScalarType::Bool => {
            let value = take_basic(emit_project_typed_expression(
                context, state, operand, expected, module, modules,
            )?)?;
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Integer { value, .. } if integer_width(expected).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, expected, value)?.into()),
        ),
        ScalarExpression::Binary { .. } | ScalarExpression::If { .. } | ScalarExpression::Block(_)
            if integer_width(expected).is_some() =>
        {
            emit_contextual_numeric(
                context, state, expression, expected, ExpressionPath::Project { module, modules },
            )
        }
        _ if matches!(expected, ScalarType::F32 | ScalarType::F64) => emit_contextual_numeric(
            context,
            state,
            expression,
            expected,
            ExpressionPath::Project { module, modules },
        ),
        ScalarExpression::StructLiteral { .. } => {
            emit_struct_literal(context, state, expected, expression, ExpressionPath::Project { module, modules })
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            emit_array_literal(context, state, expected, elements, ExpressionPath::Project { module, modules })
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
        ScalarExpression::AnonymousCallable { .. } => Err("unprepared inline callable".into()),
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            if let Some(member) = prepared_initializer_member(state, expression)? {
                return emit_initializer_member(context, state, member, expression, true);
            }
            if let Some(tag) = enum_tag {
                return Ok(EmitValue::Basic(
                    context.i32_type().const_int(u64::from(*tag), false).into(),
                ));
            }
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            let names = state.project_names.expect("project emission names");
            let target = project_namespace_target(names, module, receiver)?;
            let (global, ty) = project_member_global(target, name)?;
            if *ty == ScalarType::Unit {
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
                        storage_type(context, ty, state.layouts, state.target_layout)?,
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
        ScalarExpression::ValueField { receiver, .. } =>
            emit_value_field(context, state, expression, receiver, Some((module, modules))),
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
            ..
        } => {
            if let Some(value) = aggregate_transfer::emit_call(
                context,
                state,
                expression,
                arguments,
                Some((module, modules)),
            )? {
                return Ok(value);
            }
            let selected = state.prepared_call(expression)?;
            let (receiver, name) = match &selected {
                Some(PreparedCall::Core(operation)) => {
                    (Some("core"), initializer_core_name(*operation))
                }
                Some(target) => {
                    return emit_prepared_call(
                        context,
                        state,
                        target,
                        expression,
                        receiver.as_deref(),
                        name,
                        arguments,
                        Some(ProjectCallScope { module, modules }),
                    );
                }
                None => (receiver.as_deref(), name.as_str()),
            };
            if matches!(selected, Some(PreparedCall::Core(CoreOperationId::ArrayLen))) {
                return super::memory::emit_array_len(context, state, expression, arguments,
                    Some(ProjectCallScope { module, modules }));
            }
            if uses_invalidation_event(state, &selected, *span)? {
                return emit_core_invalidation(
                    context,
                    state,
                    *span,
                    arguments,
                    Some(ProjectCallScope { module, modules }),
                );
            }
            if receiver == Some("core") && name == "alloc" {
                return emit_core_alloc_project(context, state, arguments, module, modules);
            }
            if receiver == Some("core") && name == "free" {
                return emit_core_free_project(context, state, arguments, module, modules);
            }
            if receiver == Some("core") && name == "system_panic" {
                return emit_core_system_panic(context, state, arguments);
            }
            if receiver == Some("core") && matches!(name, "int_trunc" | "int_extend") {
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
            if receiver == Some("core")
                && matches!(
                    name,
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
            if receiver == Some("core") && name == "bitcast" {
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
            if receiver == Some("core") && name == "pointer_cast" {
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
            if receiver == Some("core") && name == "offset" {
                return emit_core_offset_project(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver == Some("core") && name == "load" {
                return emit_core_load_project(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if selected.is_some() {
                return Err("unknown LLVM extern binding core".to_owned());
            }
            let qualified = if let Some(binding) = receiver {
                let names = state.project_names.expect("project emission names");
                let scope = names.for_module(module);
                if let Some(source) = scope.namespaces.get(binding) {
                    if !names.positions.contains_key(*source) {
                        return Err(format!("unknown LLVM namespace {binding}"));
                    }
                    project_function_name(source, name)
                } else {
                    let declaration = scope.externs
                        .get(binding)
                        .ok_or_else(|| format!("unknown LLVM extern binding {binding}"))?;
                    let crate::scalar::ScalarExternModule::Valid(import_module) =
                        &declaration.actual_module
                    else {
                        return Err(format!("invalid LLVM extern module for {binding}"));
                    };
                    project_extern_lookup_key(&module.source, binding, import_module, name)
                }
            } else {
                project_function_name(&module.source, name)
            };
            let target = state
                .raw_call_symbol(expression, &qualified, type_arguments)?
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
        ScalarExpression::Float { value, .. } => {
            let ty = checked_float_type(state, expression)
                .ok_or_else(|| "floating-point literal requires an expected type".to_owned())?;
            Ok(EmitValue::Basic(float_constant(context, ty, *value)?.into()))
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
            )?)?;
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            if let Some(value) = super::ownership::guarded_scalar_condition(state, expression)? {
                return Ok(value);
            }
            emit_binary_expression(
                context,
                state,
                operator,
                left,
                right,
                ExpressionPath::Project { module, modules },
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
                expression,
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
            emit_project_unit_if(context, state, expression, condition, then_branch, module, modules)
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

fn initializer_core_name(operation: CoreOperationId) -> &'static str {
    use CoreOperationId::*;
    match operation {
        Alloc => "alloc",
        Free => "free",
        Invalidate => "invalidate",
        Rebind => "rebind",
        ArrayLen => "array_len",
        SystemPanic => "system_panic",
        ThisArtifactId => "this_artifact_id",
        DeclareArtifact => "declare_artifact",
        IntTrunc => "int_trunc",
        IntExtend => "int_extend",
        UintToFloat => "uint_to_float",
        SintToFloat => "sint_to_float",
        FloatToSintTrunc => "float_to_sint_trunc",
        FloatToUintTrunc => "float_to_uint_trunc",
        FloatTrunc => "float_trunc",
        FloatExtend => "float_extend",
        Bitcast => "bitcast",
        Offset => "offset",
        Load => "load",
        PointerCast => "pointer_cast",
    }
}

fn emit_prepared_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    selected: &PreparedCall<'_>,
    expression: &ScalarExpression,
    receiver: Option<&str>,
    name: &str,
    arguments: &[ScalarExpression],
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    if let PreparedCall::Local(key) = selected {
        return super::local_callable::invoke(context, state, *key, arguments, project);
    }
    let Some(target) = selected.symbol() else {
        return Err(match (project, receiver) {
            (Some(project), Some(receiver)) => {
                match project
                    .module
                    .namespace_bindings
                    .iter()
                    .find(|namespace| namespace.binding == receiver)
                {
                    Some(namespace) => format!(
                        "unknown LLVM callable {}",
                        project_function_name(&namespace.target, name)
                    ),
                    None => format!("unknown LLVM extern binding {receiver}"),
                }
            }
            (Some(project), None) => format!(
                "unknown LLVM callable {}",
                project_function_name(&project.module.source, name)
            ),
            (None, Some(receiver)) => format!("unknown LLVM callable {receiver}.{name}"),
            (None, None) => format!("unknown LLVM callable {name}"),
        });
    };
    let Some(ScalarType::Callable { parameters, .. }) = state.signatures.get(target) else {
        return Err(state.missing_prepared_signature(target));
    };
    if let Some(value) = aggregate_transfer::emit_private_runtime_access_call(
        context, state, expression, target, arguments, parameters, project,
    )? {
        return Ok(value);
    }
    if let Some(value) = aggregate_transfer::emit_borrowed_extent_call(
        context, state, expression, target, arguments, parameters, project,
    )? {
        return Ok(value);
    }
    let values = emit_contextual_arguments(context, state, arguments, parameters, project)?;
    let ordinary_fallible = aggregate_transfer::ordinary_fallible_allocation_call_at(state, expression);
    if ordinary_fallible || aggregate_transfer::fallible_allocation_call_at(state, expression) {
        let result = emit_sealed_allocation_call_values(context, state, target, &values)?;
        if !ordinary_fallible {
            return Ok(EmitValue::Basic(result.into()));
        }
        let parent = state.builder.get_insert_block().and_then(|block| block.get_parent())
            .ok_or("missing sealed ordinary allocation caller")?;
        let present = context.append_basic_block(parent, "ordinary.allocation_present");
        let absent = context.append_basic_block(parent, "ordinary.allocation_absent");
        let succeeded = state.builder.build_extract_value(result, 0, "ordinary.allocation_succeeded")
            .map_err(builder_error)?.into_int_value();
        state.builder.build_conditional_branch(succeeded, present, absent).map_err(builder_error)?;
        state.builder.position_at_end(absent);
        emit_terminal_system_panic(context, state)?;
        state.builder.position_at_end(present);
        let payload = state.builder.build_extract_value(result, 1, "ordinary.allocation_payload")
            .map_err(builder_error)?;
        return Ok(EmitValue::Basic(payload));
    }
    if let Some(value) = aggregate_transfer::emit_array_descriptor_call(context, state, expression, target, arguments, &values)? {
        return Ok(value);
    }
    reject_unpaired_fixed_copy_argument(state, arguments, parameters)?;
    aggregate_transfer::capture_checked_array_argument_extent(state, expression, arguments, &values)?;
    emit_call_values(context, state, target, values)
}

fn emit_sealed_allocation_call_values<'ctx>(
    context: &'ctx Context, state: &EmitState<'ctx, '_>, target: &str, values: &[EmitValue<'ctx>],
) -> Result<inkwell::values::StructValue<'ctx>, String> {
    let [EmitValue::Basic(BasicValueEnum::IntValue(count))] = values else {
        return Err("invalid sealed fallible allocation call argument".into());
    };
    let callee = state.functions.get(target).ok_or("missing sealed fallible allocation callable")?;
    let Some(ScalarType::Callable { outputs, .. }) = state.signatures.get(target) else {
        return Err("missing sealed fallible allocation callable signature".into());
    };
    let [output] = outputs.outputs.as_slice() else {
        return Err("invalid sealed fallible allocation output signature".into());
    };
    let pair = aggregate_transfer::fallible_allocation_result_type(context, &output.ty)?;
    if callee.get_linkage() != Linkage::Internal
        || callee.get_type() != pair.fn_type(&[context.i64_type().into()], false)
    {
        return Err("invalid sealed fallible allocation callable ABI".into());
    }
    let call = state.builder.build_call(*callee, &[(*count).into()], "fallible.rhs").map_err(builder_error)?;
    let Some(BasicValueEnum::StructValue(result)) = call.try_as_basic_value().basic() else {
        return Err("missing sealed fallible allocation result".into());
    };
    Ok(result)
}

fn reject_unpaired_fixed_copy_argument(
    state: &EmitState<'_, '_>,
    arguments: &[ScalarExpression],
    parameters: &[ScalarType],
) -> Result<(), String> {
    if let ([ScalarExpression::Name { name, .. }], [parameter @ ScalarType::RuntimeArray { .. }]) = (arguments, parameters) {
        let ty = state.storage.get(name).map(|(_, ty)| ty)
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty));
        if ty.is_some_and(|ty| crate::scalar::fixed_copy_to_runtime(parameter, ty).is_some()) {
            return Err("fixed Copy owning argument requires a sealed owning receiver".into());
        }
    }
    Ok(())
}

pub(super) fn emit_contextual_arguments<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
    parameters: &[ScalarType],
    project: Option<ProjectCallScope<'_>>,
) -> Result<Vec<EmitValue<'ctx>>, String> {
    arguments
        .iter()
        .zip(parameters)
        .map(|(argument, parameter)| match project {
            Some(ProjectCallScope { module, modules }) => {
                emit_project_typed_expression(context, state, argument, parameter, module, modules)
            }
            None => emit_typed_expression(context, state, argument, parameter),
        })
        .collect()
}

fn emit_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    receiver: Option<&str>,
    name: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    expression: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    let qualified =
        receiver.map_or_else(|| name.to_owned(), |receiver| format!("{receiver}.{name}"));
    let target = state
        .raw_call_symbol(expression, &qualified, type_arguments)?
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
    reject_unpaired_fixed_copy_argument(state, arguments, parameters)?;
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

fn uses_invalidation_event(
    state: &EmitState<'_, '_>,
    selected: &Option<PreparedCall<'_>>,
    span: wosy_syntax::ByteSpan,
) -> Result<bool, String> {
    let Some((operation, _, _)) = state.invalidations.get(&(span.start, span.end)) else {
        return Ok(false);
    };
    if let Some(PreparedCall::Core(selected)) = selected {
        if selected != operation {
            return Err("core invalidation differs from the prepared call target".into());
        }
    }
    Ok(true)
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
    if operation == CoreOperationId::Free
        && ownership::release_guarded_owner_at(context, state, span, arguments, &candidates)?
    {
        return Ok(EmitValue::Unit);
    }
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
            emit_project_expression(context, state, &arguments[1], scope.module, scope.modules)?
        }
        None => emit_expression(context, state, &arguments[1])?,
    })?
    .into_int_value();
    let length = take_basic(match project {
        Some(scope) => emit_project_typed_expression(
            context,
            state,
            &arguments[2],
            &ScalarType::U64,
            scope.module,
            scope.modules,
        )?,
        None => emit_typed_expression(context, state, &arguments[2], &ScalarType::U64)?,
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
    let pointer = super::ownership::emit_standard_allocation_call(state, size, alignment)?;
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

pub(super) fn emit_terminal_system_panic<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
) -> Result<(), String> {
    emit_core_system_panic(context, state, &[])?;
    state.builder.build_unreachable().map_err(builder_error)?;
    Ok(())
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
        ScalarExpression::Member { .. }
            if state.prepared_render && state.initializer_nodes.is_some() =>
        {
            prepared_initializer_member(state, expression)?.map(|member| member.ty.clone())
        }
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
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            state.prepared_call_output(expression)?
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
            state
                .raw_call_symbol(expression, &qualified, type_arguments)
                .ok()
                .flatten()
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
    let element = storage_type(context, pointee, state.layouts, state.target_layout)?;
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

fn emit_value_field<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    receiver: &ScalarExpression,
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<EmitValue<'ctx>, String> {
    let nodes = state.body_call_nodes
        .ok_or_else(|| "call result has no proven caller-owned storage for field projection".to_owned())?;
    let field = nodes.checked_value_field(expression)
        .ok_or_else(|| "missing checked value field identity".to_owned())?;
    let ty = nodes.checked_expression_type(expression)
        .ok_or_else(|| "missing checked value field type".to_owned())?;
    let base = aggregate_transfer::emit_value_field_receiver(context, state, receiver, &field.structure, project)?;
    let pointer = resolved_field_pointer(context, state, base, field)?;
    load_place_value(context, state, pointer, ty)
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
    let checked = state.body_call_nodes
        .and_then(|nodes| nodes.checked_expression_type(expression))
        .or_else(|| state.initializer_nodes
            .and_then(|nodes| nodes.checked_integer_conversion_source(expression)));
    if let Some(ty) = checked.filter(|ty| integer_width(ty).is_some()) {
        return Ok(ty.clone());
    }
    // Detached legacy emission may not have checked occurrence facts.
    let ty = match expression {
        ScalarExpression::Member { .. }
            if state.prepared_render && state.initializer_nodes.is_some() =>
        {
            prepared_initializer_member(state, expression)?.map(|member| member.ty.clone())
        }
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
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            state.prepared_call_output(expression)?
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
            state
                .raw_call_symbol(expression, &qualified, type_arguments)
                .ok()
                .flatten()
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
    if let Some(ty) = checked_float_type(state, expression) {
        return Ok(ty.clone());
    }
    let ty = match expression {
        ScalarExpression::Member { .. }
            if state.prepared_render && state.initializer_nodes.is_some() =>
        {
            prepared_initializer_member(state, expression)?.map(|member| member.ty.clone())
        }
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
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            state.prepared_call_output(expression)?
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
            state
                .raw_call_symbol(expression, &qualified, type_arguments)
                .ok()
                .flatten()
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
    if function.get_linkage() == Linkage::Internal {
        return Err(format!(
            "closed aggregate callable {name} requires its bound transfer"
        ));
    }
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
    let owner_slots = stage_checked_return_sidecars(context, state, owner_outputs, &mut arguments)?;
    let call = state
        .builder
        .build_call(function, &arguments, "call")
        .map_err(builder_error)?;
    publish_checked_return_sidecars(context, state, owner_slots)?;
    let outputs = match state.signatures.get(name) {
        Some(ScalarType::Callable { outputs, .. }) => outputs,
        _ => return Err(format!("unknown LLVM callable signature {name}")),
    };
    let returned = call.try_as_basic_value();
    if let Some(BasicTypeEnum::ArrayType(expected)) = function.get_type().get_return_type() {
        let (ValueKind::Basic(BasicValueEnum::ArrayValue(bytes)), [output]) =
            (returned, outputs.outputs.as_slice()) else {
                return Err("fixed copy callable result has an invalid ABI".into());
            };
        let storage = storage_type(context, &output.ty, state.layouts, state.target_layout)?;
        if !owner_outputs.is_empty() || !matches!(output.ty, ScalarType::Struct(_)) || bytes.get_type() != expected
            || storage != BasicTypeEnum::ArrayType(expected)
        {
            return Err("inline callable result does not match its declared struct layout".into());
        }
        let slot = entry_storage_alloca(context, state, &output.ty, "inline_result_storage")?;
        state.builder.build_store(slot, bytes).map_err(builder_error)?;
        return Ok(EmitValue::Basic(slot.into()));
    }
    ordinary_call_result(returned, outputs)
}

pub(super) fn stage_checked_return_sidecars<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    owner_outputs: &BTreeSet<usize>,
    arguments: &mut Vec<BasicMetadataValueEnum<'ctx>>,
) -> Result<BTreeMap<usize, PointerValue<'ctx>>, String> {
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
    Ok(owner_slots)
}

pub(super) fn publish_checked_return_sidecars<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    owner_slots: BTreeMap<usize, PointerValue<'ctx>>,
) -> Result<(), String> {
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
    Ok(())
}

pub(super) fn ordinary_call_result<'ctx>(
    value: ValueKind<'ctx>,
    outputs: &crate::ScalarOutputSequence,
) -> Result<EmitValue<'ctx>, String> {
    match (value, outputs.outputs.as_slice()) {
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
    emit_binary_expression(context, state, operator, left, right, ExpressionPath::Single)
}

fn prepared_initializer_unsigned(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
) -> Result<Option<bool>, String> {
    match expression {
        ScalarExpression::Member { .. } => Ok(prepared_initializer_member(state, expression)?.map(
            |member| {
                matches!(
                    member.ty,
                    ScalarType::U8
                        | ScalarType::U16
                        | ScalarType::U32
                        | ScalarType::U64
                        | ScalarType::U128
                )
            },
        )),
        ScalarExpression::Unary { operand, .. } => prepared_initializer_unsigned(state, operand),
        ScalarExpression::Binary { left, .. } => prepared_initializer_unsigned(state, left),
        _ => Ok(None),
    }
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
) -> bool {
    match expression {
        ScalarExpression::Member { receiver, name, .. } => {
            let names = state.project_names.expect("project emission names");
            if let Ok(target) = project_namespace_target(names, module, receiver) {
                return target.members.get(name.as_str()).is_some_and(|member| member.unsigned);
            }
            expression_is_unsigned(state, expression)
        }
        ScalarExpression::Unary { operand, .. } => {
            project_expression_is_unsigned(state, operand, module)
        }
        ScalarExpression::Binary { left, .. } => {
            project_expression_is_unsigned(state, left, module)
        }
        _ => expression_is_unsigned(state, expression),
    }
}

#[derive(Clone, Copy)]
pub(super) enum ExpressionPath<'a> {
    Single,
    Project {
        module: &'a ScalarModule,
        modules: &'a [&'a ScalarModule],
    },
}

impl ExpressionPath<'_> {
    fn emit<'ctx, 'module>(
        self,
        context: &'ctx Context,
        state: &mut EmitState<'ctx, 'module>,
        expression: &ScalarExpression,
    ) -> Result<EmitValue<'ctx>, String> {
        match self {
            Self::Single => emit_expression(context, state, expression),
            Self::Project { module, modules } => {
                emit_project_expression(context, state, expression, module, modules)
            }
        }
    }

    fn emit_block<'ctx, 'module>(
        self,
        context: &'ctx Context,
        state: &mut EmitState<'ctx, 'module>,
        block: &ScalarBlock,
        emission: BlockEmission<'_>,
    ) -> Result<EmitValue<'ctx>, String> {
        match self {
            Self::Single => emit_block_at(context, state, block, emission),
            Self::Project { module, modules } => {
                emit_project_block_at(context, state, block, module, modules, emission)
            }
        }
    }
}

fn checked_float_type<'a>(
    state: &EmitState<'_, 'a>,
    expression: &ScalarExpression,
) -> Option<&'a ScalarType> {
    state
        .body_call_nodes
        .and_then(|nodes| nodes.checked_expression_type(expression))
        .filter(|ty| matches!(ty, ScalarType::F32 | ScalarType::F64))
        .or_else(|| {
            state
                .initializer_nodes
                .and_then(|nodes| nodes.checked_float_type(expression))
        })
}

pub(super) fn emit_fallible_numeric_to<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression, payload: &ScalarType, path: ExpressionPath<'_>,
    failure: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<(BasicValueEnum<'ctx>, Vec<inkwell::basic_block::BasicBlock<'ctx>>), String> {
    use inkwell::basic_block::BasicBlock;
    enum Step<'ctx, 'ast> {
        Emit(&'ast ScalarExpression, &'ast ScalarType),
        Binary(&'ast BinaryOperator, &'ast ScalarType),
        Unary(&'ast crate::scalar::UnaryOperator),
        Call(&'ast ScalarExpression, usize),
        BeginShort { rhs: &'ast ScalarExpression, is_and: bool },
        EndShort { skipped: BasicBlock<'ctx>, join: BasicBlock<'ctx>, is_and: bool },
        BeginIf { left: &'ast ScalarExpression, right: &'ast ScalarExpression, ty: &'ast ScalarType },
        BeginElse { value: &'ast ScalarExpression, ty: &'ast ScalarType, block: BasicBlock<'ctx>, join: BasicBlock<'ctx> },
        EndElse { then_value: BasicValueEnum<'ctx>, then_end: BasicBlock<'ctx>, join: BasicBlock<'ctx> },
    }
    let function = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or("missing fallible expression function")?;
    let mut absent = Vec::new();
    let take_present = |state: &mut EmitState<'ctx, '_>, tagged: inkwell::values::StructValue<'ctx>,
        ty: &ScalarType, absent: &mut Vec<BasicBlock<'ctx>>| -> Result<EmitValue<'ctx>, String> {
        let pair = aggregate_transfer::fallible_allocation_result_type(context, ty)?;
        if tagged.get_type() != pair { return Err("invalid sealed fallible numeric payload".into()); }
        let test = state.builder.get_insert_block().ok_or("missing fallible operand test block")?;
        let present = context.append_basic_block(function, "fallible.binary_present");
        let succeeded = state.builder.build_extract_value(tagged, 0, "fallible.operand_succeeded")
            .map_err(builder_error)?.into_int_value();
        state.builder.build_conditional_branch(succeeded, present, failure).map_err(builder_error)?;
        absent.push(test);
        state.builder.position_at_end(present);
        let value = state.builder.build_extract_value(tagged, 1, "fallible.operand_payload")
            .map_err(builder_error)?;
        Ok(EmitValue::Basic(value))
    };
    let mut values = Vec::new();
    let mut stack = vec![Step::Emit(expression, payload)];
    while let Some(step) = stack.pop() {
        let (node, ty) = match step {
            Step::Emit(node, ty) => (node, ty),
            Step::Binary(operator, ty) => {
                let right = values.pop().ok_or("missing fallible binary right operand")?;
                let left = values.pop().ok_or("missing fallible binary left operand")?;
                values.push(emit_binary_values(state, operator, left, right, *ty == ScalarType::U64)?);
                continue;
            }
            Step::Unary(operator) => {
                let value = take_basic(values.pop().ok_or("missing fallible unary operand")?)?;
                values.push(emit_unary_value(state, operator, value)?);
                continue;
            }
            Step::Call(call, arity) => {
                let keep = values.len().checked_sub(arity).ok_or("missing sealed staged call arguments")?;
                let arguments = values.split_off(keep);
                let Some(PreparedCall::Symbol(target)) = state.prepared_call(call)? else {
                    return Err("missing sealed direct staged call target".into());
                };
                if !arguments.iter().all(|argument|
                    matches!(argument, EmitValue::Basic(BasicValueEnum::IntValue(value))
                        if value.get_type() == context.i64_type()))
                { return Err("invalid sealed staged call arguments".into()); }
                let value = if aggregate_transfer::fallible_allocation_call_at(state, call) {
                    let tagged = emit_sealed_allocation_call_values(context, state, &target, &arguments)?;
                    take_present(state, tagged, &ScalarType::U64, &mut absent)?
                } else {
                    let callee = state.functions.get(target.as_ref()).ok_or("missing exact ordinary staged call")?;
                    let types = vec![context.i64_type().into(); arity];
                    if callee.get_type() != context.i64_type().fn_type(&types, false) {
                        return Err("invalid sealed ordinary staged call ABI".into());
                    }
                    emit_call_values(context, state, &target, arguments)?
                };
                values.push(value);
                continue;
            }
            Step::BeginShort { rhs, is_and } => {
                let condition = take_basic(values.pop().ok_or("missing short-circuit left operand")?)?.into_int_value();
                let skipped = state.builder.get_insert_block().ok_or("missing short-circuit left block")?;
                let rhs_block = context.append_basic_block(function, "short_circuit.rhs");
                let join = context.append_basic_block(function, "short_circuit.merge");
                let (yes, no) = if is_and { (rhs_block, join) } else { (join, rhs_block) };
                state.builder.build_conditional_branch(condition, yes, no).map_err(builder_error)?;
                state.builder.position_at_end(rhs_block);
                stack.push(Step::EndShort { skipped, join, is_and });
                stack.push(Step::Emit(rhs, &ScalarType::Bool));
                continue;
            }
            Step::EndShort { skipped, join, is_and } => {
                let rhs = take_basic(values.pop().ok_or("missing short-circuit right operand")?)?.into_int_value();
                let rhs_end = state.builder.get_insert_block().ok_or("missing short-circuit right block")?;
                state.builder.build_unconditional_branch(join).map_err(builder_error)?;
                state.builder.position_at_end(join);
                let phi = state.builder.build_phi(context.bool_type(), "short_circuit.value").map_err(builder_error)?;
                let skip_value = context.bool_type().const_int(u64::from(!is_and), false);
                phi.add_incoming(&[(&skip_value, skipped), (&rhs, rhs_end)]);
                values.push(EmitValue::Basic(phi.as_basic_value()));
                continue;
            }
            Step::BeginIf { left, right, ty } => {
                let condition = take_basic(values.pop().ok_or("missing fallible if predicate")?)?.into_int_value();
                let then_block = context.append_basic_block(function, "if.then");
                let else_block = context.append_basic_block(function, "if.else");
                let join = context.append_basic_block(function, "if.merge");
                state.builder.build_conditional_branch(condition, then_block, else_block).map_err(builder_error)?;
                state.builder.position_at_end(then_block);
                stack.push(Step::BeginElse { value: right, ty, block: else_block, join });
                stack.push(Step::Emit(left, ty));
                continue;
            }
            Step::BeginElse { value, ty, block, join } => {
                let then_value = take_basic(values.pop().ok_or("missing fallible then value")?)?;
                let then_end = state.builder.get_insert_block().ok_or("missing fallible then block")?;
                state.builder.build_unconditional_branch(join).map_err(builder_error)?;
                state.builder.position_at_end(block);
                stack.push(Step::EndElse { then_value, then_end, join });
                stack.push(Step::Emit(value, ty));
                continue;
            }
            Step::EndElse { then_value, then_end, join } => {
                let else_value = take_basic(values.pop().ok_or("missing fallible else value")?)?;
                let else_end = state.builder.get_insert_block().ok_or("missing fallible else block")?;
                state.builder.build_unconditional_branch(join).map_err(builder_error)?;
                state.builder.position_at_end(join);
                if then_value.get_type() != else_value.get_type() {
                    return Err("sealed fallible conditional has divergent payload types".into());
                }
                let phi = state.builder.build_phi(then_value.get_type(), "fallible.selected_payload")
                    .map_err(builder_error)?;
                phi.add_incoming(&[(&then_value, then_end), (&else_value, else_end)]);
                values.push(EmitValue::Basic(phi.as_basic_value()));
                continue;
            }
        };
        match node {
            ScalarExpression::Unary { operator, operand, .. }
                if crate::scalar::fallible_total_unary_bound(ty, operator).is_some() =>
            {
                stack.push(Step::Unary(operator));
                stack.push(Step::Emit(operand, ty));
            }
            ScalarExpression::Binary { operator: operator @ (BinaryOperator::And | BinaryOperator::Or), left, right, .. } => {
                stack.push(Step::BeginShort { rhs: right, is_and: *operator == BinaryOperator::And });
                stack.push(Step::Emit(left, &ScalarType::Bool));
            }
            ScalarExpression::Binary { operator, left, right, .. } => {
                stack.push(Step::Binary(operator, ty));
                stack.push(Step::Emit(right, ty));
                stack.push(Step::Emit(left, ty));
            }
            ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                let [ScalarBlockItem::Expression(then_value)] = then_branch.items.as_slice()
                    else { return Err("missing sealed fallible then expression".into()); };
                let [ScalarBlockItem::Expression(else_value)] = else_branch.items.as_slice()
                    else { return Err("missing sealed fallible else expression".into()); };
                stack.push(Step::BeginIf { left: then_value, right: else_value, ty });
                stack.push(Step::Emit(condition, &ScalarType::Bool));
            }
            ScalarExpression::Call { arguments, .. } if state.body_call_nodes.is_some_and(|nodes|
                nodes.prepared_call_has_callful_arguments(node)) => {
                    if *ty != ScalarType::U64 { return Err("invalid sealed staged call result type".into()); }
                    stack.push(Step::Call(node, arguments.len()));
                    for argument in arguments.iter().rev() { stack.push(Step::Emit(argument, &ScalarType::U64)); }
                }
            ScalarExpression::Call { .. } if aggregate_transfer::fallible_allocation_call_at(state, node) => {
                let BasicValueEnum::StructValue(result) = take_basic(path.emit(context, state, node)?)? else {
                    return Err("missing sealed fallible operand result".into());
                };
                values.push(take_present(state, result, ty, &mut absent)?);
            }
            ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. } | ScalarExpression::Name { .. } | ScalarExpression::Call { .. } =>
                values.push(emit_contextual_numeric(context, state, node, ty, path)?),
            _ => return Err("invalid sealed fallible expression node".into()),
        }
    }
    if values.len() != 1 || absent.is_empty() { return Err("invalid sealed fallible expression result".into()); }
    Ok((take_basic(values.pop().expect("one sealed fallible expression output"))?, absent))
}

fn emit_fallible_numeric<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression, payload: &ScalarType, path: ExpressionPath<'_>, ordinary: bool,
) -> Result<EmitValue<'ctx>, String> {
    let function = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or("missing fallible expression function")?;
    let merge = context.append_basic_block(function,
        if ordinary { "ordinary.allocation_absent" } else { "fallible.binary_merge" });
    let (value, absent) = emit_fallible_numeric_to(context, state, expression, payload, path, merge)?;
    if ordinary {
        let success = state.builder.get_insert_block().ok_or("missing ordinary expression success block")?;
        state.builder.position_at_end(merge);
        emit_terminal_system_panic(context, state)?;
        state.builder.position_at_end(success);
        return Ok(EmitValue::Basic(value));
    }
    let computed = aggregate_transfer::present_fallible_allocation_result(context, state, value, payload)?;
    let computed_end = state.builder.get_insert_block().ok_or("missing fallible expression present block")?;
    state.builder.build_unconditional_branch(merge).map_err(builder_error)?;
    state.builder.position_at_end(merge);
    let phi = state.builder.build_phi(computed.get_type(), "fallible.binary_result").map_err(builder_error)?;
    let zero = computed.get_type().const_zero();
    for test in absent { phi.add_incoming(&[(&zero, test)]); }
    phi.add_incoming(&[(&computed, computed_end)]);
    Ok(EmitValue::Basic(phi.as_basic_value()))
}

pub(super) fn emit_contextual_numeric<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    expected: &ScalarType,
    path: ExpressionPath<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let ordinary = aggregate_transfer::ordinary_fallible_allocation_expression_at(state, expression);
    if ordinary || aggregate_transfer::fallible_allocation_expression_at(state, expression) {
        return emit_fallible_numeric(context, state, expression, expected, path, ordinary);
    }
    let float = matches!(expected, ScalarType::F32 | ScalarType::F64);
    let integer = !float && integer_width(expected).is_some();
    match expression {
        ScalarExpression::Integer { value, .. } if integer => {
            Ok(EmitValue::Basic(integer_constant(context, expected, value)?.into()))
        }
        ScalarExpression::Float { value, .. } if float => {
            Ok(EmitValue::Basic(float_constant(context, expected, *value)?.into()))
        }
        ScalarExpression::Unary { operator, operand, .. }
            if matches!(operator, crate::scalar::UnaryOperator::Negate) && (float || integer)
                || matches!(operator, crate::scalar::UnaryOperator::BitwiseNot) && integer =>
        {
            let operand = take_basic(emit_contextual_numeric(context, state, operand, expected, path)?)?;
            emit_unary_value(state, operator, operand)
        }
        ScalarExpression::Binary { operator, left, right, .. }
            if (float || integer) && matches!(operator, BinaryOperator::Add | BinaryOperator::Subtract
                | BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Remainder)
                || integer && matches!(operator, BinaryOperator::BitAnd | BinaryOperator::BitOr
                    | BinaryOperator::BitXor | BinaryOperator::ShiftLeft | BinaryOperator::ShiftRight) =>
        {
            let left = emit_contextual_numeric(context, state, left, expected, path)?;
            let right = emit_contextual_numeric(context, state, right, expected, path)?;
            let unsigned = matches!(expected, ScalarType::U8 | ScalarType::U16 | ScalarType::U32
                | ScalarType::U64 | ScalarType::U128);
            emit_binary_values(state, operator, left, right, unsigned)
        }
        ScalarExpression::If { condition, then_branch, else_branch, .. } => {
            let condition = take_basic(path.emit(context, state, condition)?)?.into_int_value();
            emit_if_value(context, state, condition, then_branch, else_branch, expression, |state, block| {
                path.emit_block(context, state, block, BlockEmission::ContextualNumeric(expected))
            })
        }
        ScalarExpression::Block(block) => {
            path.emit_block(context, state, block, BlockEmission::ContextualNumeric(expected))
        }
        _ => path.emit(context, state, expression),
    }
}

fn emit_binary_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: &ScalarExpression,
    right: &ScalarExpression,
    path: ExpressionPath<'_>,
) -> Result<EmitValue<'ctx>, String> {
    if let ScalarExpression::Float { value, .. } = left {
        if checked_float_type(state, left).is_none()
            && !matches!(operator, BinaryOperator::And | BinaryOperator::Or)
        {
            let right = path.emit(context, state, right)?;
            let EmitValue::Basic(BasicValueEnum::FloatValue(right)) = right else {
                return Err("floating-point literal requires an expected type".to_owned());
            };
            let ty = if right.get_type() == context.f32_type() { ScalarType::F32 } else { ScalarType::F64 };
            let left = EmitValue::Basic(float_constant(context, &ty, *value)?.into());
            return emit_binary_values(state, operator, left, EmitValue::Basic(right.into()), false);
        }
    }
    let checked_call_signedness = if matches!(
        operator, BinaryOperator::Less | BinaryOperator::LessEqual
            | BinaryOperator::Greater | BinaryOperator::GreaterEqual
    ) && matches!(left, ScalarExpression::Call { .. }) {
        // Ordered calls in prepared function bodies have their result type
        // recorded on the exact expression, independent of source spelling.
        let ty = state.body_call_nodes.and_then(|nodes| nodes.checked_expression_type(left));
        match ty {
            Some(ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128) => Some(true),
            Some(ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128) => Some(false),
            _ => None,
        }
    } else {
        None
    };
    let unsigned = match checked_call_signedness.or(prepared_initializer_unsigned(state, left)?) {
        Some(unsigned) => unsigned,
        None => match path {
            ExpressionPath::Single => expression_is_unsigned(state, left),
            ExpressionPath::Project { module, .. } => project_expression_is_unsigned(state, left, module),
        },
    };
    let left = path.emit(context, state, left)?;
    emit_binary_with_rhs(context, state, operator, left, right, path, unsigned)
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

pub(super) fn emit_binary_values<'ctx, 'module>(
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
    value: BasicValueEnum<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    use crate::scalar::UnaryOperator::{BitwiseNot, LogicalNot, Negate};
    let result = match (operator, value) {
        (LogicalNot | BitwiseNot, BasicValueEnum::IntValue(value)) => state.builder
            .build_not(value, "not")
            .map(BasicValueEnum::from),
        (Negate, BasicValueEnum::IntValue(value)) => state.builder
            .build_int_sub(value.get_type().const_zero(), value, "neg")
            .map(BasicValueEnum::from),
        (Negate, BasicValueEnum::FloatValue(value)) => state.builder
            .build_float_neg(value, "neg")
            .map(BasicValueEnum::from),
        _ => return Err("invalid unary LLVM operand".to_owned()),
    };
    result.map(EmitValue::Basic).map_err(builder_error)
}

fn emit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    condition: &ScalarExpression,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(emit_expression(context, state, condition)?)?.into_int_value();
    emit_if_value_with_route(
        context,
        state,
        condition,
        then_branch,
        else_branch,
        expression,
        true,
        |state, block| emit_block(context, state, block),
    )
}
fn emit_project_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    condition: EmitValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(condition)?.into_int_value();
    emit_if_value_with_route(
        context,
        state,
        condition,
        then_branch,
        else_branch,
        expression,
        true,
        |state, block| emit_project_block(context, state, block, module, modules),
    )
}

fn emit_unit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    condition: &ScalarExpression,
    then_branch: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(emit_expression(context, state, condition)?)?.into_int_value();
    let bypass = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    let function = bypass.get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    let enrolled = aggregate_transfer::begin_literal_array_unit(
        state, expression, then_branch, None, crate::scalar::ArrayWriterSide::Then,
    )?;
    emit_block(context, state, then_branch)?;
    let then_end = state.builder.get_insert_block().ok_or_else(|| "missing if arm terminal".to_owned())?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge);
    aggregate_transfer::finish_conditional_receipts(context, state, expression, then_end, bypass)?;
    aggregate_transfer::finish_literal_array_unit(state, enrolled)?;
    Ok(EmitValue::Unit)
}

fn emit_project_unit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    condition: EmitValue<'ctx>,
    then_branch: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(condition)?.into_int_value();
    let bypass = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    let function = bypass.get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    let enrolled = aggregate_transfer::begin_literal_array_unit(
        state, expression, then_branch, None, crate::scalar::ArrayWriterSide::Then,
    )?;
    emit_project_block(context, state, then_branch, module, modules)?;
    let then_end = state.builder.get_insert_block().ok_or_else(|| "missing if arm terminal".to_owned())?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge);
    aggregate_transfer::finish_conditional_receipts(context, state, expression, then_end, bypass)?;
    aggregate_transfer::finish_literal_array_unit(state, enrolled)?;
    Ok(EmitValue::Unit)
}

pub(super) fn emit_if_value<'ctx, 'module, F>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: inkwell::values::IntValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    expression: &ScalarExpression,
    emit: F,
) -> Result<EmitValue<'ctx>, String>
where
    F: Fn(&mut EmitState<'ctx, 'module>, &ScalarBlock) -> Result<EmitValue<'ctx>, String>,
{
    emit_if_value_with_route(context, state, condition, then_branch, else_branch, expression, false, emit)
}

fn emit_if_value_with_route<'ctx, 'module, F>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: inkwell::values::IntValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    expression: &ScalarExpression,
    literal_array_route: bool,
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
    let then_writer = literal_array_route && aggregate_transfer::begin_literal_array_unit(
        state, expression, then_branch, Some(else_branch), crate::scalar::ArrayWriterSide::Then,
    )?;
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
    let else_writer = literal_array_route && aggregate_transfer::begin_literal_array_unit(
        state, expression, then_branch, Some(else_branch), crate::scalar::ArrayWriterSide::Else,
    )?;
    state.return_owner_flags = incoming_owners.clone();
    let else_value = emit(state, else_branch)?;
    let else_owners = state.return_owner_flags.clone();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let else_end = state.builder.get_insert_block().expect("else block");
    state.builder.position_at_end(merge);
    aggregate_transfer::finish_conditional_receipts(context, state, expression, then_end, else_end)?;
    let enrolled = then_writer || else_writer;
    if enrolled && !matches!((&then_value, &else_value), (EmitValue::Unit, EmitValue::Unit)) {
        return Err("checked array conditional did not have two Unit arms".into());
    }
    aggregate_transfer::finish_literal_array_unit(state, enrolled)?;
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
