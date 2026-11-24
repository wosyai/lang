use super::*;
use crate::scalar::{mixed_owned_struct_result, mixed_owner_result_lanes, ArrayWriterSide, ClosedBody, ClosedCall, ClosedInitializer, ClosedInitializerInput, ClosedInput, ClosedLiteralArray, ClosedLocalArrayRelease, ClosedReturn, ConcreteCallSelection, FreshArrayExtent, ReplacementArrayExtent, ScalarBinding};
use inkwell::types::AnyType;

pub(super) struct InitializerEmitter<'ctx, 'plan> {
    plan: &'plan ClosedInitializer,
    receipt: Option<(usize, PointerValue<'ctx>)>,
    immediate_consumer: Option<usize>,
    visited: BTreeSet<usize>,
}

impl<'ctx, 'plan> InitializerEmitter<'ctx, 'plan> {
    pub(super) fn new(plan: &'plan ClosedInitializer) -> Self {
        Self { plan, receipt: None, immediate_consumer: None, visited: BTreeSet::new() }
    }

    fn visit(&mut self, ordinal: usize) -> Result<(), String> {
        if !self.visited.insert(ordinal) {
            return Err("duplicate closed initializer evaluation".into());
        }
        Ok(())
    }
}

pub(super) fn initializer_namespace_edge(state: &EmitState<'_, '_>) -> Result<(), String> {
    if state.initializer_aggregate.as_ref().is_some_and(|frame| frame.receipt.is_some()) {
        return Err("closed initializer receipt crosses namespace dispatch".into());
    }
    Ok(())
}

pub(super) fn finish_initializer(state: &mut EmitState<'_, '_>) -> Result<(), String> {
    if let Some(frame) = state.initializer_aggregate.take() {
        if frame.receipt.is_some() || frame.immediate_consumer.is_some()
            || frame.visited.len() != frame.plan.installs.len() + frame.plan.immediate.len() + frame.plan.inputs.len() {
            return Err("unfinished closed initializer transfer".into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum CallDestination<'ctx> {
    Body(ClosedCall),
    Initializer(PointerValue<'ctx>),
    InitializerTemporary,
}

#[derive(Clone, Copy)]
enum CallInput {
    Body(ClosedInput),
    Initializer(ClosedInitializerInput),
}


struct PendingLocalArrayRelease<'ctx> {
    route: ClosedLocalArrayRelease,
    base: PointerValue<'ctx>,
    live: PointerValue<'ctx>,
    owner: String,
}

enum LiteralArrayReceipt<'ctx> {
    Awaiting,
    Original {
        base: PointerValue<'ctx>,
        storage: Option<(PointerValue<'ctx>, PointerValue<'ctx>)>,
        argument_extent: Option<inkwell::values::IntValue<'ctx>>,
        in_writer: Option<ArrayWriterSide>,
    },
    Fresh {
        base: PointerValue<'ctx>,
        joined_storage: Option<PointerValue<'ctx>>,
    },
    Joined(PointerValue<'ctx>),
    Finished,
}

struct LiteralArrayBranchOriginal<'ctx> {
    base: PointerValue<'ctx>,
    storage: (PointerValue<'ctx>, PointerValue<'ctx>),
}

pub(super) struct AggregateEmitter<'ctx, 'plan> {
    plan: &'plan ClosedBody,
    destination: Option<PointerValue<'ctx>>,
    ordinary_destination: Option<PointerValue<'ctx>>,
    ordinary_committed: bool,
    ordinary_called: bool,
    literal_array_receipt: LiteralArrayReceipt<'ctx>,
    pub(super) fresh_recommit_extent: Option<(usize, inkwell::values::IntValue<'ctx>)>,
    literal_array_branch_original: Option<LiteralArrayBranchOriginal<'ctx>>,
    base: Option<PointerValue<'ctx>>,
    registered_allocation: Option<String>,
    mixed_return: Option<inkwell::values::StructValue<'ctx>>,
    mixed_branch_arms: u8,
    failure_site: Option<ClosedLocalArrayRelease>,
    failure_allocation: Option<PendingLocalArrayRelease<'ctx>>,
    failure_arm_visited: bool,
    committed: bool,
    scalar_descriptor_enrolled: bool,
    checked_backing_output: Option<PointerValue<'ctx>>,
    checked_backing_base: Option<(PointerValue<'ctx>, String)>,
    checked_backing_arms: u8,
    checked_backing_pending: Option<PointerValue<'ctx>>,
    checked_backing_receiver: Option<PointerValue<'ctx>>,
    checked_backing_released: bool,
    checked_backing_forwarded: bool,
    receipts: BTreeMap<usize, PointerValue<'ctx>>,
    // SSA join values indexed by the function-local, shared guard plan.
    receipt_guards: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    input: Option<(usize, PointerValue<'ctx>)>,
}

fn literal_array_write(
    plan: Option<ClosedLiteralArray>,
    ordinal: usize,
) -> Option<(usize, ReplacementArrayExtent, Option<ArrayWriterSide>)> {
    let ClosedLiteralArray::Replacement { call, destination, extent, unit, .. } = plan? else {
        return None;
    };
    if call == ordinal {
        return Some((destination, extent, unit.map(|unit| unit.writer_side)));
    }
    let other = unit?.other_writer.filter(|other| other.call == ordinal)?;
    Some((other.destination, other.extent, Some(ArrayWriterSide::Else)))
}

impl<'ctx, 'plan> AggregateEmitter<'ctx, 'plan> {
    pub(super) fn literal_array_replacement(&self, ordinal: usize) -> Option<usize> {
        literal_array_write(self.plan.literal_array, ordinal).map(|(destination, _, _)| destination)
    }

    pub(super) fn fresh_array_recommit(&self, ordinal: usize) -> Option<crate::scalar::ClosedFreshRecommit> {
        self.plan.fresh_array_recommits.get(&ordinal).copied()
    }

    pub(super) fn same_array_rebind_destination(&self, ordinal: usize) -> Option<usize> {
        self.plan.same_array_rebinds.get(&ordinal).copied()
    }

    pub(super) fn new(
        plan: Option<&'plan ClosedBody>,
        definition: FunctionValue<'ctx>,
        parameter_count: usize,
    ) -> Result<Option<Self>, String> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        let destination = if plan.returning.is_some() {
            Some(
                definition
                    .get_nth_param(parameter_count as u32)
                    .ok_or_else(|| "missing closed aggregate destination".to_owned())?
                    .into_pointer_value(),
            )
        } else {
            None
        };
        if (plan.ordinary_result || !plan.ordinary_calls.is_empty())
            && (plan.checked_backing_producer.is_some() || plan.checked_backing_consumer.is_some()
                || plan.checked_backing_wrapper.is_some())
        {
            return Err("overlapping ordinary and checked aggregate conventions".into());
        }
        let ordinary_destination = if plan.ordinary_result {
            if destination.is_some() || plan.input.is_some() || definition.count_params() as usize != parameter_count + 1 {
                return Err("overlapping ordinary aggregate destination".into());
            }
            Some(
                definition.get_nth_param(parameter_count as u32)
                    .ok_or_else(|| "missing ordinary aggregate destination".to_owned())?
                    .into_pointer_value(),
            )
        } else {
            None
        };
        let input = plan.input.map(|slot| definition.get_nth_param(definition.count_params() - 1)
            .ok_or_else(|| "missing aggregate formal input base".to_owned())
            .map(|base| (slot, base.into_pointer_value()))).transpose()?;
        let checked_backing_output = (plan.checked_backing_producer.is_some() || plan.checked_backing_wrapper.is_some())
            .then(|| definition.get_nth_param(definition.count_params() - 1)
            .ok_or_else(|| "missing private checked backing output".to_owned())
            .map(|value| value.into_pointer_value())).transpose()?;
        let failure_site = plan.local_array_failure.filter(|_| {
            plan.returning.is_none() && plan.input.is_none() && plan.calls.is_empty()
                && plan.releases.is_empty() && plan.input_calls.is_empty()
        });
        if let Some(failure) = plan.checked_backing_producer.filter(|backing| backing.failure.is_some()) {
            if failure_site != failure.failure.map(|arm| ClosedLocalArrayRelease { receipt: failure.receipt, arm }) {
                return Err("mixed checked backing producer has no matching local release".into());
            }
        }
        Ok(Some(Self {
            plan,
            input,
            destination,
            ordinary_destination,
            ordinary_committed: false,
            ordinary_called: false,
            literal_array_receipt: LiteralArrayReceipt::Awaiting,
            fresh_recommit_extent: None,
            literal_array_branch_original: None,
            base: None,
            registered_allocation: None,
            mixed_return: None,
            mixed_branch_arms: 0,
            failure_site,
            failure_allocation: None,
            failure_arm_visited: false,
            committed: false,
            scalar_descriptor_enrolled: false,
            checked_backing_output,
            checked_backing_base: None,
            checked_backing_arms: 0,
            checked_backing_pending: None,
            checked_backing_receiver: None,
            checked_backing_released: false,
            checked_backing_forwarded: false,
            receipts: BTreeMap::new(),
            receipt_guards: BTreeMap::new(),
        }))
    }

    fn set_base(&mut self, base: PointerValue<'ctx>) -> Result<(), String> {
        if self.base.replace(base).is_some() {
            return Err("duplicate closed aggregate original base".into());
        }
        Ok(())
    }
}

pub(super) fn linkage(plan: Option<&ClosedBody>) -> Option<Linkage> {
    plan.filter(|body| body.returning.is_some() || body.input.is_some() || body.checked_backing_producer.is_some()
        || body.checked_backing_wrapper.is_some() || body.ordinary_result)
        .map(|_| Linkage::Internal)
}

pub(super) fn definition_type<'ctx>(
    context: &'ctx Context,
    function: &ScalarFunction,
    cfg: &ScalarReferenceCfg,
    layout: ScalarTargetLayout,
    plan: Option<&ClosedBody>,
) -> Result<FunctionType<'ctx>, String> {
    if plan.is_some_and(|body| body.ordinary_result) {
        let ScalarType::Callable { parameters, outputs } = &function.signature else {
            return Err("invalid ordinary aggregate declaration".into());
        };
        if !matches!(outputs.outputs.as_slice(), [crate::ScalarOutput { ty: ScalarType::Struct(_), .. }])
            || !dynamic_return_outputs(function.span, cfg).is_empty()
        {
            return Err("invalid ordinary aggregate result signature".into());
        }
        let original = definition_function_type(context, function, cfg, layout)?;
        if original.count_param_types() as usize != parameters.len() {
            return Err("unexpected ordinary aggregate return sidecar".into());
        }
        let mut arguments = original.get_param_types().into_iter().map(Into::into)
            .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
        let pointer = context.ptr_type(AddressSpace::default());
        arguments.push(pointer.into());
        return Ok(context.void_type().fn_type(&arguments, false));
    }
    if plan.and_then(|body| body.returning).is_none() {
        let original = definition_function_type(context, function, cfg, layout)?;
        if !plan.is_some_and(|body| body.input.is_some() || body.checked_backing_producer.is_some()
            || body.checked_backing_wrapper.is_some()) {
            return Ok(original);
        }
        let mut parameters = original.get_param_types().into_iter().map(Into::into)
            .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
        parameters.push(context.ptr_type(AddressSpace::default()).into());
        return Ok(match original.get_return_type() {
            Some(output) => output.fn_type(&parameters, false),
            None => context.void_type().fn_type(&parameters, false),
        });
    }
    let ScalarType::Callable {
        parameters,
        outputs,
    } = &function.signature
    else {
        return Err("invalid closed aggregate declaration".into());
    };
    let mixed = mixed_owned_struct_result(outputs);
    if mixed.is_none() && !matches!(outputs.outputs.as_slice(),
        [crate::ScalarOutput { ty: ScalarType::Struct(_), .. }])
    {
        return Err("invalid closed aggregate return layout".into());
    }
    let pointer = context.ptr_type(AddressSpace::default());
    let mut arguments = parameters
        .iter()
        .map(|ty| basic_type(context, ty, layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum<'ctx>>, _>>()?;
    arguments.push(pointer.into());
    Ok(match mixed {
        Some(lanes) => {
            let mut transport = [pointer.into(); 2];
            transport[lanes.scalar] = basic_type(context, &outputs.outputs[lanes.scalar].ty, layout)?;
            context.struct_type(&transport, false).fn_type(&arguments, false)
        }
        None => pointer.fn_type(&arguments, false),
    })
}

pub(super) fn has_literal_array_plan(state: &EmitState<'_, '_>) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| aggregate.plan.literal_array.is_some())
}

fn literal_array_u64_formal<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let value = builder.get_insert_block().and_then(|block| block.get_parent())
        .and_then(|function| function.get_nth_param(0))
        .ok_or_else(|| "missing checked array extent formal".to_owned())?;
    let BasicValueEnum::IntValue(value) = value else {
        return Err("checked array extent formal is not an integer".into());
    };
    if value.get_type().get_bit_width() != 64 {
        return Err("checked array extent formal is not U64".into());
    }
    Ok(value)
}

pub(super) fn checked_array_formal_extent<'ctx>(
    state: &EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    expression: &ScalarExpression,
) -> Result<Option<inkwell::values::IntValue<'ctx>>, String> {
    let Some(ClosedLiteralArray::FreshLeaf { receipt, length_occurrence, extent: FreshArrayExtent::Formal, .. }) =
        state.aggregate.as_ref().and_then(|aggregate| aggregate.plan.literal_array)
    else { return Ok(None) };
    if !state.body_call_nodes.is_some_and(|nodes|
        nodes.binding_position(binding) == Some(receipt.allocation)
            && nodes.expression_ordinal(expression) == Some(length_occurrence))
    {
        return Err("checked array extent formal does not match its allocation".into());
    }
    // The typed route proves an immutable direct read of formal zero.
    Ok(Some(literal_array_u64_formal(&state.builder)?))
}

