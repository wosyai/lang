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
            let (_, pointer_value) = dereference_address(context, state, pointer, project)?;
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
            let base_type = assignment_place_type(state, base)
                .ok_or_else(|| "unknown LLVM indexed place type".to_owned())?;
            if !matches!(
                base_type,
                ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
            ) {
                return Err("LLVM indexed place requires an array".to_owned());
            }
            // A guarded lexical array stores its original backing directly,
            // not an ordinary runtime-array pointer slot. Authenticate and
            // retain that backing and extent before evaluating the selector.
            let guarded = if let crate::ScalarPlace::Name { name, .. } = base.as_ref() {
                if let Some((backing, _)) = state.storage.get(name) {
                    ownership::guarded_array_extent(context, state, base, *backing, &base_type)?
                        .map(|extent| (*backing, extent))
                } else {
                    None
                }
            } else {
                None
            };
            let (base_pointer, guarded_extent) = if let Some((backing, extent)) = guarded {
                (backing, Some(extent))
            } else {
                (place_pointer(context, state, base, project)?, None)
            };
            let fallback = guarded_extent.map(|_| (
                "guarded_index_out_of_bounds", "guarded_index_panic", "guarded_index_continue",
            ));
            let runtime_extent = if guarded_extent.is_none()
                && (state.require_release_checked_access || !state.local_runtime_extents.is_empty())
                && matches!(&base_type, ScalarType::RuntimeArray { .. })
            {
                // Snapshot the extent paired with this base before a selector
                // expression may evaluate arbitrary calls or local assignments.
                capture_local_runtime_extent(context, state, base, base_pointer, true)?
            } else {
                None
            };
            let checked_lexical_descriptor = runtime_extent.is_some()
                && state.local_runtime_extents.get(&base_pointer)
                    .is_some_and(|(_, extent, _)| matches!(extent,
                        LocalRuntimeExtent::OrdinaryDescriptor(_) | LocalRuntimeExtent::WidenedLiteral(_)));
            let index = emit_place_index(context, state, index, project)?;
            if let ScalarType::RuntimeArray { element, .. } = base_type {
                if let Some(extent) = guarded_extent {
                    check_array_extent(context, state, index, extent, fallback)?;
                    if let Some(length) = extent.get_zero_extended_constant() {
                        let crate::ScalarPlace::Name { span, .. } = base.as_ref() else {
                            return Err("guarded runtime array index lacks its exact lexical base".into());
                        };
                        let layout = ScalarType::Array { element, length, length_span: *span, span: *span };
                        return fixed_array_element_pointer(context, state, &layout, base_pointer, index, true);
                    }
                }
                if let Some(extent) = runtime_extent {
                    if checked_lexical_descriptor {
                        check_array_extent(context, state, index, extent, Some((
                            "ordinary_array_index_out_of_bounds", "ordinary_array_index_panic", "ordinary_array_index_continue",
                        )))?;
                    } else {
                        check_release_index(state, index, extent)?;
                    }
                } else if super::aggregate_transfer::requires_returned_array_index(state) {
                    return Err("sealed returned array index lost its paired extent".into());
                }
                let element = storage_type(context, &element, state.layouts, state.target_layout)?;
                let index = if guarded_extent.is_some() || checked_lexical_descriptor {
                    array_address_index(context, state, index)?
                } else { index };
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
            let ScalarType::Array { length, .. } = base_type else { unreachable!("fixed array") };
            let checked = guarded_extent.is_some() || !fixed_index_has_raw_root(state, base);
            if checked {
                check_fixed_index(context, state, index, length, fallback)?;
            }
            fixed_array_element_pointer(context, state, &base_type, base_pointer, index, checked)
        }
    }
}

fn dereference_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    pointer: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Result<(ScalarType, inkwell::values::IntValue<'ctx>), String> {
    let pointer_value = take_basic(match project {
        Some(project) => emit_project_expression(context, state, pointer, project.module, project.modules)?,
        None => emit_expression(context, state, pointer)?,
    })?.into_int_value();
    let pointee = pointer_target_type(state, pointer, project)
        .ok_or_else(|| "unknown LLVM dereference target type".to_owned())?;
    if dereference_requires_null_guard(state, pointer, project)
        && !ownership::guarded_alias_has_non_null_proof(state, pointer)
    {
        emit_checked_dereference_guard(context, state, pointer_value)?;
    }
    Ok((pointee, pointer_value))
}

