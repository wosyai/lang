use inkwell::attributes::AttributeLoc;
use inkwell::values::BasicValue;

use super::expression::{
    emit_contextual_numeric, emit_expression, emit_project_expression, emit_project_typed_expression,
    emit_unary_value, ExpressionPath,
};
use super::*;
use crate::ScalarPlace;

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
    layouts: &crate::scalar::layout::ScalarLayoutIndex<'_>,
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    storage_type_and_alignment(context, ty, layouts, target_layout).map(|(ty, _)| ty)
}

pub(super) fn storage_type_and_alignment<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    layouts: &crate::scalar::layout::ScalarLayoutIndex<'_>,
    target_layout: ScalarTargetLayout,
) -> Result<(BasicTypeEnum<'ctx>, Option<u64>), String> {
    match ty {
        ScalarType::Struct(id) => {
            let structure = layouts
                .aggregate(id)
                .ok_or_else(|| format!("unknown LLVM struct {}", id.index))?;
            let layout = structure
                .layout
                .as_ref()
                .ok_or_else(|| format!("LLVM struct {} has no valid layout", id.index))?;
            let size =
                u32::try_from(layout.size).map_err(|_| "struct layout exceeds LLVM array size")?;
            Ok((context.i8_type().array_type(size).into(), Some(layout.alignment)))
        }
        ScalarType::Array {
            element, length, ..
        } => {
            let (element, alignment) =
                storage_type_and_alignment(context, element, layouts, target_layout)?;
            let length =
                u32::try_from(*length).map_err(|_| "fixed array length exceeds LLVM array size")?;
            Ok((element.array_type(length).into(), alignment))
        }
        _ => Ok((basic_type(context, ty, target_layout)?, None)),
    }
}

pub(super) fn storage_alignment(alignment: Option<u64>) -> Result<Option<u32>, String> {
    match alignment {
        None | Some(1) => Ok(None),
        Some(alignment) => {
            let alignment = u32::try_from(alignment)
                .ok()
                .filter(|alignment| alignment.is_power_of_two())
                .ok_or_else(|| "LLVM aggregate storage alignment is not representable".to_owned())?;
            Ok(Some(alignment))
        }
    }
}

pub(super) fn allocation_layout(
    ty: &ScalarType,
    layouts: &crate::scalar::layout::ScalarLayoutIndex<'_>,
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
        ScalarType::Enum(_) => Ok((4, 4)),
        ScalarType::Struct(id) => layouts
            .aggregate(id)
            .and_then(|structure| structure.layout.as_ref())
            .map(|layout| (layout.size, layout.alignment))
            .ok_or_else(|| format!("runtime allocation has no layout for struct {}", id.index)),
        ScalarType::Array {
            element, length, ..
        } => {
            let (size, alignment) = allocation_layout(element, layouts, target_layout)?;
            let size = size
                .checked_mul(*length)
                .ok_or_else(|| "runtime allocation size overflows u64".to_owned())?;
            Ok((size, alignment))
        }
        ScalarType::Named { .. }
        | ScalarType::Qualified { .. }
        | ScalarType::RuntimeArray { .. }
        | ScalarType::Unit
        | ScalarType::Error
        | ScalarType::Callable { .. } => {
            Err("runtime allocation requires a sized element type".to_owned())
        }
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

pub(super) fn align_storage_alloca(
    slot: PointerValue<'_>,
    alignment: Option<u64>,
) -> Result<(), String> {
    if let Some(alignment) = storage_alignment(alignment)? {
        slot.as_instruction_value()
            .ok_or_else(|| "LLVM aggregate storage is not an alloca".to_owned())?
            .set_alignment(alignment)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn entry_storage_alloca<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    ty: &ScalarType,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    let (ty, alignment) = storage_type_and_alignment(context, ty, state.layouts, state.target_layout)?;
    let slot = entry_alloca(state, ty, name)?;
    align_storage_alloca(slot, alignment)?;
    Ok(slot)
}

pub(super) fn structure<'a>(
    structs: &[&'a ScalarStruct],
    id: crate::ScalarStructId,
) -> Result<&'a ScalarStruct, String> {
    structs
        .iter()
        .copied()
        .find(|structure| structure.id == id)
        .ok_or_else(|| format!("unknown LLVM struct {}", id.index))
}

pub(super) fn place_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<PointerValue<'ctx>, String> {
    match place {
        crate::ScalarPlace::Contextual { binding, .. } => state.contextual_storage.get(binding).map(|(pointer, _)| *pointer).ok_or_else(|| "missing prepared LLVM contextual storage".to_owned()),
        crate::ScalarPlace::Name { name, .. } => {
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                if matches!(ty, ScalarType::RuntimeArray { .. }) {
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
            let pointer_value = take_basic(match project {
                Some(project) => emit_project_expression(
                    context,
                    state,
                    pointer,
                    project.module,
                    project.modules,
                )?,
                None => emit_expression(context, state, pointer)?,
            })?
            .into_int_value();
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
            resolved_field_pointer(context, state, base, field)
        }
        crate::ScalarPlace::Index { base, index, .. } => {
            let base_pointer = place_pointer(context, state, base, project)?;
            let base_type = assignment_place_type(state, base)
                .ok_or_else(|| "unknown LLVM indexed place type".to_owned())?;
            if !matches!(
                base_type,
                ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
            ) {
                return Err("LLVM indexed place requires an array".to_owned());
            }
            let index = emit_place_index(context, state, index, project)?;
            if let ScalarType::RuntimeArray { element, .. } = base_type {
                let element = storage_type(context, &element, state.layouts, state.target_layout)?;
                return unsafe {
                    state.builder.build_in_bounds_gep(
                        element,
                        base_pointer,
                        &[index],
                        "array_element",
                    )
                }
                .map_err(builder_error);
            }
            fixed_array_element_pointer(context, state, &base_type, base_pointer, index)
        }
    }
}

fn emit_place_index<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    index: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    Ok(take_basic(match project {
        Some(project) => emit_project_typed_expression(
            context, state, index, &ScalarType::U64, project.module, project.modules,
        )?,
        None => emit_typed_expression(context, state, index, &ScalarType::U64)?,
    })?.into_int_value())
}