pub(super) fn enroll_literal_array_allocation<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    base: PointerValue<'ctx>,
    length: inkwell::values::IntValue<'ctx>,
    storage: Option<(PointerValue<'ctx>, PointerValue<'ctx>)>,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else { return Err("missing checked literal array allocation plan".into()) };
    let Some(plan) = aggregate.plan.literal_array else { return Err("missing checked literal array allocation".into()) };
    let (receipt, occurrence, extent) = match plan {
        ClosedLiteralArray::FreshLeaf { receipt, length_occurrence, extent, .. } =>
            (receipt, length_occurrence, extent),
        ClosedLiteralArray::Replacement { original, original_length_occurrence, original_elements, .. } =>
            (original, original_length_occurrence, FreshArrayExtent::Literal(original_elements)),
    };
    let extent_matches = match extent {
        FreshArrayExtent::Literal(elements) => length.get_zero_extended_constant() == Some(elements),
        FreshArrayExtent::Formal => length == literal_array_u64_formal(&state.builder)?,
    };
    let actual_extent = binding.receivers.first().and_then(|receiver| receiver.allocation_length.as_ref())
        .and_then(|expression| state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
    if state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding)) != Some(receipt.allocation)
        || actual_extent != Some(occurrence)
        || !extent_matches
        || storage.is_some() != matches!(plan, ClosedLiteralArray::Replacement { .. })
        || !matches!(aggregate.literal_array_receipt, LiteralArrayReceipt::Awaiting)
    {
        return Err("checked literal array allocation does not match its receipt".into());
    }
    aggregate.literal_array_receipt = LiteralArrayReceipt::Original { base, storage, argument_extent: None, in_writer: None };
    Ok(())
}

pub(super) fn literal_array_leaf_return(state: &EmitState<'_, '_>) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate|
        matches!(aggregate.plan.literal_array, Some(ClosedLiteralArray::FreshLeaf { .. })))
}

fn release_literal_array_base<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    base: PointerValue<'ctx>,
) -> Result<(), String> {
    super::ownership::consume_runtime_array_release(context, state,
        super::ownership::RuntimeArrayRelease { pointer: base.into(), live: None })
}

pub(super) fn finish_conditional_receipts<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    then_end: inkwell::basic_block::BasicBlock<'ctx>,
    else_end: inkwell::basic_block::BasicBlock<'ctx>,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else { return Ok(()) };
    if aggregate.plan.conditional_inputs.is_empty() {
        return Ok(());
    }
    let expression = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
        .ok_or_else(|| "missing conditional aggregate receipt occurrence".to_owned())?;
    let Some(plan) = aggregate.plan.conditional_inputs.get(&expression) else { return Ok(()) };
    for &(guard, arms) in &plan.guards {
        let incoming = |gate| match gate {
            crate::scalar::ReceiptGate::Retained => Ok(context.bool_type().const_int(1, false)),
            crate::scalar::ReceiptGate::Transferred => Ok(context.bool_type().const_zero()),
            crate::scalar::ReceiptGate::Guard(child) => aggregate.receipt_guards.get(&child).copied()
                .ok_or_else(|| "missing nested aggregate receipt guard".to_owned()),
        };
        let [then_live, else_live] = [incoming(arms[0])?, incoming(arms[1])?];
        let phi = state.builder.build_phi(context.bool_type(), "aggregate_receipt_active")
            .map_err(builder_error)?;
        phi.add_incoming(&[(&else_live, else_end), (&then_live, then_end)]);
        if aggregate.receipt_guards.insert(guard, phi.as_basic_value().into_int_value()).is_some() {
            return Err("duplicate conditional aggregate receipt guard".into());
        }
    }
    for receipt in &plan.consumed_on_both_arms {
        aggregate.receipts.remove(receipt)
            .ok_or_else(|| "missing exhaustive aggregate receipt at conditional merge".to_owned())?;
    }
    Ok(())
}

