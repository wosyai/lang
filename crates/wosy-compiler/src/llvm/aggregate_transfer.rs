use super::*;
use crate::scalar::{ClosedBody, ClosedCall, ClosedInitializer, ClosedInitializerInput, ClosedInput, ClosedReturn, ConcreteCallSelection, ScalarBinding};
use crate::scalar::control::NamedPlaceRootIdentity;
use crate::ScalarPlace;
use super::memory::checked_fixed_element_pointer;
use inkwell::types::AnyType;

pub(super) struct InitializerEmitter<'ctx, 'plan> {
    plan: &'plan ClosedInitializer,
    receipt: Option<(usize, PointerValue<'ctx>)>,
    immediate_consumer: Option<usize>,
    visited: BTreeSet<usize>,
    ordinary_visited: BTreeSet<usize>,
    ordinary_binding: Option<usize>,
}

impl<'ctx, 'plan> InitializerEmitter<'ctx, 'plan> {
    pub(super) fn new(plan: &'plan ClosedInitializer) -> Self {
        Self { plan, receipt: None, immediate_consumer: None, visited: BTreeSet::new(), ordinary_visited: BTreeSet::new(), ordinary_binding: None }
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
        if frame.receipt.is_some() || frame.immediate_consumer.is_some() || frame.ordinary_binding.is_some()
            || frame.ordinary_visited.len() != frame.plan.ordinary_calls.len()
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


pub(super) struct AggregateEmitter<'ctx, 'plan> {
    plan: &'plan ClosedBody,
    destination: Option<PointerValue<'ctx>>,
    ordinary_destination: Option<PointerValue<'ctx>>,
    ordinary_committed: bool,
    ordinary_indexed_emitted: u8,
    ordinary_visited: BTreeSet<usize>,
    base: Option<PointerValue<'ctx>>,
    registered_allocation: Option<String>,
    failure_site: Option<((usize, usize), usize)>,
    failure_allocation: Option<(PointerValue<'ctx>, PointerValue<'ctx>, String)>,
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
    input: Option<(usize, PointerValue<'ctx>)>,
}

impl<'ctx, 'plan> AggregateEmitter<'ctx, 'plan> {
    pub(super) fn same_array_rebind(&self, ordinal: usize) -> bool {
        self.plan.same_array_rebinds.contains(&ordinal)
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
            if failure_site != failure.failure.map(|arm| (failure.allocation, arm)) {
                return Err("mixed checked backing producer has no matching local release".into());
            }
        }
        Ok(Some(Self {
            plan,
            input,
            destination,
            ordinary_destination,
            ordinary_committed: false,
            ordinary_indexed_emitted: 0,
            ordinary_visited: BTreeSet::new(),
            base: None,
            registered_allocation: None,
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
    if !matches!(
        outputs.outputs.as_slice(),
        [crate::ScalarOutput {
            ty: ScalarType::Struct(_),
            ..
        }]
    ) {
        return Err("invalid closed aggregate return layout".into());
    }
    let pointer = context.ptr_type(AddressSpace::default());
    let mut arguments = parameters
        .iter()
        .map(|ty| basic_type(context, ty, layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum<'ctx>>, _>>()?;
    arguments.push(pointer.into());
    Ok(pointer.fn_type(&arguments, false))
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
        state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding)) == Some(plan.allocation)
    }) && aggregate.checked_backing_base.replace((base, name.to_owned())).is_some() {
        return Err("duplicate private checked backing allocation".into());
    }
    let Some(ClosedReturn::Allocation { block, item }) = aggregate.plan.returning else {
        return Ok(());
    };
    if state
        .body_call_nodes
        .and_then(|nodes| nodes.binding_position(binding))
        != Some((block, item))
    {
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
    let Some((position, _)) = aggregate.failure_site else {
        return Ok(());
    };
    if state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding)) == Some(position)
        && aggregate.failure_allocation.replace((base, live, name.to_owned())).is_some()
    {
        return Err("duplicate local array failure allocation".into());
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
    let Some((_, arm)) = aggregate.failure_site else {
        return Ok(());
    };
    let ordinal = state.body_call_nodes.and_then(|nodes| nodes.block_ordinal(block))
        .ok_or_else(|| "missing bound block for local array failure".to_owned())?;
    if ordinal != arm {
        return Ok(());
    }
    if std::mem::replace(&mut aggregate.failure_arm_visited, true) {
        return Err("duplicate local array failure arm".into());
    }
    let (base, live, owner) = aggregate.failure_allocation.take()
        .ok_or_else(|| "missing original local array failure allocation".to_owned())?;
    if !state.automatic_return_result_owners.is_empty() || !state.joined_return_bindings.is_empty()
        || state.runtime_array_owners.keys().any(|name| name != &owner)
    {
        if aggregate.plan.checked_backing_producer.is_some_and(|plan| plan.failure == Some(arm)) {
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
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    if !matches!(
        call_action(state, &binding.value),
        Some((_, ClosedCall::Receive))
    ) {
        return Ok(false);
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

pub(super) fn prepare_ordinary_initializer_binding(
    state: &mut EmitState<'_, '_>,
    binding: &ScalarBinding,
    source: &wosy_syntax::SourceIdentity,
) -> Result<(), String> {
    let Some(frame) = state.initializer_aggregate.as_mut() else { return Ok(()) };
    if frame.plan.ordinary_calls.is_empty() || !matches!(binding.value, ScalarExpression::Call { .. }) {
        return Ok(());
    }
    let nodes = state.initializer_nodes.ok_or_else(|| "missing ordinary initializer nodes".to_owned())?;
    let (ordinal, _) = nodes.bound_call(&binding.value)?;
    let Some((_, storage)) = frame.plan.ordinary_calls.get(&ordinal) else { return Ok(()) };
    let [receiver] = binding.receivers.as_slice() else {
        return Err("ordinary initializer result changed receiver arity".into());
    };
    if binding.is_allocation || source != &storage.source || receiver.name_span != storage.range
        || frame.ordinary_binding.replace(ordinal).is_some() {
        return Err("ordinary initializer result changed its binding identity".into());
    }
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
        if let Some((declaration, _)) = frame.plan.ordinary_calls.get(&ordinal) {
            if frame.receipt.is_some() || frame.immediate_consumer.is_some() {
                return Err("ordinary aggregate result crosses a closed initializer receipt".into());
            }
            if !matches!(target, ModuleInitializationCallTarget::Direct {
                declaration: selected, concrete: ConcreteCallSelection::Declaration,
            } if declaration == selected) {
                return Err("ordinary initializer call changed its declaration identity".into());
            }
            let frame = state.initializer_aggregate.as_mut().expect("ordinary initializer call frame");
            if frame.ordinary_binding.take() != Some(ordinal) {
                return Err("ordinary initializer result outside its bound storage initialization".into());
            }
            if !frame.ordinary_visited.insert(ordinal) {
                return Err("ordinary initializer call occurrence emitted twice".into());
            }
            return emit_ordinary_result_call(context, state, expression, arguments, project, Some(target)).map(Some);
        }
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
    if let Some(value) = try_emit_witnessed_ordinary_call(context, state, expression, arguments, project, ordinal)? {
        return Ok(Some(value));
    }
    let aggregate = state.aggregate.as_ref().expect("closed aggregate call frame");
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

pub(super) fn emit_value_field_receiver<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    receiver: &ScalarExpression,
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<PointerValue<'ctx>, String> {
    let ScalarExpression::Call { arguments, .. } = receiver else {
        return Err("field value receiver must be a call result".to_owned());
    };
    let ordinal = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(receiver))
        .ok_or_else(|| "call result has no proven caller-owned storage for field projection".to_owned())?;
    let value = try_emit_witnessed_ordinary_call(context, state, receiver, arguments, project, ordinal)?
        .ok_or_else(|| "call result has no proven caller-owned storage for field projection".to_owned())?;
    match take_basic(value)? {
        BasicValueEnum::PointerValue(pointer) => Ok(pointer),
        _ => Err("proven field receiver did not yield caller-owned storage".to_owned()),
    }
}

fn try_emit_witnessed_ordinary_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
    ordinal: usize,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(declaration) = state.aggregate.as_ref()
        .and_then(|aggregate| aggregate.plan.ordinary_calls.get(&ordinal)) else { return Ok(None) };
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
    if !aggregate.ordinary_visited.insert(ordinal) {
        return Err("ordinary aggregate call occurrence emitted twice".into());
    }
    emit_ordinary_result_call(context, state, expression, arguments, project, None).map(Some)
}

fn emit_ordinary_result_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
    initializer_target: Option<&ModuleInitializationCallTarget>,
) -> Result<EmitValue<'ctx>, String> {
    let symbol = state.direct_aggregate_symbol(expression, initializer_target)?;
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
    let destination = match action {
        Some(CallDestination::Body(ClosedCall::Receive) | CallDestination::InitializerTemporary) => {
            let [output] = outputs.outputs.as_slice() else {
                return Err(format!("invalid closed aggregate output {symbol}"));
            };
            Some(entry_storage_alloca(
                context,
                state,
                &output.ty,
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
            CallInput::Body(ClosedInput::Transfer(receipt)) => state.aggregate.as_mut().expect("closed aggregate input context").receipts.remove(&receipt)
                .ok_or_else(|| "missing owner receipt at committed input call".to_owned())?,
            CallInput::Body(ClosedInput::ForwardFormal(source)) => {
                let aggregate = state.aggregate.as_mut().expect("closed aggregate input context");
                let (formal, base) = aggregate.input.take()
                    .ok_or_else(|| "missing original formal base at owner transfer".to_owned())?;
                if formal != source {
                    return Err("formal owner transfer identity mismatch".to_owned());
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
    let base = state
        .builder
        .build_call(function, &operands, "aggregate_original_base")
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "missing closed aggregate original base".to_owned())?
        .into_pointer_value();
    state.call_owner_flags.clear();
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
    Ok(EmitValue::Basic(destination.expect("return destination").into()))
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
    let Some(exit) = aggregate.plan.scope_exits.get(&ordinal) else {
        return Ok(());
    };
    let releases = aggregate.plan.releases.get(exit);
    let release_input = aggregate.plan.input_release.as_ref() == Some(exit);
    if releases.is_none() && !release_input {
        return Ok(());
    }
    declare_core_runtime(context, state.module);
    let free = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "missing closed aggregate release".to_owned())?;
    for ordinal in releases.into_iter().flatten().rev() {
        let base = aggregate
            .receipts
            .remove(ordinal)
            .ok_or_else(|| "missing closed aggregate scope receipt".to_owned())?;
        state
            .builder
            .build_call(free, &[base.into()], "aggregate_release")
            .map_err(builder_error)?;
    }
    if release_input {
        let (slot, base) = aggregate.input.take().ok_or_else(|| "missing original formal base at owner exit".to_owned())?;
        if aggregate.plan.input != Some(slot) {
            return Err("formal owner release identity mismatch".to_owned());
        }
        if aggregate.plan.input_mixed {
            let current = state.builder.get_insert_block().and_then(|block| block.get_parent())
                .ok_or_else(|| "missing mixed aggregate formal owner block".to_owned())?;
            let live = state.builder.build_is_not_null(base, "aggregate_formal_owned").map_err(builder_error)?;
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

pub(super) fn holds_registered_return(state: &EmitState<'_, '_>) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| aggregate.registered_allocation.is_some())
}

pub(super) fn emit_checked_ordinary_snapshot<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(aggregate) = state.aggregate.as_ref() else { return Ok(None) };
    let terminal = aggregate.plan.terminal_indexed.as_deref();
    let ordinary = aggregate.plan.ordinary_indexed.as_ref();
    if terminal.is_none() && ordinary.is_none() { return Ok(None) }
    let nodes = state.body_call_nodes.ok_or_else(|| "missing checked snapshot body nodes".to_owned())?;
    let ordinal = nodes.expression_ordinal(expression);
    let (proof, bit) = if let Some(proof) = ordinary {
        (proof, 1)
    } else if let Some((position, proof)) = terminal.into_iter().flatten().enumerate()
        .find(|(_, proof)| Some(proof.value) == ordinal)
    {
        (proof, 1 << position)
    } else {
        return Ok(None);
    };
    let ScalarExpression::IndexedRead { place, .. } = expression else {
        return Err("checked snapshot changed its final expression".into());
    };
    let ScalarPlace::Index { base, index, .. } = place else {
        return Err("checked snapshot changed its indexed source".into());
    };
    let ScalarPlace::Name { name, .. } = base.as_ref() else {
        return Err("checked snapshot changed its storage root".into());
    };
    if ordinal != Some(proof.value)
        || nodes.place_ordinal(place) != Some(proof.place)
        || nodes.place_ordinal(base) != Some(proof.base)
        || nodes.expression_ordinal(index) != Some(proof.index)
        || !matches!(nodes.named_place_root(base),
            Some(NamedPlaceRootIdentity::ModuleValue(storage)) if storage == &proof.storage)
        || aggregate.ordinary_indexed_emitted & bit != 0
    {
        return Err("checked snapshot changed its prepared source identity".into());
    }
    let (global, ty) = match project {
        Some(scope) if scope.module.source == proof.storage.source => state.project_names
            .ok_or_else(|| "missing checked snapshot module scope".to_owned())?
            .for_module(scope.module).globals.get(name),
        Some(_) => return Err("checked snapshot changed its storage module".into()),
        None => state.globals.get(name),
    }.ok_or_else(|| "missing checked snapshot module storage".to_owned())?;
    if nodes.checked_place_type(base) != Some(ty)
        || !matches!(ty, ScalarType::Array { length, .. } if *length == proof.length)
    {
        return Err("checked snapshot changed its prepared storage type".into());
    }
    let pointer = checked_fixed_element_pointer(
        context, state, ty, global.as_pointer_value(), proof.length, index, project,
        ("snapshot_index_out_of_bounds", "snapshot_index_panic", "snapshot_index_continue"),
    )?;
    state.aggregate.as_mut().expect("checked snapshot frame").ordinary_indexed_emitted |= bit;
    Ok(Some(EmitValue::Basic(pointer.into())))
}

pub(super) fn commit_return<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    result: &EmitValue<'ctx>,
    outputs: &crate::ScalarOutputSequence,
) -> Result<(), String> {
    let Some(aggregate) = &mut state.aggregate else {
        return Ok(());
    };
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
    if !matches!(aggregate.plan.returning, Some(ClosedReturn::Allocation { .. })) {
        return Ok(());
    }
    let [output] = outputs.outputs.as_slice() else {
        return Err("invalid closed aggregate returned descriptor".into());
    };
    let destination = aggregate.destination
        .ok_or_else(|| "missing closed aggregate returned destination".to_owned())?;
    let name = aggregate.registered_allocation.take()
        .ok_or_else(|| "missing registered aggregate allocation at handoff".to_owned())?;
    if aggregate.base.is_none() || !state.runtime_array_owners.get(&name).copied().unwrap_or(false) {
        return Err("closed aggregate handoff has no live original owner".into());
    }
    store_value(context, state, destination, &output.ty, result.clone())?;
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
    if !aggregate.receipts.is_empty() || aggregate.input.is_some() {
        return Err("unreleased closed aggregate receipt".into());
    }
    if aggregate.ordinary_visited.len() != aggregate.plan.ordinary_calls.len() {
        return Err("ordinary aggregate call occurrence was not emitted".into());
    }
    if aggregate.ordinary_destination.is_some() {
        let expected_indexed = if aggregate.plan.terminal_indexed.is_some() {
            0b11
        } else {
            u8::from(aggregate.plan.ordinary_indexed.is_some())
        };
        if expected_indexed != aggregate.ordinary_indexed_emitted {
            return Err("missing checked ordinary snapshot read".into());
        }
        if !aggregate.ordinary_committed {
            return Err("ordinary aggregate result did not materialize before cleanup".into());
        }
        state.builder.build_return(None).map_err(builder_error)?;
        return Ok(Some(()));
    }
    let Some(route) = aggregate.plan.returning else {
        return Ok(None);
    };
    if matches!(route, ClosedReturn::Allocation { .. }) && !aggregate.committed {
        return Err("closed aggregate return did not commit ownership before cleanup".into());
    }
    let base = aggregate.base.take()
        .ok_or_else(|| "missing closed aggregate returned base".to_owned())?;
    state.builder.build_return(Some(&base)).map_err(builder_error)?;
    Ok(Some(()))
}
