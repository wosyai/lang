use super::*;

fn imported_capture_key(
    value: &RuntimeValueId,
    call_id: usize,
    output: usize,
    path: &[String],
    index: u64,
) -> String {
    format!(
        "{}:call{}:{}:{}:{}",
        ownership::capture_value_key(value),
        call_id,
        output,
        path.join("."),
        index
    )
}

fn callable_leaf_paths(ty: &ScalarType, structs: &[ScalarStruct]) -> Vec<Vec<String>> {
    match ty {
        ScalarType::Callable { .. } => vec![Vec::new()],
        ScalarType::Struct(id) => {
            let Some(structure) = structs.iter().find(|structure| structure.id == *id) else {
                return Vec::new();
            };
            let mut paths = Vec::new();
            for field in &structure.fields {
                for mut sub in callable_leaf_paths(&field.ty, structs) {
                    let mut path = vec![field.name.clone()];
                    path.append(&mut sub);
                    paths.push(path);
                }
            }
            paths
        }
        _ => Vec::new(),
    }
}

fn extract_callable_environment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    result: &EmitValue<'ctx>,
    outputs: &[ScalarType],
    position: usize,
    path: &[String],
) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
    fn environment_from_callable<'ctx>(
        state: &EmitState<'ctx, '_>,
        callable: StructValue<'ctx>,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
        let code = state
            .builder
            .build_extract_value(callable, 0, "import_code")
            .map_err(builder_error)?
            .into_pointer_value();
        let environment = state
            .builder
            .build_extract_value(callable, 1, "import_environment")
            .map_err(builder_error)?
            .into_pointer_value();
        Ok((code, environment))
    }
    if path.is_empty() {
        let ty = outputs
            .get(position)
            .ok_or_else(|| "callable import output is missing".to_owned())?
            .clone();
        if !matches!(ty, ScalarType::Callable { .. }) {
            return Err("callable import found a non-callable output".to_owned());
        }
        if outputs.len() == 1 {
            let EmitValue::Basic(value) = result else {
                return Err("callable import needs a basic call result".to_owned());
            };
            return match *value {
                BasicValueEnum::StructValue(callable) => environment_from_callable(state, callable),
                BasicValueEnum::PointerValue(pointer) => {
                    let loaded = state
                        .builder
                        .build_load(
                            storage_type(context, &ty, state.structs, state.target_layout)?,
                            pointer,
                            "import_callable",
                        )
                        .map_err(builder_error)?
                        .into_struct_value();
                    environment_from_callable(state, loaded)
                }
                _ => Err("callable import needs a struct call result".to_owned()),
            };
        }
        let EmitValue::Aggregate { value, .. } = result else {
            return Err("callable import needs an aggregate call result".to_owned());
        };
        let extracted = state
            .builder
            .build_extract_value(*value, position as u32, "import_output")
            .map_err(builder_error)?;
        return match extracted {
            BasicValueEnum::StructValue(callable) => environment_from_callable(state, callable),
            BasicValueEnum::PointerValue(pointer) => {
                let loaded = state
                    .builder
                    .build_load(
                        storage_type(context, &ty, state.structs, state.target_layout)?,
                        pointer,
                        "import_callable",
                    )
                    .map_err(builder_error)?
                    .into_struct_value();
                environment_from_callable(state, loaded)
            }
            _ => Err("callable import needs a struct call result".to_owned()),
        };
    }
    let base_ty = outputs
        .get(position)
        .ok_or_else(|| "callable import output is missing".to_owned())?
        .clone();
    let mut current_ptr = if outputs.len() == 1 {
        let EmitValue::Basic(value) = result else {
            return Err("callable import needs a basic call result".to_owned());
        };
        match *value {
            BasicValueEnum::PointerValue(pointer) => pointer,
            _ => return Err("callable import needs a struct pointer result".to_owned()),
        }
    } else {
        let EmitValue::Aggregate { value, .. } = result else {
            return Err("callable import needs an aggregate call result".to_owned());
        };
        let extracted = state
            .builder
            .build_extract_value(*value, position as u32, "import_output")
            .map_err(builder_error)?;
        match extracted {
            BasicValueEnum::PointerValue(pointer) => pointer,
            _ => return Err("callable import needs a struct pointer result".to_owned()),
        }
    };
    let mut current_ty = base_ty;
    for name in path {
        let ScalarType::Struct(id) = &current_ty else {
            return Err("callable import field path crosses a non-struct value".to_owned());
        };
        let structure = state
            .structs
            .iter()
            .find(|structure| structure.id == *id)
            .ok_or_else(|| "callable import found an unknown struct".to_owned())?;
        let (offset, field_ty) = structure
            .fields
            .iter()
            .find(|field| field.name == *name)
            .map(|field| (field.offset, field.ty.clone()))
            .ok_or_else(|| "callable import found an unknown struct field".to_owned())?;
        let offset =
            offset.ok_or_else(|| "callable import field has no valid offset".to_owned())?;
        current_ptr = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                current_ptr,
                &[context.i8_type().const_int(offset, false)],
                "import_field",
            )
        }
        .map_err(builder_error)?;
        current_ty = field_ty;
    }
    if !matches!(current_ty, ScalarType::Callable { .. }) {
        return Err("callable import found a non-callable field".to_owned());
    }
    let callable = state
        .builder
        .build_load(
            storage_type(context, &current_ty, state.structs, state.target_layout)?,
            current_ptr,
            "import_callable",
        )
        .map_err(builder_error)?
        .into_struct_value();
    environment_from_callable(state, callable)
}