pub(super) fn begin_literal_array_unit(
    state: &mut EmitState<'_, '_>,
    expression: &ScalarExpression,
    then_block: &ScalarBlock,
    else_block: Option<&ScalarBlock>,
    emitting: ArrayWriterSide,
) -> Result<bool, String> {
    let Some(aggregate) = state.aggregate.as_mut() else { return Ok(false) };
    let Some(ClosedLiteralArray::Replacement { unit: Some(unit), .. }) = aggregate.plan.literal_array
    else { return Ok(false) };
    let Some(nodes) = state.body_call_nodes else { return Err("missing checked array branch context".into()) };
    let (writer, retained) = unit.writer_side.split(Some(then_block), else_block);
    let dual = unit.other_writer.is_some();
    if nodes.expression_ordinal(expression) != Some(unit.expression)
        || writer.and_then(|block| nodes.block_ordinal(block)) != Some(unit.writer_block)
        || !writer.is_some_and(|block| block.items.len() == 1)
        || retained.is_some() != unit.retained_block.is_some()
        || retained.and_then(|block| nodes.block_ordinal(block)) != unit.retained_block
        || retained.is_some_and(|block| block.items.len() != if dual { 1 } else { 0 })
        || (dual && (unit.writer_side != ArrayWriterSide::Then || retained.is_none()))
    {
        return Err("checked array branch does not match its route".into());
    }
    if dual && emitting == ArrayWriterSide::Else {
        if !matches!(aggregate.literal_array_receipt,
            LiteralArrayReceipt::Fresh { joined_storage: Some(_), .. })
        {
            return Err("first checked array writer did not commit before the alternate arm".into());
        }
        let LiteralArrayBranchOriginal { base, storage } = aggregate.literal_array_branch_original.take()
            .ok_or_else(|| "missing checked original allocation for the alternate arm".to_owned())?;
        // Restore only the compiler token for the other runtime edge. The
        // allocation and its private slots dominate both mutually exclusive arms.
        aggregate.literal_array_receipt = LiteralArrayReceipt::Original {
            base, storage: Some(storage), argument_extent: None, in_writer: None,
        };
    } else if emitting != unit.writer_side {
        // The compiler visits then before else; this is emission state only.
        // The verified empty arm never loads or changes the runtime receipt.
        let matches_order = match unit.writer_side {
            ArrayWriterSide::Then => matches!(aggregate.literal_array_receipt,
                LiteralArrayReceipt::Fresh { joined_storage: Some(_), .. }),
            ArrayWriterSide::Else => matches!(aggregate.literal_array_receipt,
                LiteralArrayReceipt::Original {
                    storage: Some(_), argument_extent: None, in_writer: None, ..
                }),
        };
        if !matches_order {
            return Err("checked retained array arm has an unexpected receipt phase".into());
        }
        return Ok(false);
    }
    let LiteralArrayReceipt::Original { base, storage: Some(storage), argument_extent: None, in_writer } =
        &mut aggregate.literal_array_receipt
    else { return Err("checked array branch has no original receipt".into()) };
    if std::mem::replace(in_writer, Some(emitting)).is_some() {
        return Err("duplicate checked array writer arm".into());
    }
    if dual && emitting == ArrayWriterSide::Then
        && aggregate.literal_array_branch_original.replace(LiteralArrayBranchOriginal {
            base: *base, storage: *storage,
        }).is_some()
    {
        return Err("duplicate checked alternate original receipt".into());
    }
    Ok(true)
}

pub(super) fn finish_literal_array_unit(
    state: &mut EmitState<'_, '_>,
    enrolled: bool,
) -> Result<(), String> {
    if !enrolled {
        return Ok(());
    }
    let Some(aggregate) = state.aggregate.as_mut() else { return Err("missing checked array join plan".into()) };
    if aggregate.literal_array_branch_original.is_some() {
        return Err("checked array conditional did not visit the alternate writer".into());
    }
    let LiteralArrayReceipt::Fresh { joined_storage: Some(storage), .. } =
        std::mem::replace(&mut aggregate.literal_array_receipt, LiteralArrayReceipt::Finished)
    else { return Err("checked array branch did not install a fresh receipt".into()) };
    // Each checked writer updated the private slot on its own edge. A retained
    // edge leaves it untouched. Both edges were proved to own exactly one receipt.
    aggregate.literal_array_receipt = LiteralArrayReceipt::Joined(storage);
    Ok(())
}

pub(super) fn capture_checked_array_argument_extent<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    values: &[EmitValue<'ctx>],
) -> Result<(), String> {
    let Some(aggregate) = state.aggregate.as_mut() else { return Ok(()) };
    if !aggregate.plan.fresh_array_recommits.is_empty() {
        let nodes = state.body_call_nodes.ok_or("missing fresh array recommit context")?;
        if let Some(call) = nodes.expression_ordinal(expression) {
            if let Some(recommit) = aggregate.fresh_array_recommit(call) {
                if let (crate::scalar::RecommittedArrayExtent::Formal(slot), Some(occurrence)) =
                    (recommit.size, recommit.argument)
                {
                    let argument = arguments.get(slot).ok_or("missing fresh array extent argument")?;
                    let Some(EmitValue::Basic(BasicValueEnum::IntValue(extent))) = values.get(slot)
                    else { return Err("fresh array extent is not an integer".into()) };
                    if nodes.expression_ordinal(argument) != Some(occurrence)
                        || extent.get_type().get_bit_width() != 64
                        || aggregate.fresh_recommit_extent.replace((call, *extent)).is_some()
                    {
                        return Err("fresh array extent differs from prepared argument".into());
                    }
                }
            }
        }
    }
    let Some(ClosedLiteralArray::Replacement { .. }) = aggregate.plan.literal_array else { return Ok(()) };
    let Some(nodes) = state.body_call_nodes else { return Err("missing checked extent call context".into()) };
    let Some(call) = nodes.expression_ordinal(expression) else { return Ok(()) };
    let Some((_, ReplacementArrayExtent::Argument { occurrence }, expected_arm)) =
        literal_array_write(aggregate.plan.literal_array, call) else { return Ok(()) };
    let ([argument], [EmitValue::Basic(BasicValueEnum::IntValue(extent))]) = (arguments, values)
        else { return Err("checked array extent requires its one scalar argument".into()) };
    let LiteralArrayReceipt::Original { storage: Some(_), argument_extent, in_writer, .. } =
        &mut aggregate.literal_array_receipt
    else { return Err("checked array extent has no original receipt".into()) };
    if nodes.expression_ordinal(argument) != Some(occurrence) || extent.get_type().get_bit_width() != 64
        || argument_extent.is_some() || *in_writer != expected_arm
    {
        return Err("checked array extent argument does not match its occurrence".into());
    }
    *argument_extent = Some(*extent);
    Ok(())
}

pub(super) fn install_literal_array_replacement<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    call: usize,
    value: &EmitValue<'ctx>,
) -> Result<(), String> {
    let Some(aggregate) = state.aggregate.as_mut() else { return Err("missing checked literal array plan".into()) };
    let Some(ClosedLiteralArray::Replacement { unit, .. }) = aggregate.plan.literal_array
        else { return Err("missing checked literal array replacement".into()) };
    let Some((_, extent, expected_arm)) = literal_array_write(aggregate.plan.literal_array, call)
        else { return Err("checked array writer does not match its call".into()) };
    let EmitValue::Basic(BasicValueEnum::PointerValue(fresh)) = value else {
        return Err("checked literal array replacement has no backing".into());
    };
    let LiteralArrayReceipt::Original {
        base: original, storage: Some((destination, length_slot)), argument_extent, in_writer,
    } =
        std::mem::replace(&mut aggregate.literal_array_receipt, LiteralArrayReceipt::Finished)
    else { return Err("checked literal array replacement has no original receipt".into()) };
    if in_writer != expected_arm {
        return Err("checked array writer does not match its branch and call".into());
    }
    let length = match (extent, argument_extent) {
        (ReplacementArrayExtent::Literal(elements), None) => context.i64_type().const_int(elements, false),
        (ReplacementArrayExtent::Argument { .. }, Some(length)) => length,
        _ => return Err("checked array replacement has no matching extent".into()),
    };
    release_literal_array_base(context, state, original)?;
    state.builder.build_store(destination, *fresh).map_err(builder_error)?;
    state.builder.build_store(length_slot, length).map_err(builder_error)?;
    state.aggregate.as_mut().expect("checked literal array replacement").literal_array_receipt =
        LiteralArrayReceipt::Fresh { base: *fresh, joined_storage: unit.map(|_| destination) };
    Ok(())
}

