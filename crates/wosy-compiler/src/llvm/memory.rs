use super::expression::{
    emit_expression, emit_project_expression, emit_project_typed_expression, emit_unary_value,
};
use super::*;

pub(super) fn integer_width(ty: &ScalarType) -> Option<u32> {
    match ty {
        ScalarType::I8 | ScalarType::U8 => Some(8),
        ScalarType::I16 | ScalarType::U16 => Some(16),
        ScalarType::I32 | ScalarType::U32 => Some(32),
        ScalarType::I64 | ScalarType::U64 => Some(64),
        ScalarType::I128 | ScalarType::U128 => Some(128),
        _ => None,
    }
}

pub(super) fn integer_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<inkwell::types::IntType<'ctx>, String> {
    let width = integer_width(ty).ok_or_else(|| "expected integer type".to_owned())?;
    context
        .custom_width_int_type(NonZeroU32::new(width).expect("integer width is nonzero"))
        .map_err(str::to_owned)
}

pub(super) fn integer_scalar_type(width: u32) -> Result<ScalarType, String> {
    match width {
        8 => Ok(ScalarType::U8),
        16 => Ok(ScalarType::U16),
        32 => Ok(ScalarType::U32),
        64 => Ok(ScalarType::U64),
        128 => Ok(ScalarType::U128),
        _ => Err(format!("unsupported LLVM integer width {width}")),
    }
}

pub(super) fn pointer_integer_type<'ctx>(
    context: &'ctx Context,
    target_layout: ScalarTargetLayout,
) -> inkwell::types::IntType<'ctx> {
    context
        .custom_width_int_type(
            NonZeroU32::new((target_layout.pointer_size * 8) as u32)
                .expect("pointer width is nonzero"),
        )
        .expect("supported pointer width")
}

pub(super) fn basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Bool => Ok(context.bool_type().into()),
        ty if integer_width(ty).is_some() => Ok(integer_type(context, ty)?.into()),
        ScalarType::Char => Ok(context.i32_type().into()),
        ScalarType::Enum(_) => Ok(context.i32_type().into()),
        ScalarType::F32 => Ok(context.f32_type().into()),
        ScalarType::F64 => Ok(context.f64_type().into()),
        ScalarType::ArtifactId => Ok(context.ptr_type(AddressSpace::default()).into()),
        ScalarType::Callable { .. } => Ok(context
            .struct_type(
                &[
                    context.ptr_type(AddressSpace::default()).into(),
                    context.ptr_type(AddressSpace::default()).into(),
                ],
                false,
            )
            .into()),
        ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. } => {
            Ok(pointer_integer_type(context, target_layout).into())
        }
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. } => {
            Ok(context.ptr_type(AddressSpace::default()).into())
        }
        _ => Err("unit is only valid as a function result".into()),
    }
}

pub(super) fn storage_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Struct(id) => {
            let structure = structs
                .iter()
                .find(|structure| structure.id == *id)
                .ok_or_else(|| format!("unknown LLVM struct {}", id.index))?;
            let layout = structure
                .layout
                .as_ref()
                .ok_or_else(|| format!("LLVM struct {} has no valid layout", id.index))?;
            let size =
                u32::try_from(layout.size).map_err(|_| "struct layout exceeds LLVM array size")?;
            Ok(context.i8_type().array_type(size).into())
        }
        ScalarType::Array {
            element, length, ..
        } => Ok(storage_type(context, element, structs, target_layout)?
            .array_type(
                u32::try_from(*length).map_err(|_| "fixed array length exceeds LLVM array size")?,
            )
            .into()),
        _ => basic_type(context, ty, target_layout),
    }
}

pub(super) fn allocation_layout(
    ty: &ScalarType,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<(u64, u64), String> {
    match ty {
        ScalarType::Bool | ScalarType::I8 | ScalarType::U8 => Ok((1, 1)),
        ScalarType::I16 | ScalarType::U16 => Ok((2, 2)),
        ScalarType::I32 | ScalarType::U32 | ScalarType::Char | ScalarType::F32 => Ok((4, 4)),
        ScalarType::I64 | ScalarType::U64 | ScalarType::F64 => Ok((8, 8)),
        ScalarType::I128 | ScalarType::U128 => Ok((16, 16)),
        ScalarType::RawPointer(_)
        | ScalarType::CheckedReference { .. }
        | ScalarType::ArtifactId => {
            Ok((target_layout.pointer_size, target_layout.pointer_alignment))
        }
        ScalarType::Callable { .. } => Ok((
            target_layout.pointer_size * 2,
            target_layout.pointer_alignment,
        )),
        ScalarType::Enum(_) => Ok((4, 4)),
        ScalarType::Struct(id) => structs
            .iter()
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.layout.as_ref())
            .map(|layout| (layout.size, layout.alignment))
            .ok_or_else(|| format!("runtime allocation has no layout for struct {}", id.index)),
        ScalarType::Array {
            element, length, ..
        } => {
            let (size, alignment) = allocation_layout(element, structs, target_layout)?;
            let size = size
                .checked_mul(*length)
                .ok_or_else(|| "runtime allocation size overflows u64".to_owned())?;
            Ok((size, alignment))
        }
        ScalarType::Named { .. }
        | ScalarType::Qualified { .. }
        | ScalarType::RuntimeArray { .. }
        | ScalarType::Unit
        | ScalarType::Error => Err("runtime allocation requires a sized element type".to_owned()),
    }
}

pub(super) fn has_callable_holders(
    ty: &ScalarType,
    structs: &[ScalarStruct],
) -> Result<bool, String> {
    match ty {
        ScalarType::Callable { .. } => Ok(true),
        ScalarType::Array { element, .. } => has_callable_holders(element, structs),
        ScalarType::Struct(id) => {
            let structure = structure(structs, id.clone())?;
            for field in &structure.fields {
                if has_callable_holders(&field.ty, structs)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}
pub(super) fn value_type(ty: &ScalarType) -> Result<LlvmValueType, String> {
    match ty {
        ScalarType::Unit => Ok(LlvmValueType::Void),
        ScalarType::Bool => Ok(LlvmValueType::I1),
        ScalarType::I8 | ScalarType::U8 => Ok(LlvmValueType::I8),
        ScalarType::I16 | ScalarType::U16 => Ok(LlvmValueType::I16),
        ScalarType::I32 | ScalarType::U32 => Ok(LlvmValueType::I32),
        ScalarType::I64 | ScalarType::U64 => Ok(LlvmValueType::I64),
        ScalarType::I128 | ScalarType::U128 => Ok(LlvmValueType::I128),
        ScalarType::F32 => Ok(LlvmValueType::F32),
        ScalarType::F64 => Ok(LlvmValueType::F64),
        ScalarType::Char => Ok(LlvmValueType::Char),
        ScalarType::ArtifactId => Ok(LlvmValueType::ArtifactId),
        ScalarType::Callable { .. } => Ok(LlvmValueType::Aggregate(vec![
            LlvmValueType::Pointer,
            LlvmValueType::Pointer,
        ])),
        ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. } => {
            Ok(LlvmValueType::Pointer)
        }
        ScalarType::Enum(_) => Ok(LlvmValueType::I32),
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. } => {
            Ok(LlvmValueType::Pointer)
        }
        _ => Err("unsupported LLVM scalar type".into()),
    }
}

pub(super) fn integer_constant<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    value: &num_bigint::BigInt,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let integer = integer_type(context, ty)?;
    let (sign, words) = value.to_u64_digits();
    if sign == num_bigint::Sign::Minus {
        return Err("negative integer literal is invalid".to_owned());
    }
    if integer.get_bit_width() <= 64 {
        return Ok(match words.first() {
            Some(word) => integer.const_int(*word, false),
            None => integer.const_zero(),
        });
    }
    Ok(integer.const_int_arbitrary_precision(&words))
}