pub(super) fn emit_array_len<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    super::aggregate_transfer::check_replacement_extent_observation(state, expression)?;
    let [ScalarExpression::CheckedAddress { place, .. }] = arguments else {
        return Err("core.array_len lacks a prepared named array address".into());
    };
    if let crate::ScalarPlace::Dereference { pointer, .. } = place {
        // Observe the checked place normally: the existing null policy owns
        // validation before even a constant extent may be observed.
        let (referent, _) = dereference_address(context, state, pointer, project)?;
        let length = match referent {
            ScalarType::Array { length, .. } => context.i64_type().const_int(length, false),
            ScalarType::RuntimeArray { .. } if super::aggregate_transfer::borrowed_extent_at(state, expression, pointer) =>
                state.aggregate.as_ref().and_then(|aggregate| aggregate.borrowed_extent)
                    .ok_or("missing sealed borrowed array count")?,
            _ => return Err("core.array_len lacks a checked array referent extent".into()),
        };
        return Ok(EmitValue::Basic(length.into()));
    }
    let crate::ScalarPlace::Name { name, .. } = place else {
        return Err("core.array_len lacks a prepared named array address".into());
    };
    let Some((storage, ty)) = state.storage.get(name) else {
        return Err("core.array_len has no prepared local array".into());
    };
    let length = match ty {
        ScalarType::Array { length, .. } => context.i64_type().const_int(*length, false),
        ScalarType::RuntimeArray { .. } => capture_local_runtime_extent(context, state, place, *storage, false)?
            .ok_or_else(|| "core.array_len requires the sealed extent of a local runtime allocation".to_owned())?,
        _ => return Err("core.array_len requires an array".into()),
    };
    Ok(EmitValue::Basic(length.into()))
}

pub(super) fn capture_local_runtime_extent<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    base: &crate::ScalarPlace,
    address: PointerValue<'ctx>,
    for_index: bool,
) -> Result<Option<inkwell::values::IntValue<'ctx>>, String> {
    let crate::ScalarPlace::Name { name, .. } = base else { return Ok(None) };
    let Some((storage, ty @ ScalarType::RuntimeArray { .. })) = state.storage.get(name) else {
        return Ok(None);
    };
    if let Some(extent) = ownership::guarded_array_extent(context, state, base, address, ty)? {
        return Ok(Some(extent));
    }
    let Some((declaration, extent, checked_extent)) = state.local_runtime_extents.get(storage) else {
        // Incoming, forwarded and other unresolved extents use their existing path.
        return Ok(None);
    };
    if for_index && !checked_extent {
        return Ok(None);
    }
    if state.body_call_nodes.and_then(|nodes| nodes.lexical_place_declaration(base)) != Some(*declaration) {
        return Err("runtime-array index differs from its qualified local declaration".to_owned());
    }
    match extent {
        LocalRuntimeExtent::Construction(length) | LocalRuntimeExtent::WidenedLiteral(length)
        | LocalRuntimeExtent::OrdinaryDescriptor(length) if *storage == address && state.runtime_array_allocations.get(name) == Some(&true) =>
                Ok(Some(*length)),
        LocalRuntimeExtent::OwningFormal(length)
            if !for_index && *storage == address && state.runtime_array_allocations.get(name) == Some(&false) =>
                Ok(Some(*length)),
        LocalRuntimeExtent::ManagedLengthSlot(length_slot)
            if state.runtime_array_allocations.get(name) == Some(&false)
                && state.runtime_array_lengths.get(name) == Some(length_slot) =>
            state.builder.build_load(context.i64_type(), *length_slot, "runtime_extent")
                .map(|length| Some(length.into_int_value())).map_err(builder_error),
        _ => Err("indexed runtime allocation lost its sealed extent identity".to_owned()),
    }
}

fn check_release_index<'ctx>(
    state: &EmitState<'ctx, '_>,
    index: inkwell::values::IntValue<'ctx>,
    count: inkwell::values::IntValue<'ctx>,
) -> Result<(), String> {
    if index.get_zero_extended_constant().zip(count.get_zero_extended_constant())
        .is_some_and(|(index, count)| index < count)
    {
        return Ok(());
    }
    let access = state.project_names.and_then(|names| names.checked_access.as_ref())
        .ok_or_else(|| "index lacks its prepared release bounds helper".to_owned())?;
    emit_checked_access_call(state, &access.in_extent, &[index.into(), count.into()])
}