pub(super) fn record_allocation<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    name: &str,
    base: PointerValue<'ctx>,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(());
    };
    if aggregate.plan.checked_backing_producer.is_some_and(|plan| {
        state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding)) == Some(plan.receipt.allocation)
    }) && aggregate.checked_backing_base.replace((base, name.to_owned())).is_some() {
        return Err("duplicate private checked backing allocation".into());
    }
    let binding_position = || state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding));
    let registered = match aggregate.plan.returning {
        Some(ClosedReturn::Allocation { block, item }) => binding_position() == Some((block, item)),
        Some(ClosedReturn::MixedBranch) => binding_position().is_some_and(|(block, item)| {
            aggregate.plan.mixed_branch_arm(block).is_some_and(|(_, expected)| item == expected)
        }),
        _ => false,
    };
    if !registered {
        return Ok(());
    }
    aggregate.set_base(base)?;
    aggregate.registered_allocation = Some(name.to_owned());
    Ok(())
}

pub(super) fn record_failure_allocation<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    name: &str,
    base: PointerValue<'ctx>,
    live: PointerValue<'ctx>,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(());
    };
    let Some(route) = aggregate.failure_site else {
        return Ok(());
    };
    if state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding))
        == Some(route.receipt.allocation)
    {
        if aggregate.failure_allocation.is_some() || aggregate.failure_arm_visited {
            return Err("duplicate local array failure allocation".into());
        }
        aggregate.failure_allocation = Some(PendingLocalArrayRelease {
            route,
            base,
            live,
            owner: name.to_owned(),
        });
    }
    Ok(())
}

pub(super) fn release_failure_allocation<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    block: &crate::scalar::ScalarBlock,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(());
    };
    let Some(route) = aggregate.failure_site else {
        return Ok(());
    };
    let ordinal = state.body_call_nodes.and_then(|nodes| nodes.block_ordinal(block))
        .ok_or_else(|| "missing bound block for local array failure".to_owned())?;
    if ordinal != route.arm {
        return Ok(());
    }
    if std::mem::replace(&mut aggregate.failure_arm_visited, true) {
        return Err("duplicate local array failure arm".into());
    }
    let PendingLocalArrayRelease { route: recorded, base, live, owner } =
        aggregate.failure_allocation.take()
            .ok_or_else(|| "missing original local array failure allocation".to_owned())?;
    if recorded != route {
        return Err("planned local array release does not match the recorded receipt".into());
    }
    if !state.automatic_return_result_owners.is_empty() || !state.joined_return_bindings.is_empty()
        || state.runtime_array_owners.keys().any(|name| name != &owner)
    {
        if aggregate.plan.checked_backing_producer.is_some_and(|plan| plan.failure == Some(route.arm)) {
            return Err("mixed checked backing release overlaps another owner".into());
        }
        return Ok(());
    }
    super::ownership::release_proven_local_array(context, state, base, live)
}

pub(super) fn enroll_scalar_descriptor<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    expression: &ScalarExpression,
    position: usize,
    ty: &ScalarType,
    flag: Option<inkwell::values::IntValue<'ctx>>,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else { return Ok(()) };
    let backed = aggregate.plan.checked_backing_consumer;
    let Some((destination, call, output)) = aggregate.plan.scalar_descriptor_assignment
        .or_else(|| backed.map(|(destination, call)| (destination, call, 0))) else { return Ok(()) };
    let nodes = state.body_call_nodes.ok_or_else(|| "missing bound descriptor assignment".to_owned())?;
    if nodes.place_ordinal(&target.place) != Some(destination) {
        return Ok(());
    }
    let (crate::ScalarPlace::Name { name, .. }, ScalarExpression::Call { span, .. },
        ScalarType::CheckedReference { inner, .. }) = (&target.place, expression, ty) else {
        return Err("descriptor assignment changed its validated shape".into());
    };
    if position != output || nodes.expression_ordinal(expression) != Some(call)
        || std::mem::replace(&mut aggregate.scalar_descriptor_enrolled, true)
    {
        return Err("invalid descriptor assignment commit".into());
    }
    if state.joined_return_bindings.contains_key(name)
        || state.automatic_return_result_owners.contains_key(name)
    {
        return Err("descriptor assignment overlaps another owner".into());
    }
    match state.checked_call_owners.get(&(span.start, span.end, output)) {
        Some(CheckedCallOwner::Fresh) => {}
        Some(CheckedCallOwner::Mixed) if flag.is_some() => {}
        _ => return Err("missing validated checked descriptor owner".into()),
    }
    if let Some(flag) = flag {
        state.checked_owner_flags.insert(name.clone(), flag);
    }
    state.automatic_return_result_owners.insert(name.clone(), ScalarAutomaticReturnResult {
        aggregate_type: inner.as_ref().clone(),
        allocation_identity: ScalarAllocationIdentity { binding: name.clone() },
        state: ScalarAutomaticReturnResultState::Live,
    });
    if backed.is_some() {
        let base = aggregate.checked_backing_pending.take().ok_or_else(|| "missing private checked backing receipt".to_owned())?;
        if aggregate.checked_backing_receiver.replace(base).is_some() {
            return Err("duplicate private checked backing receipt".into());
        }
    }
    Ok(())
}

fn call_action(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
) -> Option<(usize, ClosedCall)> {
    let aggregate = state.aggregate.as_ref()?;
    let ordinal = state.body_call_nodes?.expression_ordinal(expression)?;
    aggregate
        .plan
        .calls
        .get(&ordinal)
        .copied()
        .map(|action| (ordinal, action))
}

pub(super) fn store_received_binding<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    match call_action(state, &binding.value) {
        Some((_, ClosedCall::Forward)) if binding.receivers.len() == 2 => {
            if !forwarded_mixed_return(state) || !matches!(value, EmitValue::Unit) {
                return Err("mixed aggregate forwarding binding lacks its original result".into());
            }
            return Ok(true);
        }
        Some((_, ClosedCall::Receive)) => {}
        _ => return Ok(false),
    }
    if let [head, tail] = binding.receivers.as_slice() {
        let lanes = mixed_owner_result_lanes(&head.ty, &tail.ty)
            .ok_or_else(|| "invalid mixed aggregate result binding".to_owned())?;
        let receiver = &binding.receivers[lanes.owner];
        let scalar = &binding.receivers[lanes.scalar];
        let owner = extract_output(state, value, lanes.owner)?;
        let EmitValue::Basic(BasicValueEnum::PointerValue(destination)) = owner else {
            return Err("mixed aggregate receiver did not receive a destination".into());
        };
        let value = extract_output(state, value, lanes.scalar)?;
        let slot = entry_storage_alloca(context, state, &scalar.ty, &scalar.name)?;
        store_value(context, state, slot, &scalar.ty, value)?;
        state.storage.insert(scalar.name.clone(), (slot, scalar.ty.clone()));
        state.storage.insert(receiver.name.clone(), (destination, receiver.ty.clone()));
        return Ok(true);
    }
    let [receiver] = binding.receivers.as_slice() else {
        return Err("closed aggregate receiver has ambiguous outputs".into());
    };
    let EmitValue::Basic(BasicValueEnum::PointerValue(destination)) = value else {
        return Err("closed aggregate receiver did not receive a destination".into());
    };
    state
        .storage
        .insert(receiver.name.clone(), (*destination, receiver.ty.clone()));
    Ok(true)
}