pub(super) fn float_constant<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    value: f64,
) -> Result<inkwell::values::FloatValue<'ctx>, String> {
    match ty {
        ScalarType::F32 => Ok(context.f32_type().const_float(value)),
        ScalarType::F64 => Ok(context.f64_type().const_float(value)),
        _ => Err("expected floating-point type".to_owned()),
    }
}

pub(super) fn aggregate_type<'ctx>(
    context: &'ctx Context,
    outputs: &crate::ScalarOutputSequence,
    target_layout: ScalarTargetLayout,
) -> Result<StructType<'ctx>, String> {
    let fields = outputs
        .outputs
        .iter()
        .map(|output| output_basic_type(context, &output.ty, target_layout))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(context.struct_type(&fields, false))
}

fn output_basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    if *ty == ScalarType::Unit {
        Ok(context.i8_type().into())
    } else {
        basic_type(context, ty, target_layout)
    }
}

pub(super) fn builder_error(error: BuilderError) -> String {
    error.to_string()
}

pub(super) fn entry_alloca<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    ty: BasicTypeEnum<'ctx>,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    let insert = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    let function = insert
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let entry = function
        .get_first_basic_block()
        .ok_or_else(|| "missing entry block".to_owned())?;
    if let Some(first) = entry.get_first_instruction() {
        state.builder.position_before(&first);
    } else {
        state.builder.position_at_end(entry);
    }
    let slot = state
        .builder
        .build_alloca(ty, name)
        .map_err(builder_error)?;
    state.builder.position_at_end(insert);
    Ok(slot)
}

fn entry_selected_slot<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    ty: BasicTypeEnum<'ctx>,
    initial: BasicValueEnum<'ctx>,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    let insert = state
        .builder
        .get_insert_block()
        .expect("selected element insertion block");
    let entry = insert
        .get_parent()
        .expect("selected element function")
        .get_first_basic_block()
        .expect("selected element entry");
    let first = entry.get_first_instruction();
    let slot = entry_alloca(state, ty, name)?;
    if let Some(first) = first {
        state.builder.position_before(&first);
    } else {
        state.builder.position_at_end(entry);
    }
    state
        .builder
        .build_store(slot, initial)
        .map_err(builder_error)?;
    state.builder.position_at_end(insert);
    Ok(slot)
}

pub(super) fn record_selected_element<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    point: crate::scalar::CfgPointId,
    index_point: crate::scalar::CfgPointId,
    index: inkwell::values::IntValue<'ctx>,
    address: PointerValue<'ctx>,
) -> Result<(), String> {
    if !state.holder_events.selected.contains_key(&point) {
        let pointer_type = context.ptr_type(AddressSpace::default());
        let index_slot = entry_selected_slot(
            state,
            context.i64_type().into(),
            context.i64_type().const_zero().into(),
            "selected_index_slot",
        )?;
        let address_slot = entry_selected_slot(
            state,
            pointer_type.into(),
            pointer_type.const_null().into(),
            "selected_address_slot",
        )?;
        let present_slot = entry_selected_slot(
            state,
            context.bool_type().into(),
            context.bool_type().const_zero().into(),
            "selected_present_slot",
        )?;
        state.holder_events.selected.insert(
            point,
            super::holder_events::SelectedElementSlots {
                index_point,
                index: index_slot,
                address: address_slot,
                present: present_slot,
            },
        );
    }
    let slots = &state.holder_events.selected[&point];
    if slots.index_point != index_point {
        return Err(format!(
            "selected point {point:?} changed evaluated index point"
        ));
    }
    state
        .builder
        .build_store(slots.index, index)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.address, address)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.present, context.bool_type().const_int(1, false))
        .map_err(builder_error)?;
    Ok(())
}

pub(super) fn selected_element<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    point: crate::scalar::CfgPointId,
    index_point: crate::scalar::CfgPointId,
) -> Result<
    (
        inkwell::values::IntValue<'ctx>,
        PointerValue<'ctx>,
        inkwell::values::IntValue<'ctx>,
    ),
    String,
> {
    let slots = state
        .holder_events
        .selected
        .get(&point)
        .ok_or_else(|| format!("missing emitted selected index {point:?}"))?;
    if slots.index_point != index_point {
        return Err(format!(
            "selected point {point:?} has a different evaluated index point"
        ));
    }
    let present = state
        .builder
        .build_load(context.bool_type(), slots.present, "selected_present")
        .map_err(builder_error)?
        .into_int_value();
    let index = state
        .builder
        .build_load(context.i64_type(), slots.index, "selected_index")
        .map_err(builder_error)?
        .into_int_value();
    let address = state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            slots.address,
            "selected_address",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    Ok((index, address, present))
}

pub(super) fn emit_callable_read<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let point = state.holder_events.read_place(expression)?.id;
    let ty = assignment_place_type(state, place)
        .ok_or_else(|| format!("missing callable read type at {point:?}"))?;
    let ScalarType::Callable { .. } = ty else {
        return Err(format!("read point {point:?} does not produce a callable"));
    };
    let selection = if matches!(place, crate::ScalarPlace::Index { .. }) {
        let binding = match &state.holder_events.read_place(expression)?.kind {
            CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1 => {
                candidates[0].binding.clone()
            }
            _ => return Err(format!("ReadPlace {point:?} has no typed indexed place")),
        };
        Some(evaluate_callable_selection(
            context, state, place, &binding, point, point, project,
        )?)
    } else {
        None
    };
    let direct_address = if selection.is_none() {
        Some(place_pointer(context, state, place, project)?)
    } else {
        None
    };
    super::ownership::begin_graph_mutation(&state.builder, state.module)?;
    let address = match selection {
        Some(selection) => resolve_callable_selection(context, state, &selection)?,
        None => direct_address.expect("non-indexed callable read address"),
    };
    let callable_type = basic_type(context, &ty, state.target_layout)?.into_struct_type();
    let callable = state
        .builder
        .build_load(callable_type, address, "read_callable")
        .map_err(builder_error)?
        .into_struct_value();
    if !state.holder_events.reads.contains_key(&point) {
        let pointer_type = context.ptr_type(AddressSpace::default());
        let callable_slot = entry_selected_slot(
            state,
            callable_type.into(),
            callable_type.const_zero().into(),
            "read_callable_slot",
        )?;
        let address_slot = entry_selected_slot(
            state,
            pointer_type.into(),
            pointer_type.const_null().into(),
            "read_address_slot",
        )?;
        let present_slot = entry_selected_slot(
            state,
            context.bool_type().into(),
            context.bool_type().const_zero().into(),
            "read_present_slot",
        )?;
        state.holder_events.reads.insert(
            point,
            super::holder_events::ReadPlaceSlots {
                callable: callable_slot,
                address: address_slot,
                present: present_slot,
            },
        );
    }
    let slots = &state.holder_events.reads[&point];
    state
        .builder
        .build_store(slots.callable, callable)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.address, address)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.present, context.bool_type().const_int(1, false))
        .map_err(builder_error)?;
    let edges = state
        .holder_events
        .read_place(expression)?
        .successors
        .clone();
    for to in edges {
        super::emit_connected_holder_actions_locked(
            context,
            state,
            super::holder_events::HolderEmissionEdge::Outgoing { from: point, to },
            None,
        )?;
    }
    super::ownership::end_graph_mutation(&state.builder, state.module)?;
    Ok(EmitValue::Basic(callable.into()))
}