fn fixed_index_has_raw_root(state: &EmitState<'_, '_>, mut place: &crate::ScalarPlace) -> bool {
    loop {
        match place {
            crate::ScalarPlace::Field { base, .. } | crate::ScalarPlace::Index { base, .. } => place = base,
            crate::ScalarPlace::Name { .. } => return false,
            crate::ScalarPlace::Dereference { pointer, .. } => return match pointer.as_ref() {
                ScalarExpression::Name { name, .. } => state.storage.get(name).map(|(_, ty)| ty)
                    .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
                    .is_some_and(|ty| matches!(ty, ScalarType::RawPointer(_))),
                ScalarExpression::Call { .. } => match state.prepared_call_output(pointer) {
                    Ok(Some(ty)) => matches!(ty, ScalarType::RawPointer(_)),
                    _ => false,
                },
                ScalarExpression::RawAddress { .. } => true,
                _ => false,
            },
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
    checked: bool,
) -> Result<PointerValue<'ctx>, String> {
    if checked && state.require_release_checked_access {
        let (bytes, _) = allocation_layout(ty, state.layouts, state.target_layout)?;
        let max_offset = match state.target_layout.pointer_size {
            4 => i32::MAX as u64,
            8 => i64::MAX as u64,
            _ => return Err("unsupported fixed-array pointer index width".to_owned()),
        };
        if bytes > max_offset {
            return Err("fixed array exceeds the target's addressable index domain".to_owned());
        }
    }
    let array = storage_type(context, ty, state.layouts, state.target_layout)?;
    // The release helper and domain checks consume the full language-level
    // selector; target-width conversion belongs only at address construction.
    let index = array_address_index(context, state, index)?;
    unsafe {
        state.builder.build_in_bounds_gep(
            array, base, &[context.i32_type().const_zero(), index], "array_element",
        )
    }
    .map_err(builder_error)
}

fn array_address_index<'ctx>(
    context: &'ctx Context, state: &EmitState<'ctx, '_>,
    index: inkwell::values::IntValue<'ctx>,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    if state.target_layout == ScalarTargetLayout::WASM32
        && index.get_type().get_bit_width() > 32
    {
        state
            .builder
            .build_int_truncate(
                index,
                pointer_integer_type(context, state.target_layout),
                "array_address_index",
            )
            .map_err(builder_error)
    } else {
        Ok(index)
    }
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
    let index = emit_place_index(context, state, index, project)?;
    check_fixed_index(context, state, index, length, Some((
        "assignment_index_out_of_bounds", "assignment_index_panic", "assignment_index_continue",
    )))?;
    fixed_array_element_pointer(context, state, &ty, pointer, index, true)
}

pub(super) fn checked_ordinary_snapshot_element_pointer<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    ty: &ScalarType,
    pointer: PointerValue<'ctx>,
    length: u64,
    index: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Result<PointerValue<'ctx>, String> {
    let index = emit_place_index(context, state, index, project)?;
    check_fixed_index(context, state, index, length, Some((
        "snapshot_index_out_of_bounds", "snapshot_index_panic", "snapshot_index_continue",
    )))?;
    fixed_array_element_pointer(context, state, ty, pointer, index, true)
}

fn check_fixed_index<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    index: inkwell::values::IntValue<'ctx>,
    length: u64,
    legacy_guard_labels: Option<(&str, &str, &str)>,
) -> Result<(), String> {
    check_array_extent(context, state, index, context.i64_type().const_int(length, false), legacy_guard_labels)
}