fn import_field_indices(
    state: &EmitState<'_, '_>,
    output: &ScalarType,
    path: &[String],
) -> Result<Vec<usize>, String> {
    let mut ty = output.clone();
    let mut fields = Vec::with_capacity(path.len());
    for name in path {
        let ScalarType::Struct(id) = ty else {
            return Err(format!(
                "imported callable field {name} crosses a non-struct output"
            ));
        };
        let structure = structure(state.structs, id)?;
        let field = structure
            .fields
            .iter()
            .find(|field| field.name == *name)
            .ok_or_else(|| format!("missing imported callable field {name}"))?;
        fields.push(field.id.index);
        ty = field.ty.clone();
    }
    if !matches!(ty, ScalarType::Callable { .. }) {
        return Err("imported holder path does not select a callable".to_owned());
    }
    Ok(fields)
}

fn entry_import_holder_slot<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    pointer: bool,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    let insert = state
        .builder
        .get_insert_block()
        .ok_or("missing imported holder insertion block")?;
    let entry = insert
        .get_parent()
        .ok_or("missing imported holder function")?
        .get_first_basic_block()
        .ok_or("missing imported holder entry")?;
    let first = entry.get_first_instruction();
    let ty = if pointer {
        context.ptr_type(AddressSpace::default()).into()
    } else {
        context.bool_type().into()
    };
    let slot = entry_alloca(state, ty, name)?;
    if let Some(first) = first {
        state.builder.position_before(&first);
    } else {
        state.builder.position_at_end(entry);
    }
    if pointer {
        state
            .builder
            .build_store(slot, context.ptr_type(AddressSpace::default()).const_null())
            .map_err(builder_error)?;
    } else {
        state
            .builder
            .build_store(slot, context.bool_type().const_zero())
            .map_err(builder_error)?;
    }
    state.builder.position_at_end(insert);
    Ok(slot)
}

pub(super) fn imported_producer_descriptor<'state, 'ctx>(
    state: &'state EmitState<'ctx, '_>,
    literal: &wosy_syntax::SourceSpan,
) -> Result<&'state holder_events::CaptureDescriptor, String> {
    let producer = state.literal_captures.get(literal).ok_or_else(|| {
        format!("source-qualified imported capture {literal:?} lacks a Wosy producer")
    })?;
    if producer.descriptor.get().is_none() {
        let events = holder_events::HolderEvents::new(&producer.parent)?;
        let point = producer
            .parent
            .points
            .get(producer.point.0)
            .ok_or_else(|| {
                format!("source-qualified imported capture {literal:?} lacks producer point")
            })?;
        let CfgPointKind::Capture { captures, .. } = &point.kind else {
            return Err(format!(
                "source-qualified imported capture {literal:?} has no Capture producer"
            ));
        };
        let storage = captures.iter().map(|capture| {
            let global = producer.parent.points.iter().any(|candidate| {
                matches!(&candidate.kind, CfgPointKind::Bind { binding } if binding == &capture.binding)
                    && producer.parent.top_level.bindings.iter().any(|entry| {
                        matches!(&entry.execution, crate::scalar::ScalarTopLevelBindingExecution::Connected { points }
                            if points.contains(&candidate.id))
                    })
            });
            let residence = if global {
                match state.all_globals.get(&capture.name) {
                    Some((_, ty)) if ty == &capture.ty => ownership::CaptureCellResidence::Global,
                    _ => ownership::CaptureCellResidence::External,
                }
            } else {
                ownership::CaptureCellResidence::Promoted
            };
            (capture.binding.clone(), residence)
        }).collect();
        let descriptor = events
            .capture_descriptor(point, state.target_layout, &storage)?
            .ok_or_else(|| {
                format!("source-qualified imported capture {literal:?} has no owned capture layout")
            })?;
        producer.descriptor.set(descriptor).map_err(|_| {
            format!("source-qualified imported capture {literal:?} changed producer descriptor")
        })?;
    }
    Ok(producer
        .descriptor
        .get()
        .expect("validated producer descriptor"))
}