pub(super) fn callable_read<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    point: crate::scalar::CfgPointId,
) -> Result<
    (
        StructValue<'ctx>,
        PointerValue<'ctx>,
        inkwell::values::IntValue<'ctx>,
    ),
    String,
> {
    let slots = state
        .holder_events
        .reads
        .get(&point)
        .ok_or_else(|| format!("missing emitted callable ReadPlace {point:?}"))?;
    let pointer_type = context.ptr_type(AddressSpace::default());
    let callable_type = context.struct_type(&[pointer_type.into(), pointer_type.into()], false);
    let callable = state
        .builder
        .build_load(callable_type, slots.callable, "cached_read_callable")
        .map_err(builder_error)?
        .into_struct_value();
    let address = state
        .builder
        .build_load(pointer_type, slots.address, "cached_read_address")
        .map_err(builder_error)?
        .into_pointer_value();
    let present = state
        .builder
        .build_load(context.bool_type(), slots.present, "cached_read_present")
        .map_err(builder_error)?
        .into_int_value();
    Ok((callable, address, present))
}

pub(super) fn record_return_output<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    output: &ScalarOutputValue,
    value: &EmitValue<'ctx>,
) -> Result<(), String> {
    if !matches!(
        output.ty,
        ScalarType::Callable { .. } | ScalarType::Struct(_)
    ) {
        return Ok(());
    }
    let point = state
        .holder_events
        .return_output(&state.source, output.span, output.position)?
        .id;
    let pointer_type = context.ptr_type(AddressSpace::default());
    if !state.holder_events.outputs.contains_key(&point) {
        let address = entry_selected_slot(
            state,
            pointer_type.into(),
            pointer_type.const_null().into(),
            "return_holder_address",
        )?;
        let present = entry_selected_slot(
            state,
            context.bool_type().into(),
            context.bool_type().const_zero().into(),
            "return_holder_present",
        )?;
        state.holder_events.outputs.insert(
            point,
            super::holder_events::ReturnOutputSlots {
                address,
                present,
                ty: output.ty.clone(),
            },
        );
    }
    let slots = &state.holder_events.outputs[&point];
    if slots.ty != output.ty {
        return Err(format!("ReturnOutput {point:?} changed type"));
    }
    let address = match &output.ty {
        ScalarType::Callable { .. } => {
            let callable = take_basic(value.clone())?.into_struct_value();
            let address = entry_alloca(state, callable.get_type().into(), "return_holder_value")?;
            state
                .builder
                .build_store(address, callable)
                .map_err(builder_error)?;
            address
        }
        ScalarType::Struct(_) => take_basic(value.clone())?.into_pointer_value(),
        _ => unreachable!(),
    };
    state
        .builder
        .build_store(slots.address, address)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.present, context.bool_type().const_int(1, false))
        .map_err(builder_error)?;
    Ok(())
}

pub(super) struct ConcreteCallableHolder<'ctx> {
    pub callable: StructValue<'ctx>,
    pub environment: PointerValue<'ctx>,
    pub address: PointerValue<'ctx>,
    pub present: inkwell::values::IntValue<'ctx>,
}

pub(super) fn constant_callable_projection_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::scalar::ReferencePlaceId,
) -> Result<PointerValue<'ctx>, String> {
    let root = state
        .capture_cells
        .get(&place.binding)
        .copied()
        .or_else(|| state.holder_events.bindings.get(&place.binding).copied())
        .ok_or_else(|| {
            format!("constant projection {place:?} lacks source-qualified binding storage")
        })?;
    let (name, root_ty) = state
        .storage
        .iter()
        .find(|(_, (address, _))| *address == root)
        .map(|(name, (_, ty))| (name.clone(), ty.clone()))
        .or_else(|| {
            state
                .globals
                .iter()
                .chain(state.all_globals.iter())
                .find(|(_, (global, _))| global.as_pointer_value() == root)
                .map(|(name, (_, ty))| (name.clone(), ty.clone()))
        })
        .ok_or_else(|| {
            format!("constant projection {place:?} lacks typed emitted binding storage")
        })?;
    let name = name;
    let mut ty = root_ty;
    let mut address = root;
    for projection in &place.projections {
        match projection {
            crate::scalar::ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(
                field,
            )) => {
                if !matches!(&ty, ScalarType::Struct(id) if *id == field.structure) {
                    return Err(format!(
                        "constant projection {place:?} has unrelated field {field:?}"
                    ));
                }
                let declared = structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == *field)
                    .ok_or_else(|| {
                        format!("constant projection {place:?} has unknown field {field:?}")
                    })?;
                let offset = declared
                    .offset
                    .ok_or_else(|| format!("constant projection {place:?} field lacks layout"))?;
                ty = declared.ty.clone();
                address = unsafe {
                    state.builder.build_in_bounds_gep(
                        context.i8_type(),
                        address,
                        &[context.i64_type().const_int(offset, false)],
                        "holder_field",
                    )
                }
                .map_err(builder_error)?;
            }
            crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer {
                value,
                ..
            }) => {
                let index = u64::try_from(value).map_err(|_| {
                    format!("constant projection {place:?} has an invalid u64 index")
                })?;
                let index_value = context.i64_type().const_int(index, false);
                let array_type = ty.clone();
                match ty {
                    ScalarType::Array {
                        element, length, ..
                    } => {
                        if index >= length {
                            return Err(format!("constant projection {place:?} index {index} exceeds fixed length {length}"));
                        }
                        let array =
                            storage_type(context, &array_type, state.structs, state.target_layout)?;
                        address = unsafe {
                            state.builder.build_in_bounds_gep(
                                array,
                                address,
                                &[context.i32_type().const_zero(), index_value],
                                "holder_constant_element",
                            )
                        }
                        .map_err(builder_error)?;
                        ty = *element;
                    }
                    ScalarType::RuntimeArray { element, .. } => {
                        let length_slot =
                            state.runtime_array_lengths.get(&name).ok_or_else(|| {
                                format!(
                                    "constant projection {place:?} lacks evaluated runtime length"
                                )
                            })?;
                        let length = state
                            .builder
                            .build_load(context.i64_type(), *length_slot, "holder_array_length")
                            .map_err(builder_error)?
                            .into_int_value();
                        let inside = state
                            .builder
                            .build_int_compare(
                                IntPredicate::ULT,
                                index_value,
                                length,
                                "holder_constant_in_bounds",
                            )
                            .map_err(builder_error)?;
                        let function = state
                            .builder
                            .get_insert_block()
                            .expect("constant holder block")
                            .get_parent()
                            .expect("constant holder function");
                        let valid =
                            context.append_basic_block(function, "holder.constant.in_bounds");
                        let invalid =
                            context.append_basic_block(function, "holder.constant.out_of_bounds");
                        state
                            .builder
                            .build_conditional_branch(inside, valid, invalid)
                            .map_err(builder_error)?;
                        state.builder.position_at_end(invalid);
                        let panic = state
                            .module
                            .get_function(CORE_SYSTEM_PANIC_SYMBOL)
                            .ok_or_else(|| {
                                "core panic declaration missing for constant holder index"
                                    .to_owned()
                            })?;
                        state
                            .builder
                            .build_call(panic, &[], "holder_constant_panic")
                            .map_err(builder_error)?;
                        state.builder.build_unreachable().map_err(builder_error)?;
                        state.builder.position_at_end(valid);
                        let direct = *state.runtime_array_allocations.get(&name)
                            .ok_or_else(|| format!("constant projection {place:?} lacks runtime allocation transport"))?;
                        let data = if address == root && direct {
                            address
                        } else {
                            state
                                .builder
                                .build_load(
                                    context.ptr_type(AddressSpace::default()),
                                    address,
                                    "holder_array_data",
                                )
                                .map_err(builder_error)?
                                .into_pointer_value()
                        };
                        let element_type =
                            storage_type(context, &element, state.structs, state.target_layout)?;
                        address = unsafe {
                            state.builder.build_in_bounds_gep(
                                element_type,
                                data,
                                &[index_value],
                                "holder_constant_element",
                            )
                        }
                        .map_err(builder_error)?;
                        ty = *element;
                    }
                    _ => return Err(format!("constant projection {place:?} indexes a non-array")),
                }
            }
            _ => {
                return Err(format!(
                    "constant projection {place:?} has an unaddressable component"
                ))
            }
        }
    }
    if !matches!(ty, ScalarType::Callable { .. }) {
        return Err(format!(
            "constant projection {place:?} does not select a callable"
        ));
    }
    Ok(address)
}