pub(super) fn emit_initializer_binding<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &ScalarBinding,
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<bool, String> {
    let Some(frame) = state.initializer_aggregate.as_ref() else { return Ok(false) };
    let Some(&producer) = frame.plan.installs.get(&binding.name_span) else { return Ok(false) };
    let ScalarExpression::Call { arguments, .. } = &binding.value else {
        return Err("missing closed initializer producer expression".into());
    };
    let nodes = state.initializer_nodes.ok_or_else(|| "missing closed initializer nodes".to_owned())?;
    let (bound, target) = nodes.bound_call(&binding.value)?;
    if bound != producer || frame.receipt.is_some() {
        return Err("closed initializer installation differs from its bound producer".into());
    }
    let (global, ty) = state.globals.get(&binding.name)
        .ok_or_else(|| "missing closed initializer global".to_owned())?;
    if *ty != binding.declared_type
        || global.get_value_type() != storage_type(context, ty, state.layouts, state.target_layout)?.as_any_type_enum()
    {
        return Err("closed initializer global layout mismatch".into());
    }
    let global = global.as_pointer_value();
    emit_selected_call(context, state, &binding.value, arguments, project, producer,
        Some(CallDestination::Initializer(global)), None, Some(target))?;
    Ok(true)
}

pub(super) fn emit_identity_relay<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    definition: FunctionValue<'ctx>,
) -> Result<(), String> {
    let parameter = definition.get_first_param()
        .ok_or_else(|| "missing same-owner relay parameter".to_owned())?;
    if definition.count_params() != 1 || !matches!(parameter, BasicValueEnum::PointerValue(_))
        || !matches!(definition.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::PointerType(_)))
    {
        return Err("same-owner relay changed its ordinary descriptor ABI".into());
    }
    builder.build_return(Some(&parameter)).map_err(builder_error)?;
    Ok(())
}

pub(super) fn emit_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    if let Some(frame) = state.initializer_aggregate.as_ref() {
        let nodes = state.initializer_nodes.ok_or_else(|| "missing closed initializer nodes".to_owned())?;
        let (ordinal, target) = nodes.bound_call(expression)?;
        if let Some(&consumer) = frame.plan.immediate.get(&ordinal) {
            if frame.immediate_consumer != Some(consumer) || frame.receipt.is_some() {
                return Err("closed initializer immediate producer outside its paired argument".into());
            }
            return emit_selected_call(context, state, expression, arguments, project, ordinal,
                Some(CallDestination::InitializerTemporary), None, Some(target)).map(Some);
        }
        let Some(&input) = frame.plan.inputs.get(&ordinal) else {
            if frame.receipt.is_some() {
                return Err("closed initializer receipt crosses an unplanned call".into());
            }
            return Ok(None);
        };
        return emit_selected_call(context, state, expression, arguments, project, ordinal,
            None, Some(CallInput::Initializer(input)), Some(target)).map(Some);
    }
    let Some(aggregate) = &state.aggregate else { return Ok(None) };
    let Some(ordinal) = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)) else { return Ok(None) };
    let forwards = aggregate.plan.checked_backing_wrapper == Some(ordinal);
    if forwards || aggregate.plan.checked_backing_consumer.is_some_and(|(_, call)| ordinal == call) {
        return emit_checked_backing_call(context, state, expression, arguments, project, forwards).map(Some);
    }
    if let Some(declaration) = aggregate.plan.ordinary_calls.get(&ordinal) {
        let Some(nodes) = state.body_call_nodes else {
            return Err("missing ordinary aggregate call identity".into());
        };
        let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration } = nodes.target(expression)? else {
            return Err("ordinary aggregate call changed its selected target".into());
        };
        if &declaration.source != source || declaration.range != *declaration_span {
            return Err("ordinary aggregate call changed its declaration identity".into());
        }
        let aggregate = state.aggregate.as_mut().expect("ordinary aggregate call frame");
        if std::mem::replace(&mut aggregate.ordinary_called, true) {
            return Err("ordinary aggregate call occurrence emitted twice".into());
        }
        return emit_ordinary_result_call(context, state, expression, arguments, project).map(Some);
    }
    if let Some(&previous) = aggregate.plan.recommits.get(&ordinal) {
        // The identity relay uses its unchanged ordinary value signature. Only
        // the caller's compile-time receipt key changes after the call.
        let symbol = state.direct_aggregate_symbol(expression, None)?;
        let definition = state.functions.get(symbol).ok_or_else(|| "missing same-owner recommit definition".to_owned())?;
        if definition.count_params() as usize != arguments.len()
            || !matches!(definition.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::PointerType(_)))
        {
            return Err("same-owner recommit changed its ordinary value ABI".into());
        }
        let value = emit_selected_call(context, state, expression, arguments, project, ordinal, None, None, None)?;
        if !matches!(value, EmitValue::Basic(_)) {
            return Err("same-owner recommit has no whole ordinary value".into());
        }
        let aggregate = state.aggregate.as_mut().expect("same-owner recommit context");
        let base = aggregate.receipts.remove(&previous)
            .ok_or_else(|| "same-owner recommit lost its original backing".to_owned())?;
        if aggregate.receipts.insert(ordinal, base).is_some() {
            return Err("duplicate same-owner recommit receipt".into());
        }
        return Ok(Some(value));
    }
    let action = aggregate.plan.calls.get(&ordinal).copied().map(CallDestination::Body);
    let input = aggregate.plan.input_calls.get(&ordinal).map(|&(_, input)| CallInput::Body(input));
    if action.is_none() && input.is_none() {
        return Ok(None);
    }
    emit_selected_call(context, state, expression, arguments, project, ordinal, action, input, None).map(Some)
}

fn emit_checked_backing_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
    forwards: bool,
) -> Result<EmitValue<'ctx>, String> {
    let symbol = state.direct_aggregate_symbol(expression, None)?;
    let function = *state.functions.get(symbol).ok_or_else(|| "missing checked backing definition".to_owned())?;
    let Some(ScalarType::Callable { parameters, outputs }) = state.signatures.get(symbol) else {
        return Err("missing checked backing signature".into());
    };
    let flags = state.dynamic_owner_outputs.get(symbol).ok_or_else(|| "missing checked backing owner outputs".to_owned())?;
    if arguments.len() != parameters.len() || function.count_params() as usize != arguments.len() + flags.len() + 1 {
        return Err("invalid private checked backing call arity".into());
    }
    let mut operands = Vec::new();
    for (argument, parameter) in arguments.iter().zip(parameters) {
        let value = match project {
            Some((module, modules)) => emit_project_typed_expression(context, state, argument, parameter, module, modules)?,
            None => emit_typed_expression(context, state, argument, parameter)?,
        };
        operands.push(BasicMetadataValueEnum::from(take_basic(value)?));
    }
    let owner_slots = super::expression::stage_checked_return_sidecars(context, state, flags, &mut operands)?;
    let pointer = context.ptr_type(AddressSpace::default());
    let slot = if forwards {
        state.aggregate.as_ref().and_then(|aggregate| aggregate.checked_backing_output)
            .ok_or_else(|| "missing private checked backing wrapper output".to_owned())?
    } else {
        entry_alloca(state, pointer.into(), "checked_backing_out")?
    };
    operands.push(slot.into());
    let value = state.builder.build_call(function, &operands, "checked_backing_call").map_err(builder_error)?;
    super::expression::publish_checked_return_sidecars(context, state, owner_slots)?;
    let aggregate = state.aggregate.as_mut().expect("closed checked backing call context");
    if forwards {
        if std::mem::replace(&mut aggregate.checked_backing_forwarded, true) {
            return Err("duplicate private checked backing forwarding".into());
        }
    } else {
        let base = state.builder.build_load(pointer, slot, "checked_backing_base").map_err(builder_error)?.into_pointer_value();
        if aggregate.checked_backing_pending.replace(base).is_some() {
            return Err("overlapping private checked backing receipts".into());
        }
    }
    super::expression::ordinary_call_result(value.try_as_basic_value(), outputs)
}

pub(super) fn publish_checked_backing_arm<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    block: &ScalarBlock,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else { return Ok(()) };
    let Some(plan) = aggregate.plan.checked_backing_producer else { return Ok(()) };
    let Some(ordinal) = state.body_call_nodes.and_then(|nodes| nodes.block_ordinal(block)) else {
        return Err("missing checked backing arm ordinal".into());
    };
    let Some(position) = plan.arms.iter().position(|&arm| arm == ordinal) else { return Ok(()) };
    let bit = 1u8 << position;
    if aggregate.checked_backing_arms & bit != 0 {
        return Err("duplicate private checked backing arm".into());
    }
    let (base, name) = aggregate.checked_backing_base.as_ref()
        .ok_or_else(|| "missing proved original checked backing".to_owned())?;
    let output = aggregate.checked_backing_output.ok_or_else(|| "missing proved checked backing output".to_owned())?;
    let published = if plan.failure == Some(ordinal) {
        if !aggregate.failure_arm_visited || aggregate.failure_allocation.is_some() {
            return Err("mixed checked backing arm did not consume the original allocation".into());
        }
        base.get_type().const_null()
    } else {
        *base
    };
    state.builder.build_store(output, published).map_err(builder_error)?;
    aggregate.checked_backing_arms |= bit;
    state.runtime_array_owners.insert(name.clone(), false);
    Ok(())
}