fn record_imported_holder<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    output: usize,
    fields: Vec<usize>,
    code: PointerValue<'ctx>,
    environment: PointerValue<'ctx>,
) -> Result<ownership::ImportedHolderKey, String> {
    let (point, holder) = state
        .holder_events
        .imported_output_holder(output, &fields)?;
    let function_span = state.holder_events.function_span();
    let key = ownership::ImportedHolderKey {
        source: state.source.clone(),
        function_span,
        call_point: point,
        output,
        fields,
    };
    if !state.holder_events.imported_holders.contains_key(&key) {
        let code_slot = entry_import_holder_slot(context, state, true, "import_holder_code")?;
        let environment_slot =
            entry_import_holder_slot(context, state, true, "import_holder_environment")?;
        let present_slot =
            entry_import_holder_slot(context, state, false, "import_holder_present")?;
        state.holder_events.imported_holders.insert(
            key.clone(),
            ownership::ImportedCaptureHolder {
                holder: holder.clone(),
                code: code_slot,
                environment: environment_slot,
                present: present_slot,
                owned: Vec::new(),
            },
        );
    }
    let slots = &state.holder_events.imported_holders[&key];
    if slots.holder != holder {
        return Err(format!(
            "imported call holder {:?} changed scalar place",
            key
        ));
    }
    state
        .builder
        .build_store(slots.code, code)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.environment, environment)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slots.present, context.bool_type().const_zero())
        .map_err(builder_error)?;
    Ok(key)
}

fn provenanced_counted_cell<'ctx>(
    state: &EmitState<'ctx, '_>,
    literal: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    fact: &crate::scalar::RuntimeBackingFact,
    producer_cell: &holder_events::CaptureDescriptorCell,
    element: &ScalarType,
) -> bool {
    if producer_cell.residence != ownership::CaptureCellResidence::Promoted {
        return false;
    }
    if !producer_cell.backings.contains(fact) {
        return false;
    }
    let Some(producer) = state.literal_captures.get(literal) else {
        return false;
    };
    if !producer.counted_cells.borrow().contains(binding) {
        return false;
    }
    let cell_element = match &producer_cell.ty {
        ScalarType::RuntimeArray { element, .. } | ScalarType::Array { element, .. } => {
            element.as_ref()
        }
        _ => return false,
    };
    if !crate::scalar::scalar_type_equal(cell_element, element) {
        return false;
    }
    true
}