pub(super) fn concrete_callable_holder<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    edge: super::holder_events::HolderEmissionEdge,
    holder: &crate::scalar::CallableHolderPlace,
) -> Result<ConcreteCallableHolder<'ctx>, String> {
    state.holder_events.connected_holder_edge(edge, holder)?;
    let pointer_type = context.ptr_type(AddressSpace::default());
    let (callable, address, present) = match holder {
        crate::scalar::CallableHolderPlace::Temporary(point) => {
            if !state.holder_events.reads.contains_key(point) {
                return Err(format!(
                    "temporary {point:?} lacks a connected emitted ReadPlace"
                ));
            }
            callable_read(context, state, *point)?
        }
        crate::scalar::CallableHolderPlace::Binding(binding) => {
            let address = *state.holder_events.bindings.get(binding).ok_or_else(|| {
                format!("binding {binding:?} lacks source-qualified callable storage")
            })?;
            let callable_type =
                context.struct_type(&[pointer_type.into(), pointer_type.into()], false);
            let callable = state
                .builder
                .build_load(callable_type, address, "holder_binding")
                .map_err(builder_error)?
                .into_struct_value();
            (callable, address, context.bool_type().const_int(1, false))
        }
        crate::scalar::CallableHolderPlace::Projection(place) => {
            let indexed = place.projections.iter().any(|projection| {
                matches!(
                    projection,
                    crate::scalar::ReferencePlaceProjection::Index(_)
                )
            });
            let constant = indexed
                && place.projections.iter().all(|projection| {
                    !matches!(projection, crate::scalar::ReferencePlaceProjection::Index(expression)
                    if !matches!(expression, ScalarExpression::Integer { .. }))
                });
            let (address, present, selected) = if constant {
                (
                    constant_callable_projection_pointer(context, state, place)?,
                    context.bool_type().const_int(1, false),
                    false,
                )
            } else if indexed {
                let to = edge.to().ok_or_else(|| {
                    format!("indexed holder {holder:?} lacks a terminal selected-index fact")
                })?;
                let (selected, index_point) =
                    state
                        .holder_events
                        .selected_projection(edge.from(), to, place)?;
                let (_, address, present) =
                    selected_element(context, state, selected, index_point)?;
                (address, present, true)
            } else {
                let mut address = *state
                    .holder_events
                    .bindings
                    .get(&place.binding)
                    .ok_or_else(|| {
                        format!("projection {place:?} lacks source-qualified root storage")
                    })?;
                for projection in &place.projections {
                    let crate::scalar::ReferencePlaceProjection::Field(
                        ScalarFieldReference::Resolved(field),
                    ) = projection
                    else {
                        return Err(format!(
                            "projection {place:?} lacks an emitted field address"
                        ));
                    };
                    let declared = structure(state.structs, field.structure.clone())?
                        .fields
                        .get(field.index)
                        .filter(|candidate| candidate.id == *field)
                        .ok_or_else(|| format!("unknown callable projection field {field:?}"))?;
                    let offset = declared
                        .offset
                        .ok_or_else(|| format!("field {field:?} lacks layout"))?;
                    address = unsafe {
                        state.builder.build_in_bounds_gep(
                            context.i8_type(),
                            address,
                            &[context.i64_type().const_int(offset, false)],
                            "holder_field",
                        )
                    }
                    .map_err(builder_error)?;
                }
                (address, context.bool_type().const_int(1, false), false)
            };
            let callable_type =
                context.struct_type(&[pointer_type.into(), pointer_type.into()], false);
            let callable = if selected {
                let function = state
                    .builder
                    .get_insert_block()
                    .expect("selected holder block")
                    .get_parent()
                    .expect("selected holder function");
                let loaded = context.append_basic_block(function, "holder.selected.present");
                let absent = context.append_basic_block(function, "holder.selected.absent");
                let next = context.append_basic_block(function, "holder.selected.next");
                state
                    .builder
                    .build_conditional_branch(present, loaded, absent)
                    .map_err(builder_error)?;
                state.builder.position_at_end(loaded);
                let callable = state
                    .builder
                    .build_load(callable_type, address, "holder_projection")
                    .map_err(builder_error)?
                    .into_struct_value();
                state
                    .builder
                    .build_unconditional_branch(next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(absent);
                state
                    .builder
                    .build_unconditional_branch(next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(next);
                let merged = state
                    .builder
                    .build_phi(callable_type, "holder_selected_callable")
                    .map_err(builder_error)?;
                merged.add_incoming(&[(&callable, loaded), (&callable_type.const_zero(), absent)]);
                merged.as_basic_value().into_struct_value()
            } else {
                state
                    .builder
                    .build_load(callable_type, address, "holder_projection")
                    .map_err(builder_error)?
                    .into_struct_value()
            };
            (callable, address, present)
        }
        crate::scalar::CallableHolderPlace::Output(output)
        | crate::scalar::CallableHolderPlace::OutputProjection { output, .. } => {
            let to = edge
                .to()
                .ok_or_else(|| format!("output {output} lacks a terminal return transfer point"))?;
            let point = state
                .holder_events
                .return_holder_point(edge.from(), to, *output)?;
            let slots = state.holder_events.outputs.get(&point).ok_or_else(|| {
                format!("output {output} lacks a registered emitted callable slot at {point:?}")
            })?;
            let present = state
                .builder
                .build_load(context.bool_type(), slots.present, "output_holder_present")
                .map_err(builder_error)?
                .into_int_value();
            let root = state
                .builder
                .build_load(pointer_type, slots.address, "output_holder_address")
                .map_err(builder_error)?
                .into_pointer_value();
            let mut ty = slots.ty.clone();
            let projections = match holder {
                crate::scalar::CallableHolderPlace::Output(_) => &[][..],
                crate::scalar::CallableHolderPlace::OutputProjection { projections, .. } => {
                    projections.as_slice()
                }
                _ => unreachable!(),
            };
            let function = state
                .builder
                .get_insert_block()
                .expect("return holder block")
                .get_parent()
                .expect("return holder function");
            let loaded = context.append_basic_block(function, "holder.output.present");
            let absent = context.append_basic_block(function, "holder.output.absent");
            let next = context.append_basic_block(function, "holder.output.next");
            state
                .builder
                .build_conditional_branch(present, loaded, absent)
                .map_err(builder_error)?;
            state.builder.position_at_end(loaded);
            let mut address = root;
            if !matches!(ty, ScalarType::Callable { .. }) {
                for projection in projections {
                    let crate::scalar::ReferencePlaceProjection::Field(
                        ScalarFieldReference::Resolved(field),
                    ) = projection
                    else {
                        return Err(format!(
                            "output {output} has an unaddressable callable projection"
                        ));
                    };
                    let declared = structure(state.structs, field.structure.clone())?.fields.get(field.index)
                        .filter(|candidate| candidate.id == *field && matches!(ty, ScalarType::Struct(ref id) if *id == field.structure))
                        .ok_or_else(|| format!("output {output} has an unrelated field {field:?}"))?;
                    let offset = declared
                        .offset
                        .ok_or_else(|| format!("output {output} field lacks layout"))?;
                    ty = declared.ty.clone();
                    address = unsafe {
                        state.builder.build_in_bounds_gep(
                            context.i8_type(),
                            address,
                            &[context.i64_type().const_int(offset, false)],
                            "output_holder_field",
                        )
                    }
                    .map_err(builder_error)?;
                }
            }
            if !matches!(ty, ScalarType::Callable { .. }) {
                return Err(format!("output {output} does not project to a callable"));
            }
            let callable_type =
                context.struct_type(&[pointer_type.into(), pointer_type.into()], false);
            let callable = state
                .builder
                .build_load(callable_type, address, "output_holder_callable")
                .map_err(builder_error)?
                .into_struct_value();
            state
                .builder
                .build_unconditional_branch(next)
                .map_err(builder_error)?;
            state.builder.position_at_end(absent);
            state
                .builder
                .build_unconditional_branch(next)
                .map_err(builder_error)?;
            state.builder.position_at_end(next);
            let merged = state
                .builder
                .build_phi(callable_type, "output_holder_value")
                .map_err(builder_error)?;
            merged.add_incoming(&[(&callable, loaded), (&callable_type.const_zero(), absent)]);
            let address_phi = state
                .builder
                .build_phi(pointer_type, "output_holder_storage")
                .map_err(builder_error)?;
            address_phi.add_incoming(&[(&address, loaded), (&pointer_type.const_null(), absent)]);
            (
                merged.as_basic_value().into_struct_value(),
                address_phi.as_basic_value().into_pointer_value(),
                present,
            )
        }
    };
    let environment = state
        .builder
        .build_extract_value(callable, 1, "holder_environment")
        .map_err(builder_error)?
        .into_pointer_value();
    Ok(ConcreteCallableHolder {
        callable,
        environment,
        address,
        present,
    })
}

pub(super) enum ConcreteOwnerAction<'ctx> {
    Holder(ConcreteCallableHolder<'ctx>),
    CaptureCell(PointerValue<'ctx>),
}

pub(super) fn concrete_owner_action<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    edge: super::holder_events::HolderEmissionEdge,
    action: &crate::scalar::CallableHolderAction,
) -> Result<ConcreteOwnerAction<'ctx>, String> {
    state.holder_events.action(edge, action)?;
    let holder = match action {
        crate::scalar::CallableHolderAction::Retain { from, .. }
        | crate::scalar::CallableHolderAction::Move { from, .. }
        | crate::scalar::CallableHolderAction::ReturnTransfer { from, .. } => from,
        crate::scalar::CallableHolderAction::Drop { holder, .. } => holder,
        crate::scalar::CallableHolderAction::DropCaptureCellOwner { .. }
        | crate::scalar::CallableHolderAction::ReplaceCaptureCellValue { .. } => {
            return Ok(ConcreteOwnerAction::CaptureCell(
                state
                    .holder_events
                    .capture_cell_owner(edge, action, &state.capture_cells)?,
            ));
        }
        _ => return Err(format!("{action:?} has no concrete scalar holder")),
    };
    Ok(ConcreteOwnerAction::Holder(concrete_callable_holder(
        context, state, edge, holder,
    )?))
}

