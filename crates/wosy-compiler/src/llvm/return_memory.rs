use super::*;

pub(super) fn transfer_automatic_return_result(
    state: &mut EmitState<'_, '_>,
    expression: &ScalarExpression,
) {
    if let ScalarExpression::Name { name, .. } = expression {
        if let Some(result) = state.automatic_return_result_owners.get_mut(name) {
            result.state = ScalarAutomaticReturnResultState::Transferred;
        }
        state.automatic_return_result_owners.remove(name);
    }
}

pub(super) fn record_forwarded_return_owner<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    output: usize,
) -> Result<(), String> {
    if !state.return_owner_slots.contains_key(&output) {
        return Ok(());
    }
    let owner = match expression {
        ScalarExpression::Call { span, .. } => match state
            .checked_call_owners
            .get(&(span.start, span.end, output))
            .ok_or_else(|| format!("missing checked return call at {span:?}"))?
        {
            CheckedCallOwner::Mixed => state
                .call_owner_flags
                .get(&output)
                .copied()
                .ok_or_else(|| format!("missing mixed return owner at {span:?}"))?,
            CheckedCallOwner::Fresh => context.bool_type().const_int(1, false),
            CheckedCallOwner::Borrowed => context.bool_type().const_zero(),
        },
        ScalarExpression::Name { name, .. } => match state.checked_owner_flags.get(name) {
            Some(flag) => *flag,
            None if state.automatic_return_result_owners.contains_key(name) => {
                context.bool_type().const_int(1, false)
            }
            None => return Err(format!("missing validated return owner for {name}")),
        },
        _ => return Ok(()),
    };
    state.return_owner_flags.insert(output, owner);
    Ok(())
}

pub(super) fn emit_automatic_return_result<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: BasicValueEnum<'ctx>,
    aggregate_type: &ScalarType,
) -> Result<BasicValueEnum<'ctx>, String> {
    let address = value.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let null_block = context.append_basic_block(
        function,
        &format!("return_result_null_{}", state.next_block),
    );
    state.next_block += 1;
    let copy_block = context.append_basic_block(
        function,
        &format!("return_result_copy_{}", state.next_block),
    );
    state.next_block += 1;
    let merge_block = context.append_basic_block(
        function,
        &format!("return_result_merge_{}", state.next_block),
    );
    state.next_block += 1;
    let is_null = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            address,
            address.get_type().const_int(0, false),
            "return_result_is_null",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(is_null, null_block, copy_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(null_block);
    state
        .builder
        .build_unconditional_branch(merge_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(copy_block);
    let destination = emit_runtime_array_allocation(
        context,
        state,
        aggregate_type,
        context.i64_type().const_int(1, false),
    )?;
    let source = state
        .builder
        .build_int_to_ptr(
            address,
            context.ptr_type(AddressSpace::default()),
            "return_result_source",
        )
        .map_err(builder_error)?;
    let aggregate = storage_type(context, aggregate_type, state.structs, state.target_layout)?;
    let copied = state
        .builder
        .build_load(aggregate, source, "return_result_value")
        .map_err(builder_error)?;
    state
        .builder
        .build_store(destination, copied)
        .map_err(builder_error)?;
    let heap_address = state
        .builder
        .build_ptr_to_int(
            destination,
            pointer_integer_type(context, state.target_layout),
            "return_result_address",
        )
        .map_err(builder_error)?;
    let copy_end = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    state
        .builder
        .build_unconditional_branch(merge_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge_block);
    let phi = state
        .builder
        .build_phi(address.get_type(), "return_result")
        .map_err(builder_error)?;
    phi.add_incoming(&[
        (
            &BasicValueEnum::IntValue(address.get_type().const_int(0, false)),
            null_block,
        ),
        (&BasicValueEnum::IntValue(heap_address), copy_end),
    ]);
    Ok(phi.as_basic_value())
}

pub(super) fn emit_return<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: EmitValue<'ctx>,
    outputs: &crate::ScalarOutputSequence,
) -> Result<(), String> {
    let value = materialize_fresh_return_outputs(context, state, value, outputs)?;
    for (output, slot) in &state.return_owner_slots {
        let owned = state
            .return_owner_flags
            .get(output)
            .copied()
            .ok_or_else(|| format!("mixed return output {output} lacks path-specific owner"))?;
        state
            .builder
            .build_store(*slot, owned)
            .map_err(builder_error)?;
    }
    match outputs.outputs.as_slice() {
        [] => {
            state.builder.build_return(None).map_err(builder_error)?;
        }
        [output] if output.ty != ScalarType::Unit => {
            let value = take_basic(value)?;
            state
                .builder
                .build_return(Some(&value))
                .map_err(builder_error)?;
        }
        [_output] => {
            state.builder.build_return(None).map_err(builder_error)?;
        }
        _ => {
            let EmitValue::Aggregate { value, .. } = value else {
                return Err("multi-output function requires an aggregate value".to_owned());
            };
            state
                .builder
                .build_return(Some(&value))
                .map_err(builder_error)?;
        }
    }
    Ok(())
}