pub(super) fn import_call_result_captures<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &str,
    result: &EmitValue<'ctx>,
    outputs: &[ScalarType],
) -> Result<(), String> {
    let Some(escapes) = state.escaping.get(target).cloned() else {
        return Ok(());
    };
    if escapes.is_empty() {
        return Ok(());
    }
    let pointer_type = context.ptr_type(AddressSpace::default());
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    for (position, ty) in outputs.iter().enumerate() {
        for path in callable_leaf_paths(ty, state.structs) {
            let (code, environment) =
                extract_callable_environment(context, state, result, outputs, position, &path)?;
            let fields = import_field_indices(state, ty, &path)?;
            let holder_key =
                record_imported_holder(context, state, position, fields, code, environment)?;
            let mut by_literal: Vec<(wosy_syntax::SourceSpan, Vec<&ownership::ValidatedEscape>)> =
                Vec::new();
            for escape in &escapes {
                if let Some((_, group)) = by_literal
                    .iter_mut()
                    .find(|(literal, _)| literal == &escape.literal)
                {
                    group.push(escape);
                } else {
                    by_literal.push((escape.literal.clone(), vec![escape]));
                }
            }
            for (literal, group) in &by_literal {
                let first = group[0];
                let descriptor = imported_producer_descriptor(state, literal)?.clone();
                let symbol = private_literal_symbol("__wosy_closure", &first.literal);
                let literal_code = state
                    .module
                    .get_function(&symbol)
                    .ok_or_else(|| format!("missing validated closure code {symbol}"))?
                    .as_global_value()
                    .as_pointer_value();
                let address_type = pointer_integer_type(context, state.target_layout);
                let actual = state
                    .builder
                    .build_ptr_to_int(code, address_type, "import_code_address")
                    .map_err(builder_error)?;
                let expected = state
                    .builder
                    .build_ptr_to_int(literal_code, address_type, "import_literal_address")
                    .map_err(builder_error)?;
                let matches = state
                    .builder
                    .build_int_compare(IntPredicate::EQ, actual, expected, "import_literal_matches")
                    .map_err(builder_error)?;
                for escape in group {
                    let producer_cell = descriptor.cells.iter().find(|cell| cell.binding == escape.binding)
                        .ok_or_else(|| format!("source-qualified imported output {holder_key:?} lacks producer cell {:?} at {literal:?}", escape.binding))?;
                    let index = descriptor
                        .cells
                        .iter()
                        .position(|cell| cell.binding == escape.binding)
                        .expect("validated producer cell");
                    let cell_offset = producer_cell.cell_offset;
                    let owned = escape
                        .backings
                        .iter()
                        .filter(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
                        .cloned()
                        .collect::<Vec<_>>();
                    if owned.is_empty() {
                        continue;
                    }
                    let current = state
                        .builder
                        .get_insert_block()
                        .ok_or("missing import block")?;
                    let function = current.get_parent().ok_or("missing import function")?;
                    let matched = context.append_basic_block(function, "import_capture.match");
                    let next = context.append_basic_block(function, "import_capture.next");
                    state
                        .builder
                        .build_conditional_branch(matches, matched, next)
                        .map_err(builder_error)?;
                    state.builder.position_at_end(matched);
                    let element = unsafe {
                        state.builder.build_in_bounds_gep(
                            context.i8_type(),
                            environment,
                            &[context.i64_type().const_int(cell_offset, false)],
                            "import_cell",
                        )
                    }
                    .map_err(builder_error)?;
                    let matched_cell = state
                        .builder
                        .build_load(pointer_type, element, "import_capture")
                        .map_err(builder_error)?
                        .into_pointer_value();
                    let holder = state
                        .holder_events
                        .imported_holders
                        .get(&holder_key)
                        .ok_or_else(|| format!("missing typed imported holder {holder_key:?}"))?;
                    state
                        .builder
                        .build_store(holder.present, context.bool_type().const_int(1, false))
                        .map_err(builder_error)?;
                    state
                        .builder
                        .build_unconditional_branch(next)
                        .map_err(builder_error)?;
                    state.builder.position_at_end(next);
                    let selected = state
                        .builder
                        .build_phi(pointer_type, "import_selected_cell")
                        .map_err(builder_error)?;
                    selected.add_incoming(&[
                        (&matched_cell, matched),
                        (&pointer_type.const_null(), current),
                    ]);
                    let cell = selected.as_basic_value().into_pointer_value();
                    for fact in owned {
                        if !producer_cell.backings.contains(&fact) {
                            return Err(format!("source-qualified imported output {holder_key:?} at {literal:?} lacks producer backing for {:?}: {fact:?}", escape.binding));
                        }
                        if producer_cell.residence == ownership::CaptureCellResidence::External {
                            return Err(format!("source-qualified imported output {holder_key:?} at {literal:?} has uncontracted external producer cell {:?}", escape.binding));
                        }
                        let element = match &producer_cell.ty {
                            ScalarType::RuntimeArray { element, .. } | ScalarType::Array { element, .. } => element.as_ref().clone(),
                            _ => return Err(format!("source-qualified imported output {holder_key:?} has non-array producer backing at {literal:?}")),
                        };
                        if fact
                            .callable_family
                            .iter()
                            .any(|family| family.element != element)
                        {
                            return Err(format!("source-qualified imported output {holder_key:?} differs from producer callable element layout at {literal:?}"));
                        }
                        if state.covered_literals.contains(&escape.literal)
                            && !state
                                .holder_events
                                .imported_holder_has_literal(&holder_key, &escape.literal)
                        {
                            continue;
                        }
                        if provenanced_counted_cell(
                            state,
                            literal,
                            &escape.binding,
                            &fact,
                            producer_cell,
                            &element,
                        ) {
                            state
                                .holder_events
                                .imported_holders
                                .get_mut(&holder_key)
                                .ok_or_else(|| {
                                    format!("missing typed imported holder {holder_key:?}")
                                })?
                                .owned
                                .push(ownership::ImportedCaptureCandidate {
                                    literal: escape.literal.clone(),
                                    backing: ownership::EnvironmentOwnedBacking {
                                        fact: fact.clone(),
                                        cell_offset,
                                        element: element.clone(),
                                    },
                                    ownership: ownership::ImportedCaptureOwnership::CountedCell,
                                });
                            continue;
                        }
                        let ownership = if state.covered_literals.contains(&escape.literal) {
                            let call_span = state.holder_events.source_span(
                                holder_events::HolderEmissionEdge::Terminal {
                                    from: holder_key.call_point,
                                },
                            )?;
                            let coverage = state.holder_events.imported_environment_covered(
                                &holder_key, &descriptor, &escape.binding, &fact,
                            )?.ok_or_else(|| format!(
                                "source-qualified imported output {holder_key:?} for {:?} at {:?} lacks complete connected holder transfer and terminal release",
                                escape.literal,
                                call_span
                            ))?;
                            ownership::ImportedCaptureOwnership::Counted(coverage)
                        } else {
                            ownership::ImportedCaptureOwnership::Synthetic
                        };
                        let counted =
                            matches!(ownership, ownership::ImportedCaptureOwnership::Counted(_));
                        state
                            .holder_events
                            .imported_holders
                            .get_mut(&holder_key)
                            .ok_or_else(|| format!("missing typed imported holder {holder_key:?}"))?
                            .owned
                            .push(ownership::ImportedCaptureCandidate {
                                literal: escape.literal.clone(),
                                backing: ownership::EnvironmentOwnedBacking {
                                    fact: fact.clone(),
                                    cell_offset,
                                    element: element.clone(),
                                },
                                ownership,
                            });
                        if counted {
                            continue;
                        }
                        if state.counted_cell_backings.contains(&fact.origin) {
                            continue;
                        }
                        if state.literal_captures.values().any(|producer| {
                            producer.counted_cells.borrow().contains(&escape.binding)
                        }) {
                            continue;
                        }
                        if producer_cell.residence == ownership::CaptureCellResidence::Global {
                            state
                                .holder_events
                                .imported_holders
                                .get_mut(&holder_key)
                                .ok_or_else(|| {
                                    format!("missing typed imported holder {holder_key:?}")
                                })?
                                .owned
                                .push(ownership::ImportedCaptureCandidate {
                                    literal: escape.literal.clone(),
                                    backing: ownership::EnvironmentOwnedBacking {
                                        fact: fact.clone(),
                                        cell_offset,
                                        element: element.clone(),
                                    },
                                    ownership: ownership::ImportedCaptureOwnership::CountedCell,
                                });
                            continue;
                        }
                        let value = fact.value;
                        let call_id = state.next_block;
                        state.next_block += 1;
                        let key =
                            imported_capture_key(&value, call_id, position, &path, index as u64);
                        if state
                            .runtime_array_owners
                            .get(&key)
                            .copied()
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        state.runtime_array_owners.insert(key.clone(), true);
                        state.runtime_array_allocations.insert(key.clone(), false);
                        state.backing_values.insert(key.clone(), vec![value]);
                        state.backing_slots.insert(
                            key.clone(),
                            (
                                cell,
                                ScalarType::RuntimeArray {
                                    element: Box::new(element),
                                    span: wosy_syntax::ByteSpan::new(0, 0),
                                },
                            ),
                        );
                        let selected_cell = entry_import_holder_slot(
                            context,
                            state,
                            true,
                            "imported_capture_cell_slot",
                        )?;
                        state
                            .builder
                            .build_store(selected_cell, cell)
                            .map_err(builder_error)?;
                        state.imported_cell_slots.insert(key.clone(), selected_cell);
                        state.backing_captured.insert(key);
                    }
                }
            }
        }
    }
    ownership::end_graph_mutation(&state.builder, state.module)?;
    Ok(())
}