pub(super) fn structure<'a>(
    structs: &'a [ScalarStruct],
    id: crate::ScalarStructId,
) -> Result<&'a ScalarStruct, String> {
    structs
        .iter()
        .find(|structure| structure.id == id)
        .ok_or_else(|| format!("unknown LLVM struct {}", id.index))
}

pub(super) struct EvaluatedCallableSelection<'ctx> {
    pub point: crate::scalar::CfgPointId,
    pub index_point: crate::scalar::CfgPointId,
    pub index: inkwell::values::IntValue<'ctx>,
    pub cell: PointerValue<'ctx>,
    length_slot: Option<PointerValue<'ctx>>,
    fixed_length: Option<u64>,
    element: ScalarType,
    direct: bool,
}

pub(super) fn evaluate_callable_selection<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    binding: &crate::scalar::ReferenceBindingId,
    point: crate::scalar::CfgPointId,
    index_point: crate::scalar::CfgPointId,
    project: Option<ProjectCallScope<'_>>,
) -> Result<EvaluatedCallableSelection<'ctx>, String> {
    let crate::ScalarPlace::Index { base, index, .. } = place else {
        return Err("callable selection requires an indexed place".to_owned());
    };
    if binding.source != state.source {
        return Err(format!(
            "indexed callable place {point:?} has foreign binding {binding:?}"
        ));
    }
    let (cell, ty, name) = match base.as_ref() {
        crate::ScalarPlace::Name { name, .. } => {
            let (cell, ty) = if let Some((slot, ty)) = state.storage.get(name) {
                (*slot, ty.clone())
            } else {
                let (global, ty) = state
                    .globals
                    .get(name)
                    .ok_or_else(|| format!("indexed callable base {name} lacks emitted storage"))?;
                (global.as_pointer_value(), ty.clone())
            };
            (cell, ty, Some(name.as_str()))
        }
        crate::ScalarPlace::Field { .. } => {
            let mut root = base.as_ref();
            while let crate::ScalarPlace::Field { base, .. } = root {
                root = base;
            }
            if !matches!(root, crate::ScalarPlace::Name { .. }) {
                return Err(format!(
                    "unsupported source-qualified indexed callable field root {root:?}"
                ));
            }
            let ty = assignment_place_type(state, base)
                .ok_or_else(|| format!("indexed callable field {base:?} lacks typed layout"))?;
            (place_pointer(context, state, base, project)?, ty, None)
        }
        _ => {
            return Err(format!(
                "unsupported source-qualified indexed callable base {base:?}"
            ))
        }
    };
    let (element, fixed_length, length_slot, direct) = match ty {
        ScalarType::Array {
            element, length, ..
        } => (
            *element,
            Some(length),
            None,
            name.is_some_and(|_| state.capture_cells.contains_key(binding)),
        ),
        ScalarType::RuntimeArray { element, .. } => {
            let name = name.ok_or_else(|| {
                format!("indexed callable field {base:?} lacks shared runtime length provenance")
            })?;
            let length = *state
                .runtime_array_lengths
                .get(name)
                .ok_or_else(|| format!("indexed callable base {name} lacks shared length slot"))?;
            let direct = if state.capture_cells.contains_key(binding) {
                false
            } else {
                *state.runtime_array_allocations.get(name).ok_or_else(|| {
                    format!("indexed callable base {name} lacks validated allocation residence")
                })?
            };
            (*element, None, Some(length), direct)
        }
        _ => {
            return Err(format!(
                "indexed callable base {base:?} has no typed array layout"
            ))
        }
    };
    if !matches!(element, ScalarType::Callable { .. }) {
        return Err(format!(
            "indexed callable base {base:?} has a non-callable element"
        ));
    }
    let evaluated = take_basic(match project {
        Some(project) => emit_project_typed_expression(
            context,
            state,
            index,
            &ScalarType::U64,
            project.module,
            project.modules,
        )?,
        None => emit_typed_expression(context, state, index, &ScalarType::U64)?,
    })?
    .into_int_value();
    Ok(EvaluatedCallableSelection {
        point,
        index_point,
        index: evaluated,
        cell,
        length_slot,
        fixed_length,
        element,
        direct,
    })
}

