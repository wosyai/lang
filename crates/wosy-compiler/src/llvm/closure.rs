use super::*;
use crate::scalar::RuntimeBackingKind;

fn capture_value_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    name: &str,
    ty: &ScalarType,
) -> Result<PointerValue<'ctx>, String> {
    if let Some((slot, actual)) = state.storage.get(name).cloned() {
        if actual != *ty {
            return Err(format!("captured binding type differs at {key:?}"));
        }
        if matches!(ty, ScalarType::Callable { .. }) {
            let producer = state
                .literal_captures
                .get(key)
                .ok_or_else(|| format!("missing source-qualified capture producer at {key:?}"))?;
            let CfgPointKind::Capture { .. } = &producer.event else {
                return Err(format!("invalid capture producer at {key:?}"));
            };
            if super::cyclic_callable_backedge(producer, binding) {
                if state
                    .module
                    .get_function(super::ownership::GC_TRIAL_SYMBOL)
                    .is_some()
                {
                    return emit_counted_cyclic_value_cell(context, state, key, binding, name, ty);
                }
                return Err(format!(
                    "unsupported source-qualified cyclic callable value cell at {key:?}, binding {binding:?}: typed cell trace/dispose and collector edge publication are required"
                ));
            }
        } else if super::memory::has_callable_holders(ty, state.structs)? {
            if state
                .literal_captures
                .get(key)
                .is_some_and(|producer| super::cyclic_callable_backedge(producer, binding))
            {
                return Err(format!(
                    "unsupported source-qualified cyclic callable value cell at {key:?}, binding {binding:?}: typed cell trace/dispose and collector edge publication are required"
                ));
            }
        }
        let (size, alignment) = allocation_layout(ty, state.structs, state.target_layout)?;
        let cell = allocated_capture_cell(context, state, size, alignment, false, None)?;
        let stored = state
            .builder
            .build_load(
                storage_type(context, ty, state.structs, state.target_layout)?,
                slot,
                "captured_binding",
            )
            .map_err(builder_error)?;
        super::ownership::begin_graph_mutation(&state.builder, state.module)?;
        state
            .builder
            .build_store(cell, stored)
            .map_err(builder_error)?;
        super::ownership::end_graph_mutation(&state.builder, state.module)?;
        state.storage.insert(name.to_owned(), (cell, ty.clone()));
        state.capture_cells.insert(binding.clone(), cell);
        return Ok(cell);
    }
    let (global, actual) = state
        .globals
        .get(name)
        .ok_or_else(|| format!("missing captured binding storage {binding:?}"))?;
    if actual != ty {
        return Err(format!("captured binding type differs at {key:?}"));
    }
    let cell = global.as_pointer_value();
    state.capture_cells.insert(binding.clone(), cell);
    Ok(cell)
}

pub(super) fn top_level_capture_binding(binding: &crate::scalar::ReferenceBindingId) -> bool {
    binding.function_span == wosy_syntax::ByteSpan::new(0, 0)
        && matches!(binding.kind, crate::scalar::ReferenceBindingKind::Declared)
}

fn counted_array_capture(
    state: &EmitState<'_, '_>,
    binding: &crate::scalar::ReferenceBindingId,
    name: &str,
    ty: &ScalarType,
    input: Option<&super::ownership::ValidatedCapture>,
) -> bool {
    if !matches!(
        ty,
        ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
    ) {
        return false;
    }
    if state.globals.contains_key(name) {
        return false;
    }
    if state.holder_events.covered_cells.contains_key(binding) {
        return false;
    }
    input.is_some_and(|input| {
        input
            .backings
            .iter()
            .any(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
    })
}

fn record_counted_cell_provenance(
    state: &EmitState<'_, '_>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
) {
    if let Some(producer) = state.literal_captures.get(key) {
        producer.counted_cells.borrow_mut().insert(binding.clone());
    }
}

fn capture_installed_elements(
    input: Option<&super::ownership::ValidatedCapture>,
) -> Result<Vec<u64>, String> {
    let Some(input) = input else {
        return Ok(Vec::new());
    };
    let mut installed = Vec::new();
    for fact in input
        .backings
        .iter()
        .flat_map(|fact| &fact.callable_family)
        .flat_map(|family| &family.elements)
        .chain(
            input
                .projected
                .iter()
                .flat_map(|projected| &projected.backing.callable_family)
                .flat_map(|family| &family.elements),
        )
    {
        let crate::scalar::CallableArrayIndexFact::Constant(value) = &fact.index else {
            return Err(format!(
                "captured array has a dynamically installed element without connected index coverage"
            ));
        };
        let (sign, words) = value.to_u64_digits();
        if sign == num_bigint::Sign::Minus || words.len() > 1 {
            return Err(format!(
                "captured array has an installed element outside its valid index domain"
            ));
        }
        installed.push(words.first().copied().unwrap_or(0));
    }
    Ok(installed)
}

fn emit_counted_runtime_array_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    name: &str,
    ty: &ScalarType,
    element: &ScalarType,
    input: Option<&super::ownership::ValidatedCapture>,
) -> Result<PointerValue<'ctx>, String> {
    let length = runtime_length_for_source(
        context,
        state,
        &ScalarExpression::Name {
            name: name.to_owned(),
            span: key.range,
        },
        0,
    )?;
    let installed = capture_installed_elements(input)?;
    let source = load_backing_pointer(context, state, name)?.0;
    let backing = source;
    if !installed.is_empty() && !matches!(element, ScalarType::Callable { .. }) {
        return Err(format!(
            "captured array {binding:?} at {key:?} installs non-callable elements without connected transfer coverage"
        ));
    }
    let cell = allocated_capture_cell(
        context,
        state,
        state.target_layout.pointer_size,
        state.target_layout.pointer_alignment,
        true,
        Some(length),
    )?;
    if matches!(element, ScalarType::Callable { .. }) {
        super::ownership::install_fixed_array_cell_descriptor(
            context, state, key, binding, cell, element,
        )?;
    }
    super::ownership::begin_graph_mutation(&state.builder, state.module)?;
    state
        .builder
        .build_store(cell, backing)
        .map_err(builder_error)?;
    super::ownership::end_graph_mutation(&state.builder, state.module)?;
    super::drop_backing_entry(state, name);
    state.runtime_array_lengths.insert(
        name.to_owned(),
        super::ownership::capture_cell_length_address(context, state, cell)?,
    );
    state.storage.insert(name.to_owned(), (cell, ty.clone()));
    state.capture_cells.insert(binding.clone(), cell);
    state.counted_fixed_cells.insert(binding.clone());
    record_counted_cell_provenance(state, key, binding);
    Ok(cell)
}