fn check_array_extent<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    index: inkwell::values::IntValue<'ctx>, count: inkwell::values::IntValue<'ctx>,
    legacy_guard_labels: Option<(&str, &str, &str)>,
) -> Result<(), String> {
    if index.get_zero_extended_constant().zip(count.get_zero_extended_constant())
        .is_some_and(|(index, count)| index < count)
    { return Ok(()); }
    if state.project_names.and_then(|names| names.checked_access.as_ref()).is_some() {
        return check_release_index(state, index, count);
    }
    if state.require_release_checked_access {
        return Err("fixed-array index lacks its prepared release helper".to_owned());
    }
    if let Some((invalid_label, panic_label, continue_label)) = legacy_guard_labels {
        let invalid = state.builder.build_int_compare(
            IntPredicate::UGE, index, count, invalid_label,
        ).map_err(builder_error)?;
        emit_checked_place_guard(
            context, state, invalid, panic_label, continue_label,
        )?;
    }
    Ok(())
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

fn ordinary_aggregate_failure<'ctx, 'a>(
    context: &'ctx Context, state: &EmitState<'ctx, '_>,
    members: impl Iterator<Item = &'a ScalarExpression>,
    label: &str,
) -> Result<Option<inkwell::basic_block::BasicBlock<'ctx>>, String> {
    if !state.aggregate.as_ref().is_some_and(|aggregate|
        !aggregate.plan().ordinary_fallible_allocating_expressions.is_empty())
    { return Ok(None); }
    let mut members = members;
    if !members.any(|member|
        super::aggregate_transfer::ordinary_fallible_allocation_expression_at(state, member))
    { return Ok(None); }
    let parent = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or("missing ordinary aggregate literal function")?;
    Ok(Some(context.append_basic_block(parent, label)))
}

fn emit_ordinary_aggregate_member<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>,
    member: &ScalarExpression, ty: &ScalarType, path: ExpressionPath<'_>,
    failure: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    if super::aggregate_transfer::ordinary_fallible_allocation_expression_at(state, member) {
        let (value, _) = super::expression::emit_fallible_numeric_to(context, state, member, ty, path, failure)?;
        return Ok(EmitValue::Basic(value));
    }
    match (path, member) {
        (ExpressionPath::Project { module, modules },
            ScalarExpression::Call { .. } | ScalarExpression::If { .. } | ScalarExpression::Binary { .. }) =>
            emit_project_typed_expression(context, state, member, ty, module, modules),
        _ => emit_typed_expression(context, state, member, ty),
    }
}

fn finish_ordinary_aggregate_fields<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    failure: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<(), String> {
    let success = state.builder.get_insert_block().ok_or("missing successful ordinary aggregate members")?;
    state.builder.position_at_end(failure);
    super::expression::emit_terminal_system_panic(context, state)?;
    state.builder.position_at_end(success);
    Ok(())
}

pub(super) fn stage_ordinary_array_elements<'ctx, 'module>(
    context: &'ctx Context, state: &mut EmitState<'ctx, 'module>,
    elements: &[ScalarExpression], element: &ScalarType, path: ExpressionPath<'_>,
) -> Result<Option<std::vec::IntoIter<EmitValue<'ctx>>>, String> {
    let failure = ordinary_aggregate_failure(context, state, elements.iter(), "ordinary.array_allocation_absent")?;
    let Some(failure) = failure else { return Ok(None) };
    let values = elements.iter().map(|value| emit_ordinary_aggregate_member(context, state, value, element, path, failure))
        .collect::<Result<Vec<_>, _>>()?;
    finish_ordinary_aggregate_fields(context, state, failure)?;
    Ok(Some(values.into_iter()))
}

pub(super) fn emit_struct_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expected: &crate::ScalarType,
    fields: &[crate::ScalarStructLiteralField],
    path: ExpressionPath<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let crate::ScalarType::Struct(id) = expected else {
        return Err("struct literal requires a resolved struct type".to_owned());
    };
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
    let failure = ordinary_aggregate_failure(context, state, fields.iter().map(|field| &field.value),
        "ordinary.struct_allocation_absent")?;
    let values = if let Some(failure) = failure {
        let values = prepare_struct_literal_with(state, id, fields, |state, field, ty|
            emit_ordinary_aggregate_member(context, state, field, ty, path, failure))?;
        finish_ordinary_aggregate_fields(context, state, failure)?;
        values
    } else {
        prepare_struct_literal(context, state, id, fields)?
    };
    write_struct_literal(context, state, values, destination)?;
    Ok(EmitValue::Basic(destination.into()))
}

pub(super) struct PreparedStructLiteral<'ctx, 'module> {
    structure: &'module ScalarStruct,
    values: BTreeMap<&'module str, (&'module ScalarType, Option<EmitValue<'ctx>>)>,
}