pub(super) fn resolve_callable_selection<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    selection: &EvaluatedCallableSelection<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    let length = if let Some(slot) = selection.length_slot {
        state
            .builder
            .build_load(context.i64_type(), slot, "selected_current_length")
            .map_err(builder_error)?
            .into_int_value()
    } else {
        context.i64_type().const_int(
            selection.fixed_length.expect("fixed callable length"),
            false,
        )
    };
    let inside = state
        .builder
        .build_int_compare(
            IntPredicate::ULT,
            selection.index,
            length,
            "selected_in_bounds",
        )
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("selected block")
        .get_parent()
        .expect("selected function");
    let valid = context.append_basic_block(function, "selected.in_bounds");
    let invalid = context.append_basic_block(function, "selected.out_of_bounds");
    state
        .builder
        .build_conditional_branch(inside, valid, invalid)
        .map_err(builder_error)?;
    state.builder.position_at_end(invalid);
    declare_core_runtime(context, state.module);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .ok_or_else(|| "core panic declaration missing for selected index".to_owned())?;
    state
        .builder
        .build_call(panic, &[], "selected_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(valid);
    let element = storage_type(
        context,
        &selection.element,
        state.structs,
        state.target_layout,
    )?;
    if selection.length_slot.is_some() {
        let data = if selection.direct {
            selection.cell
        } else {
            state
                .builder
                .build_load(
                    context.ptr_type(AddressSpace::default()),
                    selection.cell,
                    "selected_current_backing",
                )
                .map_err(builder_error)?
                .into_pointer_value()
        };
        unsafe {
            state
                .builder
                .build_in_bounds_gep(element, data, &[selection.index], "array_element")
        }
        .map_err(builder_error)
    } else {
        let data = if selection.direct {
            state
                .builder
                .build_load(
                    context.ptr_type(AddressSpace::default()),
                    selection.cell,
                    "selected_current_fixed_backing",
                )
                .map_err(builder_error)?
                .into_pointer_value()
        } else {
            selection.cell
        };
        let array = element.array_type(
            u32::try_from(selection.fixed_length.expect("fixed callable length"))
                .map_err(|_| "fixed callable length exceeds LLVM array layout")?,
        );
        unsafe {
            state.builder.build_in_bounds_gep(
                array,
                data,
                &[context.i32_type().const_zero(), selection.index],
                "array_element",
            )
        }
        .map_err(builder_error)
    }
}

pub(super) fn selected_assignment_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), String> {
    let crate::ScalarPlace::Index { base, index, .. } = place else {
        return Err("selected assignment requires an indexed place".to_owned());
    };
    let base_pointer = place_pointer(context, state, base, project)?;
    let base_type = assignment_place_type(state, base)
        .ok_or_else(|| "unknown LLVM indexed place type".to_owned())?;
    if !matches!(
        base_type,
        ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
    ) {
        return Err("LLVM indexed place requires an array".to_owned());
    }
    let evaluated = take_basic(match project {
        Some(project) => emit_project_typed_expression(
            context,
            state,
            index,
            &ScalarType::U64,
            project.module,
            project.modules,
        )?,
        None => emit_typed_expression(context, state, index, &ScalarType::U64)?,
    })?
    .into_int_value();
    let address = if let ScalarType::RuntimeArray { element, .. } = base_type {
        let element = storage_type(context, &element, state.structs, state.target_layout)?;
        unsafe {
            state
                .builder
                .build_in_bounds_gep(element, base_pointer, &[evaluated], "array_element")
        }
        .map_err(builder_error)?
    } else {
        let array = storage_type(context, &base_type, state.structs, state.target_layout)?;
        let index = state
            .builder
            .build_int_truncate(evaluated, context.i32_type(), "array_index")
            .map_err(builder_error)?;
        unsafe {
            state.builder.build_in_bounds_gep(
                array,
                base_pointer,
                &[context.i32_type().const_zero(), index],
                "array_element",
            )
        }
        .map_err(builder_error)?
    };
    Ok((address, evaluated))
}