fn emit_counted_fixed_array_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    name: &str,
    ty: &ScalarType,
    input: Option<&super::ownership::ValidatedCapture>,
) -> Result<PointerValue<'ctx>, String> {
    let ScalarType::Array {
        element, length, ..
    } = ty
    else {
        return Err(format!(
            "captured fixed callable cell {binding:?} at {key:?} lacks a fixed array type"
        ));
    };
    let (slot, _) = state
        .storage
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing captured binding storage {binding:?}"))?;
    let installed = capture_installed_elements(input)?;
    if !installed.is_empty() && !matches!(&**element, ScalarType::Callable { .. }) {
        return Err(format!(
            "captured array {binding:?} at {key:?} installs non-callable elements without connected transfer coverage"
        ));
    }
    let copied = super::ownership::materialize_fixed_copy(
        context,
        state,
        slot,
        element,
        *length,
        super::ownership::FixedCopyPublication::ConnectedHolder,
    )?;
    let cell = allocated_capture_cell(
        context,
        state,
        state.target_layout.pointer_size,
        state.target_layout.pointer_alignment,
        true,
        Some(context.i64_type().const_int(*length, false)),
    )?;
    if matches!(&**element, ScalarType::Callable { .. }) {
        super::ownership::install_fixed_array_cell_descriptor(
            context, state, key, binding, cell, element,
        )?;
    }
    super::ownership::begin_graph_mutation(&state.builder, state.module)?;
    state
        .builder
        .build_store(cell, copied)
        .map_err(builder_error)?;
    if !installed.is_empty() {
        super::ownership::retain_installed_capture_elements(
            context, state, slot, copied, element, &installed,
        )?;
    }
    super::ownership::end_graph_mutation(&state.builder, state.module)?;
    super::drop_backing_entry(state, name);
    state.runtime_array_lengths.insert(
        name.to_owned(),
        super::ownership::capture_cell_length_address(context, state, cell)?,
    );
    state.storage.insert(name.to_owned(), (cell, ty.clone()));
    state.capture_cells.insert(binding.clone(), cell);
    state.counted_fixed_cells.insert(binding.clone());
    record_counted_cell_provenance(state, key, binding);
    Ok(cell)
}

fn emit_counted_cyclic_value_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    name: &str,
    ty: &ScalarType,
) -> Result<PointerValue<'ctx>, String> {
    let (size, alignment) = allocation_layout(ty, state.structs, state.target_layout)?;
    let cell = allocated_capture_cell(context, state, size, alignment, false, None)?;
    install_scalar_callable_cell_descriptor(context, state, key, cell)?;
    let (slot, _) = state
        .storage
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing captured binding storage {binding:?}"))?;
    let stored = state
        .builder
        .build_load(
            storage_type(context, ty, state.structs, state.target_layout)?,
            slot,
            "captured_binding",
        )
        .map_err(builder_error)?;
    super::ownership::begin_graph_mutation(&state.builder, state.module)?;
    state
        .builder
        .build_store(cell, stored)
        .map_err(builder_error)?;
    super::ownership::end_graph_mutation(&state.builder, state.module)?;
    state.storage.insert(name.to_owned(), (cell, ty.clone()));
    state.capture_cells.insert(binding.clone(), cell);
    record_counted_cell_provenance(state, key, binding);
    Ok(cell)
}

fn install_scalar_callable_cell_descriptor<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    cell: PointerValue<'ctx>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let symbol = super::private_literal_symbol("__wosy_scalar_cell", key);
    let global = if let Some(global) = state.module.get_global(&symbol) {
        global
    } else {
        let trace = state.module.add_function(
            &format!("{symbol}_trace"),
            context
                .void_type()
                .fn_type(&[pointer.into(), pointer.into()], false),
            Some(Linkage::Internal),
        );
        let dispose = state.module.add_function(
            &format!("{symbol}_dispose"),
            context.void_type().fn_type(&[pointer.into()], false),
            Some(Linkage::Internal),
        );
        let descriptor_type = context.struct_type(&[pointer.into(), pointer.into()], false);
        let global = state.module.add_global(descriptor_type, None, &symbol);
        global.set_linkage(Linkage::Private);
        global.set_constant(true);
        global.set_initializer(&descriptor_type.const_named_struct(&[
            trace.as_global_value().as_pointer_value().into(),
            dispose.as_global_value().as_pointer_value().into(),
        ]));
        let caller = state
            .builder
            .get_insert_block()
            .expect("scalar descriptor caller");
        state
            .builder
            .position_at_end(context.append_basic_block(trace, "entry"));
        let current = trace
            .get_first_param()
            .expect("trace cell")
            .into_pointer_value();
        let visitor = trace
            .get_nth_param(1)
            .expect("trace visitor")
            .into_pointer_value();
        visit_scalar_callable_environment(context, state, current, Some(visitor))?;
        state.builder.build_return(None).map_err(builder_error)?;
        state
            .builder
            .position_at_end(context.append_basic_block(dispose, "entry"));
        let current = dispose
            .get_first_param()
            .expect("dispose cell")
            .into_pointer_value();
        visit_scalar_callable_environment(context, state, current, None)?;
        state.builder.build_return(None).map_err(builder_error)?;
        state.builder.position_at_end(caller);
        global
    };
    state
        .builder
        .build_store(
            super::ownership::capture_cell_descriptor_address(context, state, cell)?,
            global.as_pointer_value(),
        )
        .map_err(builder_error)?;
    Ok(())
}