pub(super) fn release_checked_backing<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else { return Ok(()) };
    if aggregate.plan.checked_backing_consumer.is_none() {
        return Ok(());
    }
    let base = aggregate.checked_backing_receiver.take().ok_or_else(|| "missing checked backing at lexical exit".to_owned())?;
    declare_core_runtime(context, state.module);
    let optional = aggregate.plan.checked_backing_optional;
    aggregate.checked_backing_released = true;
    if optional {
        let live = state.builder.build_is_not_null(base, "checked_backing_present").map_err(builder_error)?;
        super::ownership::consume_runtime_array_release(context, state, super::ownership::RuntimeArrayRelease {
            pointer: base.into(),
            live: Some(live),
        })?;
    } else {
        let free = state.module.get_function(CORE_FREE_SYMBOL).ok_or_else(|| "missing checked backing runtime free".to_owned())?;
        state.builder.build_call(free, &[base.into()], "checked_backing_release").map_err(builder_error)?;
    }
    Ok(())
}

fn emit_ordinary_result_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<EmitValue<'ctx>, String> {
    let symbol = state.direct_aggregate_symbol(expression, None)?;
    let function = *state.functions.get(symbol)
        .ok_or_else(|| "missing ordinary aggregate producer".to_owned())?;
    let Some(ScalarType::Callable { parameters, outputs }) = state.signatures.get(symbol) else {
        return Err("missing ordinary aggregate call signature".into());
    };
    let [output] = outputs.outputs.as_slice() else {
        return Err("invalid ordinary aggregate output sequence".into());
    };
    if !matches!(output.ty, ScalarType::Struct(_)) || parameters.len() != arguments.len()
        || function.count_params() as usize != parameters.len() + 1
        || function.get_type().get_return_type().is_some()
        || function.get_linkage() != Linkage::Internal
        || state.dynamic_owner_outputs.get(symbol).is_some_and(|owners| !owners.is_empty())
    {
        return Err("invalid ordinary aggregate destination call ABI".into());
    }
    let destination = entry_storage_alloca(context, state, &output.ty, "ordinary_result_destination")?;
    let mut operands = Vec::with_capacity(parameters.len() + 1);
    for (argument, parameter) in arguments.iter().zip(parameters) {
        let emitted = match project {
            Some((module, modules)) => emit_project_typed_expression(context, state, argument, parameter, module, modules)?,
            None => emit_typed_expression(context, state, argument, parameter)?,
        };
        operands.push(BasicMetadataValueEnum::from(take_basic(emitted)?));
    }
    operands.push(destination.into());
    state.builder.build_call(function, &operands, "").map_err(builder_error)?;
    Ok(EmitValue::Basic(destination.into()))
}

fn emit_selected_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
    ordinal: usize,
    action: Option<CallDestination<'ctx>>,
    input: Option<CallInput>,
    initializer_target: Option<&ModuleInitializationCallTarget>,
) -> Result<EmitValue<'ctx>, String> {
    let symbol = state.direct_aggregate_symbol(expression, initializer_target)?;
    let function = *state
        .functions
        .get(symbol)
        .ok_or_else(|| format!("missing closed aggregate definition {symbol}"))?;
    let ScalarType::Callable {
        parameters,
        outputs,
    } = state
        .signatures
        .get(symbol)
        .ok_or_else(|| format!("missing closed aggregate declaration {symbol}"))?
    else {
        return Err(format!("invalid closed aggregate declaration {symbol}"));
    };
    let empty_outputs = BTreeSet::new();
    let owner_outputs = if input.is_some() {
        state.dynamic_owner_outputs.get(symbol)
            .ok_or_else(|| format!("missing owner signature for {symbol}"))?
    } else {
        &empty_outputs
    };
    let mixed = matches!(action, Some(CallDestination::Body(_)))
        .then(|| mixed_owned_struct_result(outputs)).flatten();
    let destination = match action {
        Some(CallDestination::Body(ClosedCall::Receive) | CallDestination::InitializerTemporary) => {
            let ([output] | [output, _]) = outputs.outputs.as_slice() else {
                return Err(format!("invalid closed aggregate output {symbol}"));
            };
            if mixed.is_none() && outputs.outputs.len() != 1 {
                return Err(format!("invalid closed aggregate output {symbol}"));
            }
            Some(entry_storage_alloca(
                context,
                state,
                &mixed.map_or(output, |lanes| &outputs.outputs[lanes.owner]).ty,
                "aggregate_destination",
            )?)
        }
        Some(CallDestination::Body(ClosedCall::Forward)) => Some(state.aggregate.as_ref().and_then(|aggregate| aggregate.destination)
            .ok_or_else(|| "missing closed aggregate forwarding destination".to_owned())?),
        Some(CallDestination::Initializer(global)) => Some(global),
        None => None,
    };
    let immediate = matches!(input, Some(CallInput::Initializer(ClosedInitializerInput::Immediate(_))));
    if immediate {
        let frame = state.initializer_aggregate.as_mut().expect("closed initializer immediate context");
        if frame.immediate_consumer.is_some() || frame.receipt.is_some() {
            return Err("overlapping closed initializer immediate argument".into());
        }
        frame.immediate_consumer = Some(ordinal);
    }
    let mut operands = Vec::new();
    for (argument, parameter) in arguments.iter().zip(parameters) {
        let emitted = match project {
            Some((module, modules)) => {
                emit_project_typed_expression(context, state, argument, parameter, module, modules)?
            }
            None => emit_typed_expression(context, state, argument, parameter)?,
        };
        operands.push(BasicMetadataValueEnum::from(take_basic(emitted)?));
    }
    if immediate {
        state.initializer_aggregate.as_mut().expect("closed initializer immediate context").immediate_consumer = None;
    }
    if let Some(destination) = destination {
        operands.push(destination.into());
    }
    let owner_slots = super::expression::stage_checked_return_sidecars(context, state, owner_outputs, &mut operands)?;
    if let Some(input) = input {
        let base = match input {
            CallInput::Body(ClosedInput::Transfer(receipt)) => {
                let aggregate = state.aggregate.as_mut().expect("closed aggregate input context");
                if aggregate.plan.deferred_call_inputs.get(&ordinal) == Some(&receipt) {
                    aggregate.receipts.get(&receipt).copied()
                        .ok_or_else(|| "missing deferred owner receipt at committed input call".to_owned())?
                } else {
                    aggregate.receipts.remove(&receipt)
                        .ok_or_else(|| "missing owner receipt at committed input call".to_owned())?
                }
            },
            CallInput::Body(ClosedInput::ForwardFormal(source)) => {
                let aggregate = state.aggregate.as_mut().expect("closed aggregate input context");
                let (formal, base) = aggregate.input.take()
                    .ok_or_else(|| "missing original formal base at owner transfer".to_owned())?;
                if formal != source {
                    return Err("formal owner transfer identity mismatch".to_owned());
                }
                base
            }
            CallInput::Body(ClosedInput::ForwardFormalDeferred(source)) => {
                let aggregate = state.aggregate.as_ref().expect("closed aggregate input context");
                let (formal, base) = aggregate.input.ok_or_else(|| "missing deferred formal base at owner transfer".to_owned())?;
                if formal != source || aggregate.plan.input_release_guard.is_none() {
                    return Err("conditional formal owner transfer identity mismatch".to_owned());
                }
                base
            }
            CallInput::Body(ClosedInput::Plain) => context.ptr_type(AddressSpace::default()).const_null(),
            CallInput::Initializer(input) => {
                let frame = state.initializer_aggregate.as_mut().expect("closed initializer input frame");
                frame.visit(ordinal)?;
                match (input, frame.receipt.take()) {
                    (ClosedInitializerInput::Consume(expected) | ClosedInitializerInput::Immediate(expected),
                        Some((actual, base))) if expected == actual => base,
                    (ClosedInitializerInput::Plain, None) => context.ptr_type(AddressSpace::default()).const_null(),
                    _ => return Err("closed initializer original base is not available at its paired input".into()),
                }
            }
        };
        operands.push(base.into());
    }
    if action.is_none() {
        let call = state.builder.build_call(function, &operands, "aggregate_input_call").map_err(builder_error)?;
        super::expression::publish_checked_return_sidecars(context, state, owner_slots)?;
        return super::expression::ordinary_call_result(call.try_as_basic_value(), outputs);
    }
    let transport = state
        .builder
        .build_call(function, &operands, "aggregate_original_base")
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "missing closed aggregate original base".to_owned())?;
    state.call_owner_flags.clear();
    if mixed.is_some() && matches!(action, Some(CallDestination::Body(ClosedCall::Forward))) {
        let aggregate = state.aggregate.as_mut().expect("closed aggregate call context");
        if aggregate.mixed_return.replace(transport.into_struct_value()).is_some() {
            return Err("duplicate mixed aggregate forwarded result".into());
        }
        return Ok(EmitValue::Unit);
    }
    let (base, pair) = if let Some(lanes) = mixed {
        let pair = transport.into_struct_value();
        let base = state.builder.build_extract_value(pair, lanes.owner as u32, "aggregate_base")
            .map_err(builder_error)?.into_pointer_value();
        (base, Some((pair, lanes.owner)))
    } else {
        (transport.into_pointer_value(), None)
    };
    match action.expect("return transport action") {
        CallDestination::Body(action) => {
            let aggregate = state.aggregate.as_mut().expect("closed aggregate call context");
            match action {
                ClosedCall::Forward => aggregate.set_base(base)?,
                ClosedCall::Receive => {
                    if aggregate.receipts.insert(ordinal, base).is_some() {
                        return Err("duplicate closed aggregate receipt".into());
                    }
                }
            }
        }
        CallDestination::Initializer(_) | CallDestination::InitializerTemporary => {
            let frame = state.initializer_aggregate.as_mut().expect("closed initializer receiver frame");
            frame.visit(ordinal)?;
            if frame.receipt.replace((ordinal, base)).is_some() {
                return Err("overlapping closed initializer receipts".into());
            }
        }
    }
    let destination = destination.expect("return destination");
    if let Some((pair, lane)) = pair {
        let value = state.builder.build_insert_value(pair, destination, lane as u32, "aggregate_values")
            .map_err(builder_error)?.into_struct_value();
        Ok(EmitValue::Aggregate {
            value,
            outputs: outputs.outputs.iter().map(|output| output.ty.clone()).collect(),
        })
    } else {
        Ok(EmitValue::Basic(destination.into()))
    }
}