pub(super) fn place_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<PointerValue<'ctx>, String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => {
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                if matches!(ty, ScalarType::RuntimeArray { .. }) {
                    eprintln!(
                        "[D3DBG] place name={name} storage_hit allocations={:?}",
                        state.runtime_array_allocations.get(name).copied()
                    );
                    if state
                        .runtime_array_allocations
                        .get(name)
                        .copied()
                        .unwrap_or(false)
                    {
                        return Ok(slot);
                    }
                    return state
                        .builder
                        .build_load(basic_type(context, &ty, state.target_layout)?, slot, name)
                        .map_err(builder_error)
                        .map(BasicValueEnum::into_pointer_value);
                }
                return Ok(slot);
            }
            let (global, ty) = state
                .globals
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown LLVM place {name}"))?;
            if matches!(ty, ScalarType::RuntimeArray { .. }) {
                return state
                    .builder
                    .build_load(
                        basic_type(context, &ty, state.target_layout)?,
                        global.as_pointer_value(),
                        name,
                    )
                    .map_err(builder_error)
                    .map(BasicValueEnum::into_pointer_value);
            }
            Ok(global.as_pointer_value())
        }
        crate::ScalarPlace::Dereference { pointer, .. } => {
            let pointer_value = match project {
                Some(project) => emit_project_expression(
                    context,
                    state,
                    pointer,
                    project.module,
                    project.modules,
                )?,
                None => emit_expression(context, state, pointer)?,
            };
            let pointer_value = if let ScalarExpression::Assignment(assignment) = pointer.as_ref() {
                state
                    .holder_events
                    .emitted_assignment_result_shape(assignment)?;
                extract_output(state, pointer_value, 0)?
            } else {
                pointer_value
            };
            let pointer_value = take_basic(pointer_value)?.into_int_value();
            pointer_target_type(state, pointer, project)
                .ok_or_else(|| "unknown LLVM dereference target type".to_owned())?;
            if dereference_requires_null_guard(state, pointer, project) {
                emit_checked_dereference_guard(context, state, pointer_value)?;
            }
            state
                .builder
                .build_int_to_ptr(
                    pointer_value,
                    context.ptr_type(AddressSpace::default()),
                    "deref",
                )
                .map_err(builder_error)
        }
        crate::ScalarPlace::Field { base, field, .. } => {
            let base = place_pointer(context, state, base, project)?;
            let ScalarFieldReference::Resolved(field) = field else {
                return Err("unresolved LLVM struct field".to_owned());
            };
            let structure = structure(state.structs, field.structure.clone())?;
            let resolved = structure
                .fields
                .get(field.index)
                .filter(|candidate| candidate.id == *field)
                .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?;
            let byte_type = context.i8_type();
            let offset = byte_type.const_int(
                resolved.offset.ok_or_else(|| {
                    format!("LLVM struct field {} has no valid offset", field.index)
                })?,
                false,
            );
            unsafe {
                state.builder.build_in_bounds_gep(
                    byte_type,
                    base,
                    &[offset],
                    resolved.name.as_str(),
                )
            }
            .map_err(builder_error)
        }
        crate::ScalarPlace::Index { .. } => {
            selected_assignment_pointer(context, state, place, project).map(|(address, _)| address)
        }
    }
}

pub(super) fn emit_place_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
) -> Result<EmitValue<'ctx>, String> {
    emit_place_value_in_project(context, state, place, None)
}

pub(super) fn emit_place_value_in_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let pointer = place_pointer(context, state, place, project)?;
    let ty = match place {
        crate::ScalarPlace::Field { field, .. } => {
            let ScalarFieldReference::Resolved(field) = field else {
                return Err("unresolved LLVM struct field".to_owned());
            };
            structure(state.structs, field.structure.clone())?
                .fields
                .get(field.index)
                .filter(|candidate| candidate.id == *field)
                .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?
                .ty
                .clone()
        }
        crate::ScalarPlace::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone()))
            .ok_or_else(|| format!("unknown LLVM place {name}"))?,
        crate::ScalarPlace::Dereference { pointer, .. } => {
            pointer_target_type(state, pointer, project)
                .ok_or_else(|| "unknown LLVM dereference target type".to_owned())?
        }
        crate::ScalarPlace::Index { .. } => assignment_place_type(state, place)
            .ok_or_else(|| "unknown LLVM indexed place type".to_owned())?,
    };
    if matches!(
        ty,
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
    ) {
        return Ok(EmitValue::Basic(pointer.into()));
    }
    Ok(EmitValue::Basic(
        state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                pointer,
                "place",
            )
            .map_err(builder_error)?,
    ))
}

pub(super) fn struct_member_place<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    receiver: &str,
    name: &str,
) -> Result<Option<crate::ScalarPlace>, String> {
    let ty = state
        .storage
        .get(receiver)
        .map(|(_, ty)| ty)
        .or_else(|| state.globals.get(receiver).map(|(_, ty)| ty));
    let Some(ScalarType::Struct(id)) = ty else {
        return Ok(None);
    };
    let field = structure(state.structs, id.clone())?
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format!("unknown LLVM struct field {name}"))?;
    Ok(Some(crate::ScalarPlace::Field {
        base: Box::new(crate::ScalarPlace::Name {
            name: receiver.to_owned(),
            span: wosy_syntax::ByteSpan::new(0, 0),
        }),
        field: ScalarFieldReference::Resolved(field.id.clone()),
        span: wosy_syntax::ByteSpan::new(0, 0),
    }))
}

pub(super) fn emit_struct_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expected: &crate::ScalarType,
    fields: &[crate::ScalarStructLiteralField],
) -> Result<EmitValue<'ctx>, String> {
    let crate::ScalarType::Struct(id) = expected else {
        return Err("struct literal requires a resolved struct type".to_owned());
    };
    let structure = structure(state.structs, id.clone())?;
    let layout = structure
        .layout
        .as_ref()
        .ok_or_else(|| format!("LLVM struct {} has no valid layout", id.index))?;
    let storage = context.i8_type().array_type(
        u32::try_from(layout.size).map_err(|_| "struct layout exceeds LLVM array size")?,
    );
    let destination = entry_alloca(state, storage.into(), "struct_literal")?;
    let mut values = BTreeMap::new();
    for field in fields {
        values.insert(
            field.name.clone(),
            emit_typed_expression(
                context,
                state,
                &field.value,
                &structure
                    .fields
                    .iter()
                    .find(|candidate| candidate.name == field.name)
                    .expect("validated struct field")
                    .ty,
            )?,
        );
    }
    for field in &structure.fields {
        let value = values
            .remove(&field.name)
            .ok_or_else(|| format!("missing LLVM struct literal field {}", field.name))?;
        let pointer = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                destination,
                &[context.i8_type().const_int(
                    field.offset.ok_or_else(|| {
                        format!("LLVM struct field {} has no valid offset", field.id.index)
                    })?,
                    false,
                )],
                field.name.as_str(),
            )
        }
        .map_err(builder_error)?;
        store_value(context, state, pointer, &field.ty, value)?;
    }
    Ok(EmitValue::Basic(destination.into()))
}