fn visit_scalar_callable_environment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    visitor: Option<PointerValue<'ctx>>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let holder = state
        .builder
        .build_load(
            context.struct_type(&[pointer.into(), pointer.into()], false),
            cell,
            "scalar_cell_holder",
        )
        .map_err(builder_error)?
        .into_struct_value();
    let environment = state
        .builder
        .build_extract_value(holder, 1, "scalar_cell_environment")
        .map_err(builder_error)?
        .into_pointer_value();
    let function = state
        .builder
        .get_insert_block()
        .expect("scalar cell block")
        .get_parent()
        .expect("scalar cell function");
    let present = context.append_basic_block(function, "scalar_cell.present");
    let done = context.append_basic_block(function, "scalar_cell.done");
    let live = state
        .builder
        .build_is_not_null(environment, "scalar_cell_present")
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(live, present, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(present);
    if let Some(visitor) = visitor {
        state
            .builder
            .build_indirect_call(
                context.void_type().fn_type(&[pointer.into()], false),
                visitor,
                &[environment.into()],
                "scalar_cell_trace_environment",
            )
            .map_err(builder_error)?;
    } else {
        super::ownership::drop_environment(context, state, environment)?;
    }
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

fn capture_runtime_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    name: &str,
    ty: &ScalarType,
    input: Option<&super::ownership::ValidatedCapture>,
    descriptor: &super::holder_events::CaptureDescriptorCell,
) -> Result<PointerValue<'ctx>, String> {
    let backings = input
        .map(|input| input.backings.clone())
        .unwrap_or_default();
    let owned = backings
        .iter()
        .filter(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
        .map(|fact| fact.value.clone())
        .collect::<Vec<_>>();
    if let Some(coverage) = state.holder_events.covered_cells.get(binding) {
        if owned.len() != 1
            || !coverage.backings.iter().any(|backing| {
                backing.value == owned[0]
                    && backing.owner.source == binding.source
                    && backing.owner.function_span == binding.function_span
                    && backing.owner.declaration_span == binding.declaration_span
            })
        {
            return Err(format!(
                "capture cell {binding:?} lacks exact source-qualified backing coverage at {key:?}"
            ));
        }
    }
    let source_owned = state
        .runtime_array_owners
        .get(name)
        .copied()
        .unwrap_or(false);
    if backings.is_empty() && source_owned {
        return Err(super::ownership::missing_backing_transport(
            &format!("captured backing `{name}`"),
            (key.range.start, key.range.end),
        ));
    }
    if state.storage.contains_key(name) {
    } else if state.globals.contains_key(name) {
        let (global, actual) = state
            .globals
            .get(name)
            .ok_or_else(|| format!("missing captured binding storage {binding:?}"))?
            .clone();
        if actual != *ty {
            return Err(format!("captured binding type differs at {key:?}"));
        }
        if !matches!(actual, ScalarType::RuntimeArray { .. }) {
            let cell = global.as_pointer_value();
            state.capture_cells.insert(binding.clone(), cell);
            return Ok(cell);
        }
        let cell = global.as_pointer_value();
        if !owned.is_empty() || source_owned {
            let direct = state
                .runtime_array_allocations
                .get(name)
                .copied()
                .unwrap_or(false);
            let length = state
                .runtime_array_lengths
                .get(name)
                .copied()
                .ok_or_else(|| {
                    format!("captured runtime array {name} lacks evaluated length at {key:?}")
                })?;
            super::drop_backing_entry(state, name);
            state.runtime_array_lengths.insert(name.to_owned(), length);
            install_capture_entry(
                context,
                state,
                cell,
                owned.iter().cloned().map(|value| (value, direct)).collect(),
            )?;
        }
        state.capture_cells.insert(binding.clone(), cell);
        return Ok(cell);
    } else {
        return Err(format!("missing captured binding storage {binding:?}"));
    }
    let (slot, actual) = state
        .storage
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing captured binding storage {binding:?}"))?;
    if actual != *ty {
        return Err(format!("captured binding type differs at {key:?}"));
    }
    match actual {
        ScalarType::Array {
            element, length, ..
        } => {
            let copied = super::ownership::materialize_fixed_copy(
                context,
                state,
                slot,
                &element,
                length,
                super::ownership::FixedCopyPublication::RequiresHolderAction,
            )?;
            let cell = allocated_capture_cell(
                context,
                state,
                state.target_layout.pointer_size,
                state.target_layout.pointer_alignment,
                !owned.is_empty(),
                Some(context.i64_type().const_int(length, false)),
            )?;
            state
                .builder
                .build_store(cell, copied)
                .map_err(builder_error)?;
            if !owned.is_empty() && !state.holder_events.covered_cells.contains_key(binding) {
                install_capture_entry(
                    context,
                    state,
                    cell,
                    owned.into_iter().map(|value| (value, false)).collect(),
                )?;
            }
            state.storage.insert(name.to_owned(), (cell, ty.clone()));
            state.capture_cells.insert(binding.clone(), cell);
            Ok(cell)
        }
        ScalarType::RuntimeArray { element, .. } => {
            if counted_array_capture(state, binding, name, ty, input) {
                return emit_counted_runtime_array_cell(
                    context,
                    state,
                    key,
                    binding,
                    name,
                    ty,
                    element.as_ref(),
                    input,
                );
            }
            let (pointer, _) = load_backing_pointer(context, state, name)?;
            let length = runtime_length_for_source(
                context,
                state,
                &ScalarExpression::Name {
                    name: name.to_owned(),
                    span: key.range,
                },
                0,
            )?;
            let top_level = top_level_capture_binding(binding);
            let cell = allocated_capture_cell(
                context,
                state,
                state.target_layout.pointer_size,
                state.target_layout.pointer_alignment,
                (source_owned || !owned.is_empty()) && !top_level,
                Some(length),
            )?;
            state
                .builder
                .build_store(cell, pointer)
                .map_err(builder_error)?;
            if state.holder_events.covered_cells.contains_key(binding)
                && matches!(&descriptor.ty, ScalarType::RuntimeArray { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
            {
                super::ownership::install_capture_cell_descriptor(
                    context, state, cell, descriptor,
                )?;
            }
            if source_owned {
                if !top_level {
                    super::drop_backing_entry(state, name);
                }
                state
                    .runtime_array_allocations
                    .insert(name.to_owned(), false);
            }
            state.runtime_array_lengths.insert(
                name.to_owned(),
                super::ownership::capture_cell_length_address(context, state, cell)?,
            );
            if (!owned.is_empty() || source_owned)
                && !top_level
                && !state
                    .holder_events
                    .covered_cells
                    .get(binding)
                    .is_some_and(|coverage| {
                        matches!(&descriptor.ty, ScalarType::RuntimeArray { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
                            || coverage.environments.iter().all(|environment| {
                                state.covered_literals.contains(&environment.literal)
                            })
                    })
            {
                install_capture_entry(
                    context,
                    state,
                    cell,
                    owned.into_iter().map(|value| (value, false)).collect(),
                )?;
            }
            state.storage.insert(name.to_owned(), (cell, ty.clone()));
            state.capture_cells.insert(binding.clone(), cell);
            if top_level && !state.holder_events.covered_cells.contains_key(binding) {
                state
                    .top_level_remainder_cells
                    .push((binding.clone(), element.as_ref().clone()));
            }
            Ok(cell)
        }
        _ => Err(format!("captured backing `{name}` has an invalid type")),
    }
}

fn install_capture_entry<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    owned: Vec<(crate::scalar::RuntimeValueId, bool)>,
) -> Result<(), String> {
    for (value, direct) in owned {
        let capture = super::ownership::capture_value_key(&value);
        if state
            .runtime_array_owners
            .get(&capture)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        state.runtime_array_owners.insert(capture.clone(), true);
        state
            .runtime_array_allocations
            .insert(capture.clone(), direct);
        state.backing_values.insert(capture.clone(), vec![value]);
        let insert = state
            .builder
            .get_insert_block()
            .expect("capture insertion block");
        let entry = insert
            .get_parent()
            .expect("capture function")
            .get_first_basic_block()
            .expect("capture entry block");
        let first = entry.get_first_instruction();
        let cell_slot = entry_alloca(
            state,
            context.ptr_type(AddressSpace::default()).into(),
            "captured_cell_pointer_slot",
        )?;
        if let Some(first) = first {
            state.builder.position_before(&first);
        } else {
            state.builder.position_at_end(entry);
        }
        state
            .builder
            .build_store(
                cell_slot,
                context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(builder_error)?;
        state.builder.position_at_end(insert);
        state
            .builder
            .build_store(cell_slot, cell)
            .map_err(builder_error)?;
        state.imported_cell_slots.insert(capture.clone(), cell_slot);
        state.backing_slots.insert(
            capture.clone(),
            (
                cell,
                ScalarType::RuntimeArray {
                    element: Box::new(ScalarType::U8),
                    span: wosy_syntax::ByteSpan::new(0, 0),
                },
            ),
        );
        state.backing_captured.insert(capture);
    }
    Ok(())
}

fn capture_struct_backings<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    cell: PointerValue<'ctx>,
    ty: &ScalarType,
    input: Option<&super::ownership::ValidatedCapture>,
) -> Result<(), String> {
    let Some(input) = input else {
        return Ok(());
    };
    if state.counted_fixed_cells.contains(binding) {
        return Ok(());
    }
    if !matches!(ty, ScalarType::Struct(_)) {
        let owned = input
            .backings
            .iter()
            .filter(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
            .count();
        if owned > 0 {
            return Err(format!(
                "validated capture backing lacks a located place at {key:?}"
            ));
        }
        return Ok(());
    }
    let whole = input
        .backings
        .iter()
        .filter(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
        .map(|fact| fact.value.clone())
        .collect::<Vec<_>>();
    super::drop_matching_backing_entries(state, &whole);
    for projected in &input.projected {
        if projected.backing.kind != RuntimeBackingKind::OwnedAllocation {
            continue;
        }
        let mut path = Vec::new();
        let mut place = &projected.place.place;
        loop {
            match place {
                crate::ScalarPlace::Name { .. } => break,
                crate::ScalarPlace::Field { base, field, .. } => {
                    let ScalarFieldReference::Resolved(field) = field else {
                        return Err(format!(
                            "validated capture backing lacks a resolved field at {key:?}"
                        ));
                    };
                    let structure = state
                        .structs
                        .iter()
                        .find(|structure| structure.id == field.structure)
                        .ok_or_else(|| format!("unknown LLVM struct {}", field.structure.index))?;
                    let declared = structure
                        .fields
                        .get(field.index)
                        .filter(|candidate| candidate.id == *field)
                        .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?;
                    path.push(declared.name.clone());
                    place = base;
                }
                _ => {
                    return Err(format!(
                        "validated capture backing lacks a located place at {key:?}"
                    ));
                }
            }
        }
        path.reverse();
        let _address = super::struct_field_address(context, state, cell, ty, &path)?;
        return Err(format!(
            "captured struct backing {binding:?} at {key:?} lacks a connected counted cell route for its owned array storage"
        ));
    }
    Ok(())
}

fn load_backing_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    name: &str,
) -> Result<(PointerValue<'ctx>, ScalarType), String> {
    let (slot, ty) = state
        .storage
        .get(name)
        .cloned()
        .or_else(|| {
            state
                .globals
                .get(name)
                .map(|(global, ty)| (global.as_pointer_value(), ty.clone()))
        })
        .ok_or_else(|| format!("missing captured backing storage for {name}"))?;
    match &ty {
        ScalarType::RuntimeArray { .. } => {
            if state
                .runtime_array_allocations
                .get(name)
                .copied()
                .unwrap_or(false)
            {
                Ok((slot, ty))
            } else {
                let pointer = state
                    .builder
                    .build_load(
                        context.ptr_type(AddressSpace::default()),
                        slot,
                        "captured_backing",
                    )
                    .map_err(builder_error)?
                    .into_pointer_value();
                Ok((pointer, ty))
            }
        }
        ScalarType::Array { .. } => Ok((slot, ty)),
        _ => Err(format!("captured backing `{name}` has an invalid type")),
    }
}

fn allocated_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    size: u64,
    alignment: u64,
) -> Result<PointerValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let allocator = state
        .module
        .get_function(CORE_ALLOC_SYMBOL)
        .expect("core allocator declaration");
    let pointer = state
        .builder
        .build_call(
            allocator,
            &[
                context.i64_type().const_int(size, false).into(),
                context.i64_type().const_int(alignment, false).into(),
            ],
            "closure_allocation",
        )
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .expect("core allocator pointer")
        .into_pointer_value();
    let missing = state
        .builder
        .build_is_null(pointer, "closure_allocation_failed")
        .map_err(builder_error)?;
    let current = state
        .builder
        .get_insert_block()
        .expect("closure insertion block");
    let parent = current.get_parent().expect("closure parent function");
    let fail = context.append_basic_block(parent, "closure_allocation.fail");
    let ready = context.append_basic_block(parent, "closure_allocation.ready");
    state
        .builder
        .build_conditional_branch(missing, fail, ready)
        .map_err(builder_error)?;
    state.builder.position_at_end(fail);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .expect("core panic declaration");
    state
        .builder
        .build_call(panic, &[], "closure_allocation_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(ready);
    Ok(pointer)
}

fn allocated_capture_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    size: u64,
    alignment: u64,
    owns_backing: bool,
    length: Option<inkwell::values::IntValue<'ctx>>,
) -> Result<PointerValue<'ctx>, String> {
    let alignment = alignment.max(8);
    let payload_offset = super::ownership::CELL_GC_PREFIX
        .checked_add(alignment - 1)
        .ok_or("capture cell header alignment overflow")?
        & !(alignment - 1);
    let payload_size = if length.is_some() {
        size.max(8)
            .checked_add(8)
            .ok_or("capture cell length payload overflow")?
    } else {
        size
    };
    let allocation_size = payload_offset
        .checked_add(payload_size)
        .ok_or("capture cell allocation size overflow")?;
    if allocation_size > u64::from(u32::MAX) {
        return Err("capture cell exceeds D-0022 allocation size".to_owned());
    }
    let base = allocated_cell(context, state, allocation_size, alignment)?;
    let cell = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            base,
            &[context.i64_type().const_int(payload_offset, false)],
            "promoted_capture_cell",
        )
    }
    .map_err(builder_error)?;
    let count = super::ownership::capture_cell_count_address(context, state, cell)?;
    let original = super::ownership::capture_cell_base_address(context, state, cell)?;
    let flag = super::ownership::capture_cell_owned_address(context, state, cell)?;
    let descriptor = super::ownership::capture_cell_descriptor_address(context, state, cell)?;
    state
        .builder
        .build_store(
            descriptor,
            context.ptr_type(AddressSpace::default()).const_null(),
        )
        .map_err(builder_error)?;
    super::ownership::init_capture_cell_gc_header(context, state, cell)?;
    state
        .builder
        .build_store(count, context.i64_type().const_int(1, false))
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_store(original, base)
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_store(
            flag,
            context.i64_type().const_int(u64::from(owns_backing), false),
        )
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    if let Some(length) = length {
        state
            .builder
            .build_store(
                super::ownership::capture_cell_length_address(context, state, cell)?,
                length,
            )
            .map_err(builder_error)?
            .set_alignment(8)
            .map_err(|error| error.to_string())?;
    }
    Ok(cell)
}