pub(super) fn release_scope<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    block: &crate::scalar::ScalarBlock,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(());
    };
    if aggregate.plan.scope_exits.is_empty() {
        return Ok(());
    }
    let ordinal = state.body_call_nodes.and_then(|nodes| nodes.block_ordinal(block))
        .ok_or_else(|| "missing bound block for aggregate scope exit".to_owned())?;
    let Some(exit) = aggregate.plan.scope_exits.get(&ordinal).copied() else {
        return Ok(());
    };
    if let Some(ClosedLiteralArray::Replacement { exit: planned, block, unit, .. }) = aggregate.plan.literal_array {
        if (planned, block) != (exit, ordinal) {
            return Err("checked literal array release does not match its exit".into());
        }
        let receipt = std::mem::replace(&mut aggregate.literal_array_receipt, LiteralArrayReceipt::Finished);
        let backing = match (unit, receipt) {
            (None, LiteralArrayReceipt::Fresh { base, joined_storage: None }) => base,
            (Some(_), LiteralArrayReceipt::Joined(storage)) => state.builder
                .build_load(context.ptr_type(AddressSpace::default()), storage, "joined_array_owner")
                .map_err(builder_error)?.into_pointer_value(),
            _ => return Err("checked literal array exit has no matching receipt".into()),
        };
        return release_literal_array_base(context, state, backing);
    }
    let releases = aggregate.plan.releases.get(&exit);
    let release_input = aggregate.plan.input_release == Some(exit);
    if releases.is_none() && !release_input {
        return Ok(());
    }
    declare_core_runtime(context, state.module);
    let free = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "missing closed aggregate release".to_owned())?;
    for planned in releases.into_iter().flatten().rev() {
        let ordinal = &planned.receipt;
        let base = aggregate
            .receipts
            .remove(ordinal)
            .ok_or_else(|| "missing closed aggregate scope receipt".to_owned())?;
        if let Some(guard) = planned.guard {
            let live = aggregate.receipt_guards.get(&guard).copied()
                .ok_or_else(|| "missing guarded aggregate receipt at scope cleanup".to_owned())?;
            let function = state.builder.get_insert_block().and_then(|block| block.get_parent())
                .ok_or_else(|| "missing guarded aggregate release block".to_owned())?;
            let release = context.append_basic_block(function, "aggregate.receipt.release");
            let next = context.append_basic_block(function, "aggregate.receipt.end");
            state.builder.build_conditional_branch(live, release, next).map_err(builder_error)?;
            state.builder.position_at_end(release);
            state.builder.build_call(free, &[base.into()], "aggregate_release").map_err(builder_error)?;
            state.builder.build_unconditional_branch(next).map_err(builder_error)?;
            state.builder.position_at_end(next);
        } else {
            state.builder.build_call(free, &[base.into()], "aggregate_release").map_err(builder_error)?;
        }
    }
    if release_input {
        let (slot, base) = aggregate.input.take().ok_or_else(|| "missing original formal base at owner exit".to_owned())?;
        if aggregate.plan.input != Some(slot) {
            return Err("formal owner release identity mismatch".to_owned());
        }
        let active = if let Some(guard) = aggregate.plan.input_release_guard {
            Some(aggregate.receipt_guards.get(&guard).copied()
                .ok_or_else(|| "missing guarded formal receipt at scope cleanup".to_owned())?)
        } else if aggregate.plan.input_mixed {
            Some(state.builder.build_is_not_null(base, "aggregate_formal_owned").map_err(builder_error)?)
        } else {
            None
        };
        if let Some(live) = active {
            let current = state.builder.get_insert_block().and_then(|block| block.get_parent())
                .ok_or_else(|| "missing guarded aggregate formal owner block".to_owned())?;
            let release = context.append_basic_block(current, "aggregate.formal.release");
            let next = context.append_basic_block(current, "aggregate.formal.end");
            state.builder.build_conditional_branch(live, release, next).map_err(builder_error)?;
            state.builder.position_at_end(release);
            state.builder.build_call(free, &[base.into()], "aggregate_formal_release").map_err(builder_error)?;
            state.builder.build_unconditional_branch(next).map_err(builder_error)?;
            state.builder.position_at_end(next);
        } else {
            state.builder.build_call(free, &[base.into()], "aggregate_formal_release").map_err(builder_error)?;
        }
    }
    Ok(())
}

pub(super) fn forwarded_mixed_return(state: &EmitState<'_, '_>) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate|
        matches!(aggregate.plan.returning, Some(ClosedReturn::Forward { .. }))
            && aggregate.mixed_return.is_some())
}

pub(super) fn holds_registered_return(state: &EmitState<'_, '_>) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| aggregate.registered_allocation.is_some())
}

pub(super) fn match_mixed_branch_conditional(
    state: &EmitState<'_, '_>, expression: &ScalarExpression,
) -> Result<(), String> {
    if let Some(conditional) = state.aggregate.as_ref().and_then(|aggregate| aggregate.plan.mixed_branch_conditional()) {
        if state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)) != Some(conditional) {
            return Err("mixed aggregate return conditional does not match its sealed occurrence".into());
        }
    }
    Ok(())
}

fn mixed_allocation_pair<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>, result: &EmitValue<'ctx>,
    outputs: &crate::ScalarOutputSequence, lane: usize, destination: PointerValue<'ctx>, base: PointerValue<'ctx>,
) -> Result<inkwell::values::StructValue<'ctx>, String> {
    let EmitValue::Aggregate { value, .. } = result else {
        return Err("mixed aggregate return has no ordered values".into());
    };
    let descriptor = extract_output(state, result, lane)?;
    store_value(context, state, destination, &outputs.outputs[lane].ty, descriptor)?;
    Ok(state.builder.build_insert_value(*value, base, lane as u32, "closed_base")
        .map_err(builder_error)?.into_struct_value())
}