pub(super) fn emit_array_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expected: &ScalarType,
    elements: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let ScalarType::Array {
        element, length, ..
    } = expected
    else {
        return Err("array literal requires a resolved fixed array type".to_owned());
    };
    if elements.len() as u64 != *length {
        return Err("array literal element count does not match fixed array length".to_owned());
    }
    let storage = storage_type(context, expected, state.structs, state.target_layout)?;
    let destination = entry_alloca(state, storage, "array_literal")?;
    let element_storage = storage_type(context, element, state.structs, state.target_layout)?;
    for (index, expression) in elements.iter().enumerate() {
        let pointer = unsafe {
            state.builder.build_in_bounds_gep(
                storage,
                destination,
                &[
                    context.i32_type().const_zero(),
                    context.i32_type().const_int(index as u64, false),
                ],
                "array_element",
            )
        }
        .map_err(builder_error)?;
        let value = emit_typed_expression(context, state, expression, element)?;
        let _ = element_storage;
        store_value(context, state, pointer, element, value)?;
    }
    Ok(EmitValue::Basic(destination.into()))
}

pub(super) fn emit_typed_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    expected: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    match (expression, expected) {
        (ScalarExpression::Utf8 { value, .. }, ScalarType::Struct(_)) => {
            emit_utf8_literal(context, state, value, expected)
        }
        (ScalarExpression::StructLiteral { fields, .. }, ScalarType::Struct(_)) => {
            emit_struct_literal(context, state, expected, fields)
        }
        (ScalarExpression::ArrayLiteral { elements, .. }, ScalarType::Array { .. }) => {
            emit_array_literal(context, state, expected, elements)
        }
        (
            ScalarExpression::Unary {
                operator, operand, ..
            },
            ty,
        ) if integer_width(ty).is_some() || *ty == ScalarType::Bool => {
            let value =
                take_basic(emit_typed_expression(context, state, operand, ty)?)?.into_int_value();
            emit_unary_value(state, operator, value)
        }
        (ScalarExpression::Integer { value, .. }, ty) if integer_width(ty).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, ty, value)?.into()),
        ),
        (ScalarExpression::Float { value, .. }, ty) => Ok(EmitValue::Basic(
            float_constant(context, ty, *value)?.into(),
        )),
        _ => emit_expression(context, state, expression),
    }
}

pub(super) fn emit_utf8_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: &[u8],
    expected: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let ScalarType::Struct(id) = expected else {
        return Err("string literal requires a resolved std.utf8 struct".to_owned());
    };
    let structure = structure(state.structs, id.clone())?;
    if structure.fields.len() != 2
        || structure.fields[0].name != "data"
        || structure.fields[1].name != "length"
    {
        return Err("std.utf8 has an invalid field layout".to_owned());
    }
    let byte_type = context.i8_type();
    let literal_type =
        byte_type.array_type(u32::try_from(value.len()).map_err(|_| "utf8 literal is too large")?);
    let literal = state.module.add_global(
        literal_type,
        None,
        &format!("wosy_utf8_literal_{}", state.next_literal),
    );
    state.next_literal += 1;
    literal.set_linkage(Linkage::Private);
    literal.set_constant(true);
    let bytes = value
        .iter()
        .map(|byte| byte_type.const_int(u64::from(*byte), false))
        .collect::<Vec<_>>();
    literal.set_initializer(&byte_type.const_array(&bytes));
    let destination = entry_alloca(
        state,
        storage_type(context, expected, state.structs, state.target_layout)?,
        "utf8_literal",
    )?;
    let data = unsafe {
        state.builder.build_in_bounds_gep(
            byte_type,
            destination,
            &[byte_type.const_int(
                structure.fields[0]
                    .offset
                    .ok_or_else(|| "std.utf8 data field has no valid offset".to_owned())?,
                false,
            )],
            "data",
        )
    }
    .map_err(builder_error)?;
    let address = state
        .builder
        .build_ptr_to_int(
            literal.as_pointer_value(),
            pointer_integer_type(context, state.target_layout),
            "utf8_address",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(data, address)
        .map_err(builder_error)?;
    let length = unsafe {
        state.builder.build_in_bounds_gep(
            byte_type,
            destination,
            &[byte_type.const_int(
                structure.fields[1]
                    .offset
                    .ok_or_else(|| "std.utf8 length field has no valid offset".to_owned())?,
                false,
            )],
            "length",
        )
    }
    .map_err(builder_error)?;
    state
        .builder
        .build_store(
            length,
            context.i64_type().const_int(value.len() as u64, false),
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(destination.into()))
}

pub(super) fn store_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    destination: PointerValue<'ctx>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    if has_callable_holders(ty, state.structs)? {
        super::ownership::begin_graph_mutation(&state.builder, state.module)?;
        store_value_locked(context, state, destination, ty, value)?;
        super::ownership::end_graph_mutation(&state.builder, state.module)
    } else {
        store_value_locked(context, state, destination, ty, value)
    }
}

pub(super) fn store_value_locked<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    destination: PointerValue<'ctx>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        let source = take_basic(value)?.into_pointer_value();
        let aggregate = storage_type(context, ty, state.structs, state.target_layout)?;
        let value = state
            .builder
            .build_load(aggregate, source, "struct_value")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(destination, value)
            .map_err(builder_error)
            .map(|_| ())
    } else {
        state
            .builder
            .build_store(destination, take_basic(value)?)
            .map_err(builder_error)
            .map(|_| ())
    }
}

pub(super) fn take_basic(value: EmitValue<'_>) -> Result<BasicValueEnum<'_>, String> {
    match value {
        EmitValue::Basic(value) => Ok(value),
        EmitValue::Unit => Err("unit value used where a scalar value is required".into()),
        EmitValue::Aggregate { .. } => {
            Err("aggregate value used where a scalar value is required".into())
        }
    }
}

pub(super) fn build_aggregate<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outputs: &crate::ScalarOutputSequence,
    values: Vec<EmitValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let ty = aggregate_type(context, outputs, state.target_layout)?;
    let mut aggregate = ty.const_zero();
    for (index, (output, value)) in outputs.outputs.iter().zip(values).enumerate() {
        let value = if output.ty == ScalarType::Unit {
            context.i8_type().const_zero().into()
        } else {
            take_basic(value)?
        };
        aggregate = state
            .builder
            .build_insert_value(aggregate, value, index as u32, "output")
            .map_err(builder_error)?
            .into_struct_value();
    }
    Ok(EmitValue::Aggregate {
        value: aggregate,
        outputs: outputs
            .outputs
            .iter()
            .map(|output| output.ty.clone())
            .collect(),
    })
}

pub(super) fn extract_output<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    value: EmitValue<'ctx>,
    position: usize,
) -> Result<EmitValue<'ctx>, String> {
    let EmitValue::Aggregate { value, outputs } = value else {
        return Ok(value);
    };
    let ty = outputs
        .get(position)
        .ok_or_else(|| "LLVM output position is out of range".to_owned())?;
    if *ty == ScalarType::Unit {
        return Ok(EmitValue::Unit);
    }
    Ok(EmitValue::Basic(
        state
            .builder
            .build_extract_value(value, position as u32, "output")
            .map_err(builder_error)?
            .into(),
    ))
}