pub(super) fn materialize_fresh_return_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: EmitValue<'ctx>,
    outputs: &crate::ScalarOutputSequence,
) -> Result<EmitValue<'ctx>, String> {
    if state.fresh_return_outputs.is_empty() {
        return Ok(value);
    }
    if outputs.outputs.len() == 1 {
        let ScalarType::CheckedReference { inner, .. } = &outputs.outputs[0].ty else {
            return Err("fresh return output lacks checked-reference type".to_owned());
        };
        return Ok(EmitValue::Basic(emit_automatic_return_result(
            context,
            state,
            take_basic(value)?,
            inner,
        )?));
    }
    let EmitValue::Aggregate { value, .. } = value else {
        return Err("multi-output function requires an aggregate value".to_owned());
    };
    let mut values = Vec::new();
    for (index, output) in outputs.outputs.iter().enumerate() {
        let extracted = state
            .builder
            .build_extract_value(value, index as u32, "return_output")
            .map_err(builder_error)?;
        let mut extracted = extracted.into();
        if state.fresh_return_outputs.contains(&index) {
            let ScalarType::CheckedReference { inner, .. } = &output.ty else {
                return Err(format!(
                    "fresh return output {index} lacks checked-reference type"
                ));
            };
            extracted = emit_automatic_return_result(context, state, extracted, inner)?;
        }
        values.push(EmitValue::Basic(extracted));
    }
    build_aggregate(context, state, outputs, values)
}

pub(super) fn materialize_return_branch<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let Some((indices, covered, joined)) = state
        .branch_return_outputs
        .get(&(block.span.start, block.span.end))
        .cloned()
    else {
        return Ok(value);
    };
    for output in covered {
        let owner = if let Some(name) = joined.get(&output) {
            let (slot, _) = state
                .joined_return_bindings
                .get(name)
                .ok_or_else(|| format!("missing joined return binding {name}"))?;
            state
                .builder
                .build_load(context.bool_type(), *slot, "joined_return_owned")
                .map_err(builder_error)?
                .into_int_value()
        } else {
            context
                .bool_type()
                .const_int(u64::from(indices.contains(&output)), false)
        };
        state.return_owner_flags.insert(output, owner);
    }
    if indices.is_empty() {
        return Ok(value);
    }
    let outputs = crate::ScalarOutputSequence {
        outputs: block
            .final_output_values
            .iter()
            .enumerate()
            .map(|(position, output)| crate::ScalarOutput {
                ty: state.return_output_types[if block.final_output_values.len() == 1 {
                    *indices.iter().next().expect("selected return output")
                } else {
                    position
                }]
                .clone(),
                span: output.span,
            })
            .collect(),
        span: block.span,
    };
    let previous = std::mem::replace(&mut state.fresh_return_outputs, indices);
    let result = materialize_fresh_return_outputs(context, state, value, &outputs);
    state.fresh_return_outputs = previous;
    result
}