pub(super) fn emit_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    span: wosy_syntax::ByteSpan,
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let ScalarExpression::FunctionLiteral {
        parameters, body, ..
    } = expression
    else {
        return Err("closure emission requires a function literal".to_owned());
    };
    let key = wosy_syntax::SourceSpan::new(state.source.clone(), span);
    let occurrence = crate::scalar::ScalarCaptureOccurrence::for_literal(span, body);
    let capture_point = state.holder_events.capture(&occurrence)?;
    let signature = state
        .literal_signatures
        .get(&key)
        .ok_or_else(|| format!("missing typed closure signature at {key:?}"))?
        .clone();
    let ScalarType::Callable {
        outputs,
        parameters: parameter_types,
    } = &signature
    else {
        return Err(format!("literal signature is not callable at {key:?}"));
    };
    let capture_event = &capture_point.kind;
    let CfgPointKind::Capture {
        captures,
        signature: validated,
        ..
    } = capture_event
    else {
        return Err(format!("invalid closure capture event at {key:?}"));
    };
    if *validated != signature {
        return Err(format!("closure signature differs from CFG at {key:?}"));
    }
    if !captures.is_empty() {
        if state
            .holder_events
            .counted_environment(match capture_event {
                CfgPointKind::Capture { environment, .. } => environment,
                _ => unreachable!(),
            })
        {
            state.holder_events.capture_initial_handoff(capture_point)?;
        }
    }
    let capture_storage = captures
        .iter()
        .map(|capture| {
            let residence = if state.globals.contains_key(&capture.name) {
                super::ownership::CaptureCellResidence::Global
            } else if state.storage.contains_key(&capture.name) {
                super::ownership::CaptureCellResidence::Promoted
            } else {
                super::ownership::CaptureCellResidence::External
            };
            (capture.binding.clone(), residence)
        })
        .collect::<BTreeMap<_, _>>();
    let descriptor = state.holder_events.capture_descriptor(
        capture_point,
        state.target_layout,
        &capture_storage,
    )?;
    if let Some(actual) = &descriptor {
        let producer = state.literal_captures.get(&key).ok_or_else(|| {
            format!("Capture at {key:?} lacks source-qualified producer registry entry")
        })?;
        if let Some(recorded) = producer.descriptor.get() {
            if recorded != actual {
                return Err(format!(
                    "Capture at {key:?} differs from its imported producer descriptor"
                ));
            }
        } else {
            producer
                .descriptor
                .set(actual.clone())
                .map_err(|_| format!("Capture at {key:?} changed its producer descriptor"))?;
        }
    }
    let reference_cfg = state.holder_events.literal_cfg(capture_point.id)?.clone();
    let mut typed_body = body.clone();
    for (value, output) in typed_body
        .final_output_values
        .iter_mut()
        .zip(&outputs.outputs)
    {
        value.ty = output.ty.clone();
    }
    let literal = ScalarFunction {
        name: format!("literal_{}_{}", span.start, span.end),
        name_span: span,
        signature: signature.clone(),
        parameters: parameters.clone(),
        parameter_spans: match capture_event {
            CfgPointKind::Capture {
                parameter_spans, ..
            } => parameter_spans.clone(),
            _ => unreachable!("validated capture event"),
        },
        body: typed_body.clone(),
        span,
        generic_parameters: Vec::new(),
        overload_arms: Vec::new(),
        reference_cfg: reference_cfg.clone(),
    };
    let owner_outputs = outputs
        .outputs
        .iter()
        .enumerate()
        .filter_map(|(index, output)| {
            matches!(output.ty, ScalarType::CheckedReference { .. }).then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let dynamic_owners = dynamic_return_outputs(&literal);
    let (fresh_return_outputs, branch_return_outputs) = fresh_return_outputs(&literal)?;
    let joined_bindings = joined_return_binding_types(&literal, &branch_return_outputs)?;
    let joined_assignment_owners = joined_assignment_owners(&literal, &joined_bindings)?;
    let mut distinct = BTreeMap::new();
    for capture in captures {
        distinct
            .entry(capture.binding.clone())
            .or_insert((capture.name.clone(), capture.ty.clone()));
    }
    let pointer_type = context.ptr_type(AddressSpace::default());
    let wide_environment = distinct
        .values()
        .any(|(_, ty)| matches!(ty, ScalarType::RuntimeArray { .. }));
    let slot_width = if wide_environment {
        8
    } else {
        state.target_layout.pointer_size
    };
    let slot_alignment = if wide_environment {
        8
    } else {
        state.target_layout.pointer_alignment
    };
    let environment_slot_type: BasicTypeEnum<'ctx> = if wide_environment {
        context.i64_type().into()
    } else {
        pointer_type.into()
    };
    let environment_slots = distinct
        .values()
        .map(|(_, ty)| 1 + usize::from(matches!(ty, ScalarType::RuntimeArray { .. })))
        .sum::<usize>();
    if let Some(descriptor) = &descriptor {
        if descriptor.literal != key
            || descriptor.signature != signature
            || descriptor.cells.len() != distinct.len()
            || descriptor.slot_width != slot_width
            || descriptor.payload_size != environment_slots as u64 * slot_width
            || descriptor
                .cells
                .iter()
                .zip(&distinct)
                .any(|(cell, (binding, (name, ty)))| {
                    &cell.binding != binding
                        || &cell.ty != ty
                        || cell.residence
                            != if state.globals.contains_key(name) {
                                super::ownership::CaptureCellResidence::Global
                            } else {
                                super::ownership::CaptureCellResidence::Promoted
                            }
                })
        {
            return Err(format!(
                "Capture at {key:?} has an inconsistent emitted descriptor layout"
            ));
        }
    } else if !distinct.is_empty() {
        return Err(format!("Capture at {key:?} lacks a typed descriptor"));
    }
    let environment = if distinct.is_empty() {
        pointer_type.const_null()
    } else {
        let payload = (environment_slots as u64)
            .checked_mul(slot_width)
            .ok_or("closure environment size overflows u64")?;
        let size = payload
            .checked_add(super::ownership::ENVIRONMENT_GC_PREFIX)
            .ok_or("closure environment size overflows u64")?;
        if size > u64::from(u32::MAX) {
            return Err(format!("Capture at {key:?} exceeds D-0022 allocation size"));
        }
        let base = allocated_cell(context, state, size, slot_alignment.max(8))?;
        let environment = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                base,
                &[context
                    .i64_type()
                    .const_int(super::ownership::ENVIRONMENT_GC_PREFIX, false)],
                "closure_environment_data",
            )
        }
        .map_err(builder_error)?;
        let count = super::ownership::environment_count_address(context, state, environment)?;
        let count_store = state
            .builder
            .build_store(count, context.i64_type().const_int(1, false))
            .map_err(builder_error)?;
        count_store
            .set_alignment(8)
            .map_err(|error| error.to_string())?;
        let base_slot = super::ownership::environment_base_address(context, state, environment)?;
        state
            .builder
            .build_store(base_slot, base)
            .map_err(builder_error)?;
        let descriptor_global = super::ownership::install_environment_descriptor(
            context,
            state,
            descriptor.as_ref().expect("validated nonempty capture"),
        )?;
        let descriptor_slot =
            super::ownership::environment_descriptor_address(context, state, environment)?;
        state
            .builder
            .build_store(descriptor_slot, descriptor_global.as_pointer_value())
            .map_err(builder_error)?
            .set_alignment(state.target_layout.pointer_alignment as u32)
            .map_err(|error| error.to_string())?;
        super::ownership::init_environment_gc_header(context, state, environment)?;
        environment
    };
    let capture_inputs = state
        .backing
        .captures
        .get(&(span.start, span.end))
        .cloned()
        .unwrap_or_default();
    let mut staged_cells = Vec::new();
    let mut environment_index = 0;
    for (binding, (name, ty)) in &distinct {
        let cell_descriptor = descriptor
            .as_ref()
            .and_then(|descriptor| {
                descriptor
                    .cells
                    .iter()
                    .find(|cell| &cell.binding == binding)
            })
            .ok_or_else(|| {
                format!("Capture at {key:?} lacks a binding descriptor for {binding:?}")
            })?;
        if cell_descriptor.cell_offset != environment_index as u64 * slot_width
            || cell_descriptor.length_offset
                != matches!(ty, ScalarType::RuntimeArray { .. })
                    .then_some((environment_index as u64 + 1) * slot_width)
        {
            return Err(format!(
                "Capture at {key:?} has a mismatched cell slot for {binding:?}"
            ));
        }
        let captured_length = if matches!(ty, ScalarType::RuntimeArray { .. }) {
            Some(runtime_length_for_source(
                context,
                state,
                &ScalarExpression::Name {
                    name: name.clone(),
                    span,
                },
                0,
            )?)
        } else {
            None
        };
        let cell = if let Some(cell) = state.capture_cells.get(binding) {
            *cell
        } else if matches!(ty, ScalarType::RuntimeArray { .. }) {
            let input = capture_inputs
                .iter()
                .find(|input| input.binding == *binding);
            capture_runtime_cell(
                context,
                state,
                &key,
                binding,
                name,
                ty,
                input,
                cell_descriptor,
            )?
        } else if matches!(ty, ScalarType::Array { .. })
            && counted_array_capture(
                state,
                binding,
                name,
                ty,
                capture_inputs
                    .iter()
                    .find(|input| input.binding == *binding),
            )
        {
            let input = capture_inputs
                .iter()
                .find(|input| input.binding == *binding);
            emit_counted_fixed_array_cell(context, state, &key, binding, name, ty, input)?
        } else {
            let input = capture_inputs
                .iter()
                .find(|input| input.binding == *binding);
            let cell = capture_value_cell(context, state, &key, binding, name, ty)?;
            capture_struct_backings(context, state, &key, binding, cell, ty, input)?;
            cell
        };
        let residence = if state.globals.contains_key(name) {
            super::ownership::CaptureCellResidence::Global
        } else {
            super::ownership::CaptureCellResidence::Promoted
        };
        let terminal_owned = state
            .top_level_remainder_cells
            .iter()
            .any(|(known, _)| known == binding);
        staged_cells.push((cell, residence, captured_length, terminal_owned));
        environment_index += 1 + usize::from(matches!(ty, ScalarType::RuntimeArray { .. }));
    }
    super::ownership::begin_graph_mutation(&state.builder, state.module)?;
    super::ownership::register_environment(context, state, environment)?;
    environment_index = 0;
    for (cell, residence, captured_length, terminal_owned) in staged_cells {
        if !terminal_owned {
            super::ownership::retain_capture_cell(context, state, cell, residence)?;
            if matches!(residence, super::ownership::CaptureCellResidence::Promoted) {
                super::ownership::register_capture_cell(context, state, cell)?;
            }
        }
        let element = unsafe {
            state.builder.build_in_bounds_gep(
                environment_slot_type,
                environment,
                &[context
                    .i32_type()
                    .const_int(environment_index as u64, false)],
                "environment_cell",
            )
        }
        .map_err(builder_error)?;
        state
            .builder
            .build_store(element, cell)
            .map_err(builder_error)?;
        environment_index += 1;
        if let Some(length) = captured_length {
            let length_element = unsafe {
                state.builder.build_in_bounds_gep(
                    context.i64_type(),
                    environment,
                    &[context
                        .i32_type()
                        .const_int(environment_index as u64, false)],
                    "environment_length",
                )
            }
            .map_err(builder_error)?;
            state
                .builder
                .build_store(length_element, length)
                .map_err(builder_error)?;
            environment_index += 1;
        }
    }
    super::ownership::end_graph_mutation(&state.builder, state.module)?;
    let ordinary = function_type(context, &signature, state.target_layout)?.0;
    let mut llvm_parameters = vec![pointer_type.into()];
    for ty in parameter_types {
        llvm_parameters.push(basic_type(context, ty, state.target_layout)?.into());
        if matches!(ty, ScalarType::RuntimeArray { .. }) {
            llvm_parameters.push(context.i64_type().into());
        }
    }
    llvm_parameters.extend(
        owner_outputs
            .iter()
            .map(|_| BasicMetadataTypeEnum::from(pointer_type)),
    );
    llvm_parameters.extend(
        outputs
            .outputs
            .iter()
            .filter(|output| matches!(output.ty, ScalarType::RuntimeArray { .. }))
            .map(|_| BasicMetadataTypeEnum::from(pointer_type)),
    );
    let code_type = match ordinary.get_return_type() {
        Some(output) => output.fn_type(&llvm_parameters, false),
        None => context.void_type().fn_type(&llvm_parameters, false),
    };
    let symbol = super::private_literal_symbol("__wosy_closure", &key);
    let code = state
        .module
        .add_function(&symbol, code_type, Some(Linkage::Internal));
    let caller = state
        .builder
        .get_insert_block()
        .expect("closure caller block");
    let entry = context.append_basic_block(code, "entry");
    state.builder.position_at_end(entry);
    let literal_events = super::holder_events::HolderEvents::new(&reference_cfg)?;
    if !literal_events.covered_cells.is_empty() {
        return Err(format!(
            "unsupported source-qualified captured callable literal maker at {:?}:{}..{} lacks connected imported output/field holder transport",
            state.source, span.start, span.end
        ));
    }
    let mut child = EmitState {
        builder: state.builder,
        module: state.module,
        functions: state.functions,
        call_targets: state.call_targets,
        signatures: state.signatures,
        dynamic_owner_outputs: state.dynamic_owner_outputs,
        literal_signatures: state.literal_signatures,
        literal_captures: state.literal_captures,
        literal_reference_cfgs: state.literal_reference_cfgs,
        covered_literals: state.covered_literals.clone(),
        cyclic_literals: state.cyclic_literals.clone(),
        counted_fixed_cells: state.counted_fixed_cells.clone(),
        top_level_remainder_cells: Vec::new(),
        counted_cell_backings: state.counted_cell_backings.clone(),
        resolved_calls: resolved_cfg_calls(&reference_cfg),
        source: state.source.clone(),
        capture_cells: BTreeMap::new(),
        backing: validated_backing_maps(&reference_cfg),
        holder_events: literal_events,
        escaping: state.escaping.clone(),
        backing_values: BTreeMap::new(),
        backing_slots: BTreeMap::new(),
        imported_cell_slots: BTreeMap::new(),
        backing_captured: BTreeSet::new(),
        values: state.values.clone(),
        storage: BTreeMap::new(),
        fresh_return_outputs,
        branch_return_outputs,
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners,
        return_output_types: outputs
            .outputs
            .iter()
            .map(|output| output.ty.clone())
            .collect(),
        return_body_span: Some(body.span),
        checked_call_owners: checked_call_owners(&literal)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        return_length_slots: BTreeMap::new(),
        call_length_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations: mutable_runtime_allocations(&literal),
        invalidations: resolved_invalidations(&literal)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: state.globals.clone(),
        all_globals: state.all_globals.clone(),
        structs: state.structs,
        target_layout: state.target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_joined_return_bindings(context, &mut child, joined_bindings)?;
    let input_count = parameter_types.len()
        + parameter_types
            .iter()
            .filter(|ty| matches!(ty, ScalarType::RuntimeArray { .. }))
            .count();
    for (position, output) in owner_outputs.iter().enumerate() {
        let slot = code
            .get_nth_param((input_count + position + 1) as u32)
            .ok_or_else(|| format!("missing closure return owner slot {output}"))?
            .into_pointer_value();
        child.return_owner_slots.insert(*output, slot);
    }
    let mut output_index = input_count + owner_outputs.len() + 1;
    for (index, output) in outputs.outputs.iter().enumerate() {
        if matches!(output.ty, ScalarType::RuntimeArray { .. }) {
            let slot = code
                .get_nth_param(output_index as u32)
                .ok_or_else(|| format!("missing closure return length slot {index}"))?
                .into_pointer_value();
            child.return_length_slots.insert(index, slot);
            output_index += 1;
        }
    }
    let environment_parameter = code
        .get_nth_param(0)
        .expect("closure environment")
        .into_pointer_value();
    let mut environment_index = 0;
    for (binding, (name, ty)) in &distinct {
        let element = unsafe {
            child.builder.build_in_bounds_gep(
                environment_slot_type,
                environment_parameter,
                &[context
                    .i32_type()
                    .const_int(environment_index as u64, false)],
                "capture_cell",
            )
        }
        .map_err(builder_error)?;
        let cell = child
            .builder
            .build_load(pointer_type, element, "capture_address")
            .map_err(builder_error)?
            .into_pointer_value();
        child.storage.insert(name.clone(), (cell, ty.clone()));
        child.capture_cells.insert(binding.clone(), cell);
        environment_index += 1;
        if matches!(ty, ScalarType::RuntimeArray { .. }) {
            let element = unsafe {
                child.builder.build_in_bounds_gep(
                    context.i64_type(),
                    environment_parameter,
                    &[context
                        .i32_type()
                        .const_int(environment_index as u64, false)],
                    "capture_length_cell",
                )
            }
            .map_err(builder_error)?;
            let length = if child.globals.contains_key(name) {
                element
            } else {
                super::ownership::capture_cell_length_address(context, &child, cell)?
            };
            child.runtime_array_lengths.insert(name.clone(), length);
            environment_index += 1;
        }
    }
    let mut argument_index = 1;
    for (index, (name, ty)) in parameters.iter().zip(parameter_types).enumerate() {
        let slot = entry_alloca(
            &child,
            storage_type(context, ty, child.structs, child.target_layout)?,
            name,
        )?;
        let argument = code
            .get_nth_param(argument_index)
            .expect("closure argument");
        argument_index += 1;
        store_value(context, &mut child, slot, ty, EmitValue::Basic(argument))?;
        child.storage.insert(name.clone(), (slot, ty.clone()));
        if matches!(ty, ScalarType::RuntimeArray { .. }) {
            let length = code
                .get_nth_param(argument_index)
                .ok_or_else(|| format!("missing closure parameter length {name}"))?
                .into_int_value();
            argument_index += 1;
            let length_slot =
                entry_alloca(&child, context.i64_type().into(), &format!("{name}_length"))?;
            child
                .builder
                .build_store(length_slot, length)
                .map_err(builder_error)?;
            child
                .runtime_array_lengths
                .insert(name.clone(), length_slot);
            child.runtime_array_owners.insert(name.clone(), true);
            child.runtime_array_allocations.insert(name.clone(), false);
            if let Some(param_span) = literal.parameter_spans.get(index) {
                let values = super::param_backing_values(&child, span, body.span, *param_span);
                if !values.is_empty() {
                    child.backing_values.insert(name.clone(), values);
                }
            }
        }
    }
    let mut result = match project {
        Some(scope) => emit_project_block(
            context,
            &mut child,
            &typed_body,
            scope.module,
            scope.modules,
        )?,
        None => emit_block(context, &mut child, &typed_body)?,
    };
    if let [ScalarBlockItem::Expression(expression)] = body.items.as_slice() {
        result = super::apply_expression_body_return(
            context, &mut child, body, outputs, expression, result,
        )?;
        super::emit_expression_body_return_handoff(context, &mut child, &typed_body, &result)?;
    }
    if let [ScalarBlockItem::Expression(expression)] = body.items.as_slice() {
        for output in &dynamic_owners {
            record_forwarded_return_owner(context, &mut child, expression, *output)?;
        }
        super::validate_return_holder_events(&child, &typed_body)?;
        let exit = child.holder_events.point(
            &child.source,
            body.span,
            super::holder_events::HolderPointKind::ScopeExit,
        )?;
        child.holder_events.validated(exit)?;
    }
    for output in owner_outputs {
        if dynamic_owners.contains(&output) {
            continue;
        }
        let fresh = literal
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::ReturnOutput {
                    output: index,
                    checked: true,
                    ..
                } if *index == output => point.possible_origins.as_ref(),
                _ => None,
            })
            .flatten()
            .any(|origin| {
                matches!(
                    literal.reference_cfg.facts[origin.0],
                    ReferenceOrigin::Fresh { .. }
                )
            });
        child.return_owner_flags.insert(
            output,
            context.bool_type().const_int(u64::from(fresh), false),
        );
    }
    let prepared = super::return_memory::prepare_return(context, &mut child, result, outputs)?;
    release_automatic_return_result_owners(context, &mut child, &BTreeMap::new())?;
    release_captured_backings(context, &mut child)?;
    emit_return(&child, prepared)?;
    state.builder.position_at_end(caller);
    let layout = basic_type(context, &signature, state.target_layout)?.into_struct_type();
    let code_pointer = code.as_global_value().as_pointer_value();
    let callable = state
        .builder
        .build_insert_value(layout.const_zero(), code_pointer, 0, "closure_code")
        .map_err(builder_error)?;
    let callable = state
        .builder
        .build_insert_value(callable, environment, 1, "closure_environment")
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(callable.into_struct_value().into()))
}