pub(super) fn prepare_struct_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    id: &crate::ScalarStructId,
    fields: &[crate::ScalarStructLiteralField],
) -> Result<PreparedStructLiteral<'ctx, 'module>, String> {
    prepare_struct_literal_with(state, id, fields, |state, value, ty|
        emit_typed_expression(context, state, value, ty))
}

pub(super) fn prepare_struct_literal_with<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    id: &crate::ScalarStructId,
    fields: &[crate::ScalarStructLiteralField],
    mut emit_field: impl FnMut(&mut EmitState<'ctx, 'module>, &ScalarExpression, &ScalarType) -> Result<EmitValue<'ctx>, String>,
) -> Result<PreparedStructLiteral<'ctx, 'module>, String> {
    let structure = structure(state.structs, id.clone())?;
    let mut values = BTreeMap::new();
    for field in &structure.fields {
        values.entry(field.name.as_str()).or_insert((&field.ty, None));
    }
    for field in fields {
        let (ty, value) = values
            .get_mut(field.name.as_str())
            .expect("validated struct field");
        *value = Some(emit_field(state, &field.value, ty)?);
    }
    Ok(PreparedStructLiteral { structure, values })
}

pub(super) fn write_struct_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    prepared: PreparedStructLiteral<'ctx, 'module>,
    destination: PointerValue<'ctx>,
) -> Result<(), String> {
    let PreparedStructLiteral { structure, mut values } = prepared;
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
    Ok(())
}

pub(super) fn emit_array_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expected: &ScalarType,
    elements: &[ScalarExpression],
    path: ExpressionPath<'_>,
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
    let mut staged = stage_ordinary_array_elements(context, state, elements, element, path)?;
    for (index, expression) in elements.iter().enumerate() {
        let pointer = literal_array_element_pointer(context, state, storage, destination, index)?;
        let value = match staged.as_mut() {
            Some(staged) => staged.next().expect("one staged value per array element"),
            None => emit_typed_expression(context, state, expression, element)?,
        };
        store_value(context, state, pointer, element, value)?;
    }
    Ok(EmitValue::Basic(destination.into()))
}

fn literal_array_element_pointer<'ctx>(
    context: &'ctx Context, state: &EmitState<'ctx, '_>, storage: BasicTypeEnum<'ctx>,
    destination: PointerValue<'ctx>, index: usize,
) -> Result<PointerValue<'ctx>, String> {
    // The callers authenticate the literal's element count against its fixed
    // layout, so this per-element index is statically in range.
    unsafe {
        state.builder.build_in_bounds_gep(
            storage, destination,
            &[context.i32_type().const_zero(), context.i32_type().const_int(index as u64, false)],
            "array_element",
        )
    }.map_err(builder_error)
}

pub(super) fn write_fixed_array_literal<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>, ty: &ScalarType,
    values: Vec<EmitValue<'ctx>>, destination: PointerValue<'ctx>,
) -> Result<(), String> {
    let ScalarType::Array { element, length, .. } = ty else {
        return Err("guarded array literal requires a fixed layout".into());
    };
    if *length != values.len() as u64 { return Err("invalid guarded array element count".into()); }
    let storage = storage_type(context, ty, state.layouts, state.target_layout)?;
    for (index, value) in values.into_iter().enumerate() {
        let pointer = literal_array_element_pointer(context, state, storage, destination, index)?;
        store_value(context, state, pointer, element, value)?;
    }
    Ok(())
}

pub(super) fn emit_typed_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    expected: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    if super::aggregate_transfer::fallible_allocation_expression_at(state, expression)
        || super::aggregate_transfer::ordinary_fallible_allocation_expression_at(state, expression)
    {
        return emit_contextual_numeric(context, state, expression, expected, ExpressionPath::Single);
    }
    match (expression, expected) {
        (ScalarExpression::Utf8 { value, .. }, ScalarType::Struct(_)) => {
            emit_utf8_literal(context, state, value, expected)
        }
        (ScalarExpression::StructLiteral { fields, .. }, ScalarType::Struct(_)) => {
            emit_struct_literal(context, state, expected, fields, ExpressionPath::Single)
        }
        (ScalarExpression::ArrayLiteral { elements, .. }, ScalarType::Array { .. }) => {
            emit_array_literal(context, state, expected, elements, ExpressionPath::Single)
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