fn fixed_array_element_pointer<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    ty: &ScalarType,
    base: PointerValue<'ctx>,
    index: inkwell::values::IntValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    let array = storage_type(context, ty, state.layouts, state.target_layout)?;
    let index = if state.target_layout == ScalarTargetLayout::WASM32
        && index.get_type().get_bit_width() > 32
    {
        state
            .builder
            .build_int_truncate(
                index,
                pointer_integer_type(context, state.target_layout),
                "array_address_index",
            )
            .map_err(builder_error)?
    } else {
        index
    };
    unsafe {
        state.builder.build_in_bounds_gep(
            array, base, &[context.i32_type().const_zero(), index], "array_element",
        )
    }
    .map_err(builder_error)
}

pub(super) fn checked_fixed_assignment_element_pointer<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    base: &ScalarPlace,
    index: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Result<PointerValue<'ctx>, String> {
    let ScalarPlace::Name { .. } = base else {
        return Err("checked fixed assignment requires a directly named array".to_owned());
    };
    let Some(ty @ ScalarType::Array { length, .. }) = assignment_place_type(state, base) else {
        return Err("checked fixed assignment requires a fixed array extent".to_owned());
    };
    let pointer = place_pointer(context, state, base, project)?;
    checked_fixed_element_pointer(
        context, state, &ty, pointer, length, index, project,
        ("assignment_index_out_of_bounds", "assignment_index_panic", "assignment_index_continue"),
    )
}

pub(super) fn checked_fixed_element_pointer<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    ty: &ScalarType,
    pointer: PointerValue<'ctx>,
    length: u64,
    index: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
    labels: (&str, &str, &str),
) -> Result<PointerValue<'ctx>, String> {
    let index = emit_place_index(context, state, index, project)?;
    if !index.get_zero_extended_constant().is_some_and(|index| index < length) {
        let invalid = state.builder.build_int_compare(
            IntPredicate::UGE, index, context.i64_type().const_int(length, false), labels.0,
        ).map_err(builder_error)?;
        emit_checked_place_guard(context, state, invalid, labels.1, labels.2)?;
    }
    fixed_array_element_pointer(context, state, ty, pointer, index)
}


fn byte_layout_field_pointer<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    base: PointerValue<'ctx>,
    offset: u64,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    let byte = context.i8_type();
    let index = if offset <= i8::MAX as u64 {
        byte.const_int(offset, false)
    } else {
        let index = pointer_integer_type(context, state.target_layout);
        let width = index.get_bit_width();
        if width <= u64::BITS && offset >> (width - 1) != 0 {
            return Err("LLVM struct field byte offset exceeds target index range".to_owned());
        }
        index.const_int(offset, false)
    };
    unsafe { state.builder.build_in_bounds_gep(byte, base, &[index], name) }
        .map_err(builder_error)
}

pub(super) fn resolved_field_pointer<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    base: PointerValue<'ctx>,
    field: &crate::ScalarStructFieldId,
) -> Result<PointerValue<'ctx>, String> {
    let structure = structure(state.structs, field.structure.clone())?;
    let resolved = structure
        .fields
        .get(field.index)
        .filter(|candidate| candidate.id == *field)
        .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?;
    let offset = state
        .layouts
        .field(field)
        .and_then(|result| result.offset)
        .ok_or_else(|| format!("LLVM struct field {} has no valid offset", field.index))?;
    byte_layout_field_pointer(context, state, base, offset, resolved.name.as_str())
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
        crate::ScalarPlace::Contextual { ty, .. } => ty.clone(),
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
    load_place_value(context, state, pointer, &ty)
}