pub(super) fn commit_mixed_branch_arm<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>, block: &ScalarBlock,
    result: EmitValue<'ctx>, outputs: &crate::ScalarOutputSequence,
) -> Result<EmitValue<'ctx>, String> {
    let Some(aggregate) = state.aggregate.as_mut() else { return Ok(result) };
    let Some(lane) = aggregate.plan.mixed_branch_owner_lane() else { return Ok(result) };
    let Some((arm, _)) = state.body_call_nodes.and_then(|nodes| nodes.block_ordinal(block))
        .and_then(|block| aggregate.plan.mixed_branch_arm(block)) else { return Ok(result) };
    if aggregate.mixed_branch_arms & (1 << arm) != 0
        || !mixed_owned_struct_result(outputs).is_some_and(|lanes| lanes.owner == lane)
    {
        return Err("invalid sealed mixed aggregate arm".into());
    }
    let destination = aggregate.destination.ok_or_else(|| "missing mixed aggregate destination".to_owned())?;
    let name = aggregate.registered_allocation.take().ok_or_else(|| "missing mixed arm allocation".to_owned())?;
    let base = aggregate.base.take().filter(|_| state.runtime_array_owners.get(&name) == Some(&true))
        .ok_or_else(|| "missing live mixed arm backing".to_owned())?;
    let pair = mixed_allocation_pair(context, state, &result, outputs, lane, destination, base)?;
    state.runtime_array_owners.insert(name, false);
    state.aggregate.as_mut().expect("sealed mixed arm").mixed_branch_arms |= 1 << arm;
    let EmitValue::Aggregate { outputs, .. } = result else { unreachable!("ordered mixed result checked") };
    Ok(EmitValue::Aggregate { value: pair, outputs })
}

pub(super) fn commit_return<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    block: &ScalarBlock,
    result: &EmitValue<'ctx>,
    outputs: &crate::ScalarOutputSequence,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(());
    };
    if let Some(ClosedLiteralArray::FreshLeaf { output, .. }) = aggregate.plan.literal_array {
        let bound = block.final_expressions().next()
            .and_then(|(expression, _)| state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
        if bound != Some(output)
            || !matches!(outputs.outputs.as_slice(), [output] if matches!(output.ty, ScalarType::RuntimeArray { .. }))
        {
            return Err("checked fresh literal array return does not match its output occurrence".into());
        }
        let LiteralArrayReceipt::Original { base, storage: None, argument_extent: None, in_writer: None } =
            std::mem::replace(&mut aggregate.literal_array_receipt, LiteralArrayReceipt::Finished)
        else { return Err("checked fresh literal array return has no original receipt".into()) };
        // The admitted immutable direct binding returns the recorded LLVM SSA
        // value itself, not a slot load. This compares IR identity, not runtime addresses.
        if !matches!(result, EmitValue::Basic(BasicValueEnum::PointerValue(returned)) if *returned == base) {
            return Err("checked fresh literal array return did not carry its allocation".into());
        }
        return Ok(());
    }
    if let Some(destination) = aggregate.ordinary_destination {
        let [output] = outputs.outputs.as_slice() else {
            return Err("invalid ordinary aggregate returned value".into());
        };
        if aggregate.ordinary_committed || !matches!(output.ty, ScalarType::Struct(_))
            || !state.return_owner_slots.is_empty() || !state.fresh_return_outputs.is_empty()
        {
            return Err("overlapping ordinary aggregate result materialization".into());
        }
        store_value(context, state, destination, &output.ty, result.clone())?;
        state.aggregate.as_mut().expect("ordinary aggregate frame").ordinary_committed = true;
        return Ok(());
    }
    if aggregate.plan.returning == Some(ClosedReturn::MixedBranch) {
        let EmitValue::Aggregate { value, .. } = result else {
            return Err("sealed mixed aggregate paths did not form an ordered result".into());
        };
        if aggregate.mixed_branch_arms != 0b11 || aggregate.base.is_some() || aggregate.registered_allocation.is_some() {
            return Err("sealed mixed aggregate paths did not transfer both planned receipts".into());
        }
        aggregate.mixed_return = Some(*value);
        return Ok(());
    }
    if let Some(ClosedReturn::Recommit { call, output }) = aggregate.plan.returning {
        let returned = block.final_expressions().next().and_then(|(expression, _)|
            state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
        let [descriptor] = outputs.outputs.as_slice() else {
            return Err("recommitted aggregate return is not a single descriptor".into());
        };
        if returned != Some(output) || !matches!(descriptor.ty, ScalarType::Struct(_)) || aggregate.committed {
            return Err("recommitted aggregate return changed its proved root output".into());
        }
        let destination = aggregate.destination.ok_or_else(|| "missing recommitted aggregate destination".to_owned())?;
        let base = aggregate.receipts.remove(&call)
            .ok_or_else(|| "missing original backing at recommitted aggregate return".to_owned())?;
        store_value(context, state, destination, &descriptor.ty, result.clone())?;
        let aggregate = state.aggregate.as_mut().expect("recommitted aggregate return frame");
        aggregate.set_base(base)?;
        aggregate.committed = true;
        return Ok(());
    }
    if !matches!(aggregate.plan.returning, Some(ClosedReturn::Allocation { .. })) {
        return Ok(());
    }
    let mixed = mixed_owned_struct_result(outputs);
    let ([output] | [output, _]) = outputs.outputs.as_slice() else {
        return Err("invalid closed aggregate returned descriptor".into());
    };
    if mixed.is_none() && outputs.outputs.len() != 1 {
        return Err("invalid closed aggregate returned descriptor".into());
    }
    let destination = aggregate.destination
        .ok_or_else(|| "missing closed aggregate returned destination".to_owned())?;
    let name = aggregate.registered_allocation.take()
        .ok_or_else(|| "missing registered aggregate allocation at handoff".to_owned())?;
    let base = aggregate.base
        .filter(|_| state.runtime_array_owners.get(&name).copied().unwrap_or(false))
        .ok_or_else(|| "closed aggregate handoff has no live original owner".to_owned())?;
    if let Some(lanes) = mixed {
        let pair = mixed_allocation_pair(context, state, result, outputs, lanes.owner, destination, base)?;
        state.aggregate.as_mut().expect("closed aggregate handoff").mixed_return = Some(pair);
    } else {
        store_value(context, state, destination, &output.ty, result.clone())?;
    }
    state.runtime_array_owners.insert(name, false);
    state.aggregate.as_mut().expect("closed aggregate handoff").committed = true;
    Ok(())
}

pub(super) fn finish_return<'ctx>(
    state: &mut EmitState<'ctx, '_>,
) -> Result<Option<()>, String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(None);
    };
    if aggregate.plan.literal_array.is_some()
        && !matches!(aggregate.literal_array_receipt, LiteralArrayReceipt::Finished)
    {
        return Err("unfinished checked literal array receipt".into());
    }
    if aggregate.failure_site.is_some()
        && (!aggregate.failure_arm_visited || aggregate.failure_allocation.is_some())
    {
        return Err("unconsumed local array failure allocation".into());
    }
    if aggregate.plan.checked_backing_wrapper.is_some() && !aggregate.checked_backing_forwarded {
        return Err("missing private checked backing forwarding".into());
    }
    if aggregate.plan.checked_backing_producer.is_some() && aggregate.checked_backing_arms != 0b11 {
        return Err("incomplete checked backing producer arms".into());
    }
    if aggregate.plan.checked_backing_consumer.is_some() && (!aggregate.checked_backing_released
        || aggregate.checked_backing_pending.is_some() || aggregate.checked_backing_receiver.is_some())
    {
        return Err("unreleased checked backing receipt".into());
    }
    if aggregate.plan.scalar_descriptor_assignment.is_some() && !aggregate.scalar_descriptor_enrolled {
        return Err("descriptor assignment did not commit ownership".into());
    }
    if !aggregate.receipts.is_empty() || aggregate.receipt_guards.len() != aggregate.plan.conditional_guard_count || aggregate.input.is_some() {
        return Err("unreleased closed aggregate receipt".into());
    }
    if aggregate.ordinary_called == aggregate.plan.ordinary_calls.is_empty() {
        return Err("ordinary aggregate call occurrence was not emitted".into());
    }
    if aggregate.ordinary_destination.is_some() {
        if !aggregate.ordinary_committed {
            return Err("ordinary aggregate result did not materialize before cleanup".into());
        }
        state.builder.build_return(None).map_err(builder_error)?;
        return Ok(Some(()));
    }
    let Some(route) = aggregate.plan.returning else {
        return Ok(None);
    };
    if matches!(route, ClosedReturn::Allocation { .. } | ClosedReturn::Recommit { .. }) && !aggregate.committed {
        return Err("closed aggregate return did not commit ownership before cleanup".into());
    }
    if let Some(pair) = aggregate.mixed_return.take() {
        state.builder.build_return(Some(&pair)).map_err(builder_error)?;
    } else {
        let base = aggregate.base.take()
            .ok_or_else(|| "missing closed aggregate returned base".to_owned())?;
        state.builder.build_return(Some(&base)).map_err(builder_error)?;
    }
    Ok(Some(()))
}