pub(super) fn load_place_value<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    pointer: PointerValue<'ctx>,
    ty: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
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
                basic_type(context, ty, state.target_layout)?,
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
    let layout = state
        .layouts
        .aggregate(id)
        .ok_or_else(|| format!("unknown LLVM struct {}", id.index))?
        .layout
        .as_ref()
        .ok_or_else(|| format!("LLVM struct {} has no valid layout", id.index))?;
    let storage = context.i8_type().array_type(
        u32::try_from(layout.size).map_err(|_| "struct layout exceeds LLVM array size")?,
    );
    let destination = entry_alloca(state, storage.into(), "struct_literal")?;
    align_storage_alloca(destination, Some(layout.alignment))?;
    let mut values = BTreeMap::new();
    for field in &structure.fields {
        values.entry(field.name.as_str()).or_insert((&field.ty, None));
    }
    for field in fields {
        let (ty, value) = values
            .get_mut(field.name.as_str())
            .expect("validated struct field");
        *value = Some(emit_typed_expression(context, state, &field.value, ty)?);
    }
    for field in &structure.fields {
        let value = values
            .get_mut(field.name.as_str())
            .and_then(|(_, value)| value.take())
            .ok_or_else(|| format!("missing LLVM struct literal field {}", field.name))?;
        let offset = state
            .layouts
            .field(&field.id)
            .and_then(|result| result.offset)
            .ok_or_else(|| format!("LLVM struct field {} has no valid offset", field.id.index))?;
        let pointer =
            byte_layout_field_pointer(context, state, destination, offset, field.name.as_str())?;
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
    let (storage, alignment) =
        storage_type_and_alignment(context, expected, state.layouts, state.target_layout)?;
    let destination = entry_alloca(state, storage, "array_literal")?;
    align_storage_alloca(destination, alignment)?;
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
            let value = take_basic(emit_typed_expression(context, state, operand, ty)?)?;
            emit_unary_value(state, operator, value)
        }
        (ScalarExpression::Integer { value, .. }, ty) if integer_width(ty).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, ty, value)?.into()),
        ),
        (ScalarExpression::Binary { .. } | ScalarExpression::If { .. } | ScalarExpression::Block(_), ty)
            if integer_width(ty).is_some() =>
        {
            emit_contextual_numeric(context, state, expression, ty, ExpressionPath::Single)
        }
        (_, ScalarType::F32 | ScalarType::F64) => emit_contextual_numeric(
            context,
            state,
            expression,
            expected,
            ExpressionPath::Single,
        ),
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
    let destination = entry_storage_alloca(context, state, expected, "utf8_literal")?;
    let data_offset = state
        .layouts
        .field(&structure.fields[0].id)
        .and_then(|result| result.offset)
        .ok_or_else(|| "std.utf8 data field has no valid offset".to_owned())?;
    let data = byte_layout_field_pointer(context, state, destination, data_offset, "data")?;
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
    let length_offset = state
        .layouts
        .field(&structure.fields[1].id)
        .and_then(|result| result.offset)
        .ok_or_else(|| "std.utf8 length field has no valid offset".to_owned())?;
    let length = byte_layout_field_pointer(context, state, destination, length_offset, "length")?;
    state
        .builder
        .build_store(
            length,
            context.i64_type().const_int(value.len() as u64, false),
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(destination.into()))
}

pub(super) fn copy_fixed_aggregate<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    destination: PointerValue<'ctx>,
    source: PointerValue<'ctx>,
    ty: &ScalarType,
) -> Result<(), String> {
    let (size, _) = allocation_layout(ty, state.layouts, state.target_layout)?;
    if size == 0 {
        return Ok(());
    }
    if state.target_layout == ScalarTargetLayout::WASM32 {
        u32::try_from(size).map_err(|_| "fixed aggregate copy exceeds Wasm32 address space")?;
        let function = state
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| "missing function for fixed aggregate copy".to_owned())?;
        if function
            .get_string_attribute(AttributeLoc::Function, "target-features")
            .is_none()
        {
            function.add_attribute(
                AttributeLoc::Function,
                context.create_string_attribute("target-features", "+bulk-memory"),
            );
        }
    }
    let size = pointer_integer_type(context, state.target_layout).const_int(size, false);
    state
        .builder
        .build_memmove(destination, 1, source, 1, size)
        .map_err(builder_error)
        .map(|_| ())
}

pub(super) fn store_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    destination: PointerValue<'ctx>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        let source = take_basic(value)?.into_pointer_value();
        copy_fixed_aggregate(context, state, destination, source, ty)
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
    build_aggregate_at(context, state.builder, state.target_layout, outputs, values)
}

pub(super) fn build_aggregate_at<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    target: ScalarTargetLayout,
    outputs: &crate::ScalarOutputSequence,
    values: Vec<EmitValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let ty = aggregate_type(context, outputs, target)?;
    let mut aggregate = ty.const_zero();
    for (index, (output, value)) in outputs.outputs.iter().zip(values).enumerate() {
        let value = if output.ty == ScalarType::Unit {
            context.i8_type().const_zero().into()
        } else {
            take_basic(value)?
        };
        aggregate = builder
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
    value: &EmitValue<'ctx>,
    position: usize,
) -> Result<EmitValue<'ctx>, String> {
    let (value, outputs) = match value {
        EmitValue::Unit => return Ok(EmitValue::Unit),
        EmitValue::Basic(value) => return Ok(EmitValue::Basic(*value)),
        EmitValue::Aggregate { value, outputs } => (*value, outputs),
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
