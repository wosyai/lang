use super::*;
use crate::scalar::control::NamedPlaceRootIdentity;
use crate::ScalarPlace;
use crate::scalar::{mixed_owned_struct_result, mixed_owner_result_lanes, ArrayWriterSide, ClosedBody, ClosedCall, ClosedInitializer, ClosedInitializerInput, ClosedInput, ClosedLiteralArray, ClosedLocalArrayRelease, ClosedReturn, ConcreteCallSelection, FreshArrayExtent, ReplacementArrayExtent, ScalarBinding};
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
enum CallInput<'plan> {
    Body(&'plan [(usize, ClosedInput)]),
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
        joined_storage: Option<(PointerValue<'ctx>, PointerValue<'ctx>)>,
    },
    Joined((PointerValue<'ctx>, PointerValue<'ctx>)),
    Finished,
}

struct LiteralArrayBranchOriginal<'ctx> {
    base: PointerValue<'ctx>,
    storage: (PointerValue<'ctx>, PointerValue<'ctx>),
}

pub(super) struct AggregateEmitter<'ctx, 'plan> {
    plan: &'plan ClosedBody,
    pub(super) fallible_relay_next: usize,
    pub(super) fallible_relay_unwind: Option<inkwell::basic_block::BasicBlock<'ctx>>,
    pub(super) borrowed_extent: Option<inkwell::values::IntValue<'ctx>>,
    pub(super) private_runtime_access_pair: Option<(inkwell::values::IntValue<'ctx>, inkwell::values::IntValue<'ctx>)>,
    destination: Option<PointerValue<'ctx>>,
    ordinary_destination: Option<PointerValue<'ctx>>,
    local_choice_destination: Option<PointerValue<'ctx>>,
    local_choice_committed: usize,
    local_choice_visited: usize,
    ordinary_committed: bool,
    ordinary_indexed_emitted: u8,
    ordinary_visited: BTreeSet<usize>,
    literal_array_receipt: LiteralArrayReceipt<'ctx>,
    array_descriptor_rebound: bool,
    array_descriptor_extent: Option<inkwell::values::IntValue<'ctx>>,
    array_descriptor_result: Option<inkwell::values::StructValue<'ctx>>,
    pub(super) fresh_recommit_extent: Option<(usize, inkwell::values::IntValue<'ctx>)>,
    literal_array_branch_original: Option<LiteralArrayBranchOriginal<'ctx>>,
    base: Option<PointerValue<'ctx>>,
    registered_allocation: Option<String>,
    mixed_return: Option<inkwell::values::StructValue<'ctx>>,
    mixed_branch_arms: u8,
    structured_arms_visited: usize,
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
    receipts: BTreeMap<(usize, Option<usize>), PointerValue<'ctx>>,
    multi_destinations: Vec<PointerValue<'ctx>>,
    multi_allocations: BTreeMap<(usize, usize), Option<(PointerValue<'ctx>, String)>>,
    // SSA join values indexed by the function-local, shared guard plan.
    receipt_guards: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    inputs: BTreeMap<usize, PointerValue<'ctx>>,
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
    pub(super) fn plan(&self) -> &ClosedBody {
        self.plan
    }

    pub(super) fn rebound_array_destination(&self, call: usize) -> Option<usize> {
        self.plan.array_descriptor_rebinds.get(&call).map(|(_, destination)| *destination)
    }

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
        let destination = if plan.returning.is_some() && plan.multi_return_width() == 0 {
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
            if destination.is_some() || !plan.inputs.is_empty() || definition.count_params() as usize != parameter_count + 1 {
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
        let inputs = plan.inputs.keys().copied().enumerate().map(|(offset, slot)| {
            let position = (definition.count_params() as usize).checked_sub(plan.inputs.len())
                .and_then(|first| u32::try_from(first + offset).ok())
                .ok_or_else(|| "invalid aggregate formal sidecar position".to_owned())?;
            if (position as usize) < parameter_count || slot >= parameter_count {
                return Err("aggregate formal sidecar changed source arity".to_owned());
            }
            let base = definition.get_nth_param(position)
                .ok_or_else(|| "missing aggregate formal input base".to_owned())?;
            Ok((slot, base.into_pointer_value()))
        }).collect::<Result<BTreeMap<_, _>, String>>()?;
        let checked_backing_output = (plan.checked_backing_producer.is_some() || plan.checked_backing_wrapper.is_some())
            .then(|| definition.get_nth_param(definition.count_params() - 1)
            .ok_or_else(|| "missing private checked backing output".to_owned())
            .map(|value| value.into_pointer_value())).transpose()?;
        let failure_site = plan.local_array_failure.filter(|_| {
            plan.returning.is_none() && plan.inputs.is_empty() && plan.calls.is_empty()
                && plan.releases.is_empty() && plan.input_calls.is_empty()
        });
        if let Some(failure) = plan.checked_backing_producer.filter(|backing| backing.failure.is_some()) {
            if failure_site != failure.failure.map(|arm| ClosedLocalArrayRelease { receipt: failure.receipt, arm }) {
                return Err("mixed checked backing producer has no matching local release".into());
            }
        }
        let multi_destinations = (0..plan.multi_return_width()).map(|lane| {
            let index = u32::try_from(parameter_count + lane).map_err(|_| "owning return destination index overflow")?;
            definition.get_nth_param(index).map(|value| value.into_pointer_value())
                .ok_or_else(|| "missing ordered owning return destination".to_owned())
        }).collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Self {
            plan,
            fallible_relay_next: 0,
            fallible_relay_unwind: None,
            borrowed_extent: None,
            private_runtime_access_pair: None,
            inputs,
            destination,
            ordinary_destination,
            local_choice_destination: None,
            local_choice_committed: 0,
            local_choice_visited: 0,
            ordinary_committed: false,
            ordinary_indexed_emitted: 0,
            ordinary_visited: BTreeSet::new(),
            literal_array_receipt: LiteralArrayReceipt::Awaiting,
            array_descriptor_extent: None,
            array_descriptor_rebound: false,
            array_descriptor_result: None,
            fresh_recommit_extent: None,
            literal_array_branch_original: None,
            base: None,
            registered_allocation: None,
            mixed_return: None,
            mixed_branch_arms: 0,
            structured_arms_visited: 0,
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
            multi_destinations,
            multi_allocations: plan.multi_returns.iter().map(|&(position, _)| (position, None)).collect(),
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
    plan.filter(|body| body.fallible_allocation_leaf || body.fallible_allocation_relay || body.returning.is_some() || !body.inputs.is_empty() || body.checked_backing_producer.is_some()
        || body.checked_backing_wrapper.is_some() || body.ordinary_result || body.array_descriptor_return || body.array_descriptor_identity || body.borrowed_extent_observer.is_some() || body.borrowed_extent_forward.is_some() || body.private_runtime_access_formal.is_some())
        .map(|_| Linkage::Internal)
}

pub(super) fn array_descriptor_type<'ctx>(context: &'ctx Context) -> inkwell::types::StructType<'ctx> {
    context.struct_type(&[context.ptr_type(AddressSpace::default()).into(), context.i64_type().into()], false)
}

fn borrowed_extent_type<'ctx>(context: &'ctx Context, layout: ScalarTargetLayout) -> inkwell::types::StructType<'ctx> {
    let address = super::memory::pointer_integer_type(context, layout);
    context.struct_type(&[address.into(), context.i64_type().into()], false)
}

pub(super) fn borrowed_extent_at(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    pointer: &ScalarExpression,
) -> bool {
    state.aggregate.as_ref().and_then(|aggregate| aggregate.plan.borrowed_extent_observer)
        .is_some_and(|(occurrence, formal)| state.body_call_nodes.is_some_and(|nodes|
            nodes.expression_ordinal(expression) == Some(occurrence)
                && nodes.lexical_expression_declaration(pointer) == Some(formal)))
}

pub(super) fn capture_borrowed_extent<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    declaration: wosy_syntax::ByteSpan,
    count: inkwell::values::IntValue<'ctx>,
) -> Result<(), String> {
    if let Some(aggregate) = &mut state.aggregate {
        if aggregate.plan.borrowed_extent_call.is_some_and(|(_, owner, _)| owner == declaration)
            && aggregate.borrowed_extent.replace(count).is_some()
        {
            return Err("duplicate sealed borrowed array allocation".into());
        }
    }
    Ok(())
}

pub(super) fn extract_borrowed_extent_formal<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    argument: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, String> {
    let BasicValueEnum::StructValue(pair) = argument else {
        return Err("missing sealed borrowed array extent formal".into());
    };
    if pair.get_type() != borrowed_extent_type(context, state.target_layout) {
        return Err("invalid sealed borrowed array extent formal type".into());
    }
    let address = state.builder.build_extract_value(pair, 0, "borrowed_array_address").map_err(builder_error)?;
    let count = state.builder.build_extract_value(pair, 1, "borrowed_array_extent").map_err(builder_error)?;
    let aggregate = state.aggregate.as_mut().expect("sealed borrowed extent formal");
    if aggregate.plan.private_runtime_access_formal.is_some() {
        aggregate.private_runtime_access_pair = Some((address.into_int_value(), count.into_int_value()));
    } else {
        aggregate.borrowed_extent = Some(count.into_int_value());
    }
    Ok(address)
}

pub(super) fn emit_private_runtime_access_call<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>, expression: &ScalarExpression,
    symbol: &str, arguments: &[ScalarExpression], parameters: &[ScalarType],
    project: Option<ProjectCallScope<'_>>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan)
        .filter(|plan| !plan.private_runtime_access_calls.is_empty()) else { return Ok(None) };
    let nodes = state.body_call_nodes.ok_or("private runtime array call lacks prepared occurrences")?;
    let Some(occurrence) = nodes.expression_ordinal(expression) else { return Ok(None) };
    let Some(site) = plan.private_runtime_access_calls.get(&occurrence) else { return Ok(None) };
    let declaration = site.owner;
    let portable = plan.private_runtime_portable_calls.contains(&occurrence);
    if portable && (site.portable_birth.is_none() || !state.project_names.is_some_and(|names|
        names.private_runtime_portable_symbols.contains(symbol))) {
        return Err("portable private runtime array call lost its exact sink".into());
    }
    let Some((ScalarExpression::CheckedAddress { mutability, place: place @ ScalarPlace::Name { .. }, .. },
        scalar_arguments)) = arguments.split_first()
        else { return Err("private runtime array call changed its exact arguments".into()); };
    let Some((ScalarType::CheckedReference { inner, mutability: prepared_mode }, scalar_parameters)) = parameters.split_first()
        else { return Err("private runtime array call changed its exact parameter types".into()); };
    if mutability != prepared_mode || scalar_arguments.len() != scalar_parameters.len() {
        return Err("private runtime array call changed its loan or scalar argument arity".into());
    }
    let ScalarType::RuntimeArray { element, .. } = inner.as_ref()
        else { return Err("private runtime array call changed its scalar element type".into()); };
    if nodes.lexical_place_declaration(place) != Some(declaration) {
        return Err("private runtime array call changed its exact owner".into());
    }
    let pointer = memory::place_pointer(context, state, place, project)?;
    let Some(&(owner, extent, true)) = state.local_runtime_extents.get(&pointer)
        else { return Err("private runtime array call lacks its original checked backing and extent".into()); };
    let length = match extent {
        LocalRuntimeExtent::PortableConstruction { length, domain }
            if portable && domain.matches_consumer(state.target_layout, element) => length,
        LocalRuntimeExtent::Construction { length, signed: Some(domain) }
            if !portable && domain.matches_consumer(state.target_layout, element) => length,
        _ => return Err("private runtime array call changed its prepared extent domain".into()),
    };
    if owner != declaration {
        return Err("private runtime array call changed its original backing".into());
    }
    let address = state.builder.build_ptr_to_int(pointer, memory::pointer_integer_type(context, state.target_layout),
        "private_runtime_address").map_err(builder_error)?;
    let descriptor = borrowed_extent_type(context, state.target_layout);
    let pair = state.builder.build_insert_value(descriptor.get_undef(), address, 0, "private_runtime_argument_address")
        .map_err(builder_error)?.into_struct_value();
    let pair = state.builder.build_insert_value(pair, length, 1, "private_runtime_argument")
        .map_err(builder_error)?.into_struct_value();
    let function = *state.functions.get(symbol).ok_or("missing private runtime array accessor")?;
    let Some(ScalarType::Callable { outputs, .. }) = state.signatures.get(symbol) else {
        return Err("private runtime array accessor lost its exact output".into());
    };
    let mutable = *mutability == crate::scalar::ScalarReferenceMutability::Mutable;
    let function_type = function.get_type();
    let exact_abi =
        if mutable && crate::scalar::private_runtime_write_signature(parameters, &outputs.outputs) {
            function_type.get_param_types().as_slice()
                == [descriptor.into(), context.i64_type().into(), basic_type(context, element, state.target_layout)?.into()]
                && function_type.get_return_type().is_none()
        } else if !mutable && crate::scalar::private_runtime_read_signature(parameters, &outputs.outputs) {
            function_type.get_param_types().as_slice() == [descriptor.into(), context.i64_type().into()]
                && function_type.get_return_type() == Some(basic_type(context, &outputs.outputs[0].ty, state.target_layout)?)
        } else { false };
    if function.get_linkage() != Linkage::Internal || !exact_abi {
        return Err("private runtime array accessor changed its sealed ABI".into());
    }
    let values = super::expression::emit_contextual_arguments(context, state, scalar_arguments, scalar_parameters, project)?;
    let mut args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(parameters.len());
    args.push(pair.into());
    for value in values {
        let EmitValue::Basic(value) = value else { return Err("private runtime array accessor lacks a scalar argument".into()); };
        args.push(value.into());
    }
    let value = state.builder.build_call(function, &args, if mutable { "" } else { "private_runtime_read" })
        .map_err(builder_error)?;
    if mutable {
        Ok(Some(EmitValue::Unit))
    } else {
        let value = value.try_as_basic_value().basic().ok_or("private runtime reader returned no scalar")?;
        Ok(Some(EmitValue::Basic(value)))
    }
}

pub(super) fn emit_borrowed_extent_call<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    symbol: &str,
    arguments: &[ScalarExpression],
    parameters: &[ScalarType],
    project: Option<ProjectCallScope<'_>>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan) else { return Ok(None) };
    if plan.borrowed_extent_call.is_none() && plan.borrowed_current_widened_extents.is_empty() {
        return Ok(None);
    }
    let nodes = state.body_call_nodes.ok_or("missing sealed checked borrow nodes")?;
    let Some(occurrence) = nodes.expression_ordinal(expression) else { return Ok(None) };
    let (address, count) = if let Some(&declaration) = plan.borrowed_current_widened_extents.get(&occurrence) {
        let [ScalarExpression::CheckedAddress { mutability: crate::scalar::ScalarReferenceMutability::Shared,
            place: place @ ScalarPlace::Name { .. }, .. }] = arguments else {
            return Err("sealed widened borrow needs one whole shared place".into());
        };
        if nodes.lexical_place_declaration(place) != Some(declaration) {
            return Err("sealed widened borrow changed its checked argument identity".into());
        }
        // Evaluate the place once. The same qualified backing is the key of
        // the current extent and the source of the checked address argument.
        let pointer = memory::place_pointer(context, state, place, project)?;
        let count = memory::capture_local_runtime_extent(context, state, place, pointer, false)?
            .ok_or("missing current sealed widened array extent")?;
        let address = state.builder.build_ptr_to_int(pointer,
            memory::pointer_integer_type(context, state.target_layout), "address").map_err(builder_error)?;
        (address, count)
    } else {
        let Some((legacy, declaration, alias)) = plan.borrowed_extent_call else { return Ok(None) };
        if legacy != occurrence { return Ok(None); }
        let exact = match (arguments, alias) {
            ([ScalarExpression::CheckedAddress { place: place @ ScalarPlace::Name { .. }, .. }], None) =>
                nodes.lexical_place_declaration(place) == Some(declaration),
            ([argument @ ScalarExpression::Name { .. }], Some(alias)) =>
                nodes.lexical_expression_declaration(argument) == Some(alias),
            _ => false,
        };
        if !exact {
            return Err("sealed borrowed extent changed its checked argument identity".into());
        }
        let values = super::expression::emit_contextual_arguments(context, state, arguments, parameters, project)?;
        let [EmitValue::Basic(BasicValueEnum::IntValue(address))] = values.as_slice()
        else { return Err("missing sealed borrowed array address value".into()) };
        let count = state.aggregate.as_mut().and_then(|aggregate| aggregate.borrowed_extent.take())
            .ok_or("missing sealed borrowed array extent")?;
        (*address, count)
    };
    let function = *state.functions.get(symbol).ok_or("missing sealed borrowed array observer")?;
    let descriptor = borrowed_extent_type(context, state.target_layout);
    if function.get_linkage() != Linkage::Internal || function.get_type().get_param_types().as_slice() != [descriptor.into()]
        || function.get_type().get_return_type() != Some(context.i64_type().into())
    {
        return Err("sealed borrowed array extent call ABI changed".into());
    }
    let pair = state.builder.build_insert_value(descriptor.get_undef(), address, 0, "borrowed_array_argument_address").map_err(builder_error)?.into_struct_value();
    let pair = state.builder.build_insert_value(pair, count, 1, "borrowed_array_argument").map_err(builder_error)?.into_struct_value();
    let value = state.builder.build_call(function, &[pair.into()], "borrowed_extent").map_err(builder_error)?
        .try_as_basic_value().basic().ok_or("missing observed borrowed array extent")?;
    Ok(Some(EmitValue::Basic(value)))
}

pub(super) fn emit_array_descriptor_call<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    symbol: &str,
    arguments: &[ScalarExpression],
    values: &[EmitValue<'ctx>],
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(calls) = state.aggregate.as_ref().map(|aggregate| &aggregate.plan.array_descriptor_calls)
        .filter(|calls| !calls.is_empty()) else { return Ok(None) };
    let Some(ordinal) = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)) else {
        return Ok(None);
    };
    if !calls.contains(&ordinal) {
        return Ok(None);
    }
    let function = *state.functions.get(symbol).ok_or("missing checked array descriptor producer")?;
    let plan = state.aggregate.as_ref().expect("sealed array descriptor call").plan;
    let rebind = plan.array_descriptor_rebinds.get(&ordinal).map(|&(declaration, _)| declaration);
    let fresh = plan.widened_owning_receives.get(&ordinal).map(|&(declaration, _, _)| declaration);
    let owning = rebind.or(fresh);
    let args = if let Some(declaration) = owning {
        let [argument @ ScalarExpression::Name { name, .. }] = arguments else {
            return Err("checked array rebind has no exact lexical argument".into());
        };
        if state.body_call_nodes.and_then(|nodes| nodes.lexical_expression_declaration(argument)) != Some(declaration) {
            return Err("checked array rebind argument changed its lexical identity".into());
        }
        let [EmitValue::Basic(BasicValueEnum::PointerValue(base))] = values else {
            return Err("checked array rebind requires one backing pointer".into());
        };
        let (base, length) = if let Some(&count) = plan.fixed_copy_owning_arguments.get(&ordinal) {
            let (source, ty) = state.storage.get(name).cloned().ok_or("missing fixed Copy owning argument")?;
            let ScalarType::Array { element, length, .. } = &ty else {
                return Err("owning fixed Copy argument has no fixed storage".into());
            };
            if source != *base || *length != count || fresh.is_none() {
                return Err("owning fixed Copy argument changed the exact prepared source".into());
            }
            let count = context.i64_type().const_int(count, false);
            let backing = ownership::emit_runtime_array_allocation_with_domain(context, state, element, count, true)?;
            memory::copy_fixed_aggregate(context, state, backing, source, &ty)?;
            (backing, count.into())
        } else if fresh.is_some() || plan.widened_owning_roundtrip
            .is_some_and(|(call, _, owner)| call == ordinal && owner == declaration)
        {
            let Some(&(owner, LocalRuntimeExtent::WidenedLiteral { length, .. }, true)) = state.local_runtime_extents.get(base)
            else { return Err("widened roundtrip argument has no current extent".into()); };
            if owner != declaration || state.storage.get(name).is_none_or(|(backing, _)| backing != base)
                || state.runtime_array_owners.get(name) != Some(&true)
                || state.runtime_array_allocations.get(name) != Some(&true)
            { return Err("widened roundtrip argument changed its exact live owner".into()); }
            (*base, length.into())
        } else {
            let length_slot = *state.runtime_array_lengths.get(name).ok_or("missing checked array rebind extent slot")?;
            (*base, state.builder.build_load(context.i64_type(), length_slot, "array_argument_extent").map_err(builder_error)?)
        };
        let pair = state.builder.build_insert_value(array_descriptor_type(context).get_undef(), base,
            0, "array_argument_backing").map_err(builder_error)?.into_struct_value();
        let pair = state.builder.build_insert_value(pair, length, 1, "array_argument").map_err(builder_error)?
            .into_struct_value();
        vec![pair.into()]
    } else { values.iter().map(|value| match value {
        EmitValue::Basic(value) => Ok((*value).into()),
        _ => Err("unsupported checked array descriptor argument"),
    }).collect::<Result<Vec<BasicMetadataValueEnum<'ctx>>, _>>()? };
    if function.get_linkage() != Linkage::Internal || function.count_params() as usize != args.len()
        || function.get_type().get_return_type() != Some(array_descriptor_type(context).into())
        || (owning.is_some() && function.get_type().get_param_types().as_slice() != [array_descriptor_type(context).into()])
    {
        return Err("checked array descriptor transport differs from prepared call".into());
    }
    let value = state.builder.build_call(function, &args, "array_descriptor").map_err(builder_error)?
        .try_as_basic_value().basic().ok_or("missing checked array descriptor result")?;
    Ok(Some(EmitValue::Basic(value)))
}

pub(super) fn install_ordinary_array_descriptor<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan)
        .filter(|plan| !plan.ordinary_array_descriptor_receivers.is_empty()) else { return Ok(false); };
    let nodes = state.body_call_nodes.ok_or("missing ordinary returned array receipt")?;
    let Some(root) = nodes.expression_ordinal(&binding.value) else { return Ok(false); };
    let Some(&span) = plan.ordinary_array_descriptor_receivers.get(&root) else { return Ok(false); };
    let [receiver] = binding.receivers.as_slice() else { return Err("missing ordinary array receiver".into()); };
    if receiver.name_span != span || nodes.binding_position(binding).is_none()
        || !matches!(receiver.ty, ScalarType::RuntimeArray { .. })
        || binding.fallible_assignment.is_some() || binding.is_allocation
    { return Err("ordinary array receiver changed its prepared assignment boundary".into()); }
    let signed = memory::SignedRuntimeAddressDomain::for_closed_receiver(state.target_layout, &receiver.ty,
        plan.signed_ordinary_array_descriptor_receivers.get(&root))?;
    let (backing, extent) = unpack_widened_owning_descriptor(context, state, value)?;
    require_ordinary_array_backing(context, state, backing)?;
    install_runtime_array_binding_owner(state, &receiver.name, true, true, None);
    state.storage.insert(receiver.name.clone(), (backing, receiver.ty.clone()));
    state.local_runtime_extents.insert(backing, (span, LocalRuntimeExtent::OrdinaryDescriptor { length: extent, signed }, true));
    Ok(true)
}

pub(super) fn require_ordinary_array_backing<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    backing: PointerValue<'ctx>,
) -> Result<(), String> {
    let function = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or("missing ordinary array receiver block")?;
    let present = context.append_basic_block(function, "ordinary.array_present");
    let absent = context.append_basic_block(function, "ordinary.array_absent");
    let available = state.builder.build_is_not_null(backing, "ordinary.array_available").map_err(builder_error)?;
    state.builder.build_conditional_branch(available, present, absent).map_err(builder_error)?;
    state.builder.position_at_end(absent);
    super::expression::emit_terminal_system_panic(context, state)?;
    state.builder.position_at_end(present);
    Ok(())
}

pub(super) fn install_widened_owning_receiver<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan)
        .filter(|plan| !plan.widened_owning_receives.is_empty()) else { return Ok(false) };
    let nodes = state.body_call_nodes.ok_or("missing checked owning array receiver nodes")?;
    let Some(call) = nodes.expression_ordinal(&binding.value) else { return Ok(false); };
    let Some(&(source, receiver, None)) = plan.widened_owning_receives.get(&call) else { return Ok(false); };
    let ([receiver_slot], ScalarExpression::Call { arguments, .. }) =
        (binding.receivers.as_slice(), &binding.value)
    else { return Err("missing exact widened array receiving binding".into()); };
    let [argument @ ScalarExpression::Name { name, .. }] = arguments.as_slice()
    else { return Err("missing exact widened array source".into()); };
    if receiver_slot.name_span != receiver || nodes.binding_position(binding).is_none()
        || nodes.lexical_expression_declaration(argument) != Some(source)
    { return Err("widened owning receiver changed its exact lexical move".into()); }
    if plan.fixed_copy_owning_arguments.contains_key(&call) {
        let (base, count) = unpack_widened_owning_descriptor(context, state, value)?;
        install_runtime_array_binding_owner(state, &receiver_slot.name, true, true, None);
        state.storage.insert(receiver_slot.name.clone(), (base, receiver_slot.ty.clone()));
        state.local_runtime_extents.insert(base, (receiver, LocalRuntimeExtent::WidenedLiteral { length: count, signed: None }, true));
    } else {
        install_widened_owning_local(context, state, (name, source), (&receiver_slot.name, receiver, &receiver_slot.ty),
            false, value)?;
    }
    Ok(true)
}

pub(super) fn install_widened_owning_local<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    (source_name, source): (&str, wosy_syntax::ByteSpan),
    (target_name, receiver, target_ty): (&str, wosy_syntax::ByteSpan, &ScalarType),
    replace: bool,
    value: &EmitValue<'ctx>,
) -> Result<(), String> {
    if source_name == target_name
        || state.runtime_array_owners.get(source_name) != Some(&true)
        || state.runtime_array_allocations.get(source_name) != Some(&true)
    {
        return Err("widened owning install lost its active source".into());
    }
    let (previous, _) = state.storage.get(source_name).ok_or("missing original widened array backing")?;
    if !matches!(state.local_runtime_extents.get(previous),
        Some((declaration, LocalRuntimeExtent::WidenedLiteral { .. }, true)) if *declaration == source)
    { return Err("widened owning receiver lost its active source extent".into()); }
    let previous = *previous;
    let (base, count) = unpack_widened_owning_descriptor(context, state, value)?;
    if replace {
        let (target, _) = state.storage.get(target_name).ok_or("missing widened predecessor")?;
        if *target == previous || state.runtime_array_owners.get(target_name) != Some(&true)
            || state.runtime_array_allocations.get(target_name) != Some(&true)
            || !matches!(state.local_runtime_extents.get(target),
                Some((declaration, LocalRuntimeExtent::WidenedLiteral { .. }, true)) if *declaration == receiver)
        { return Err("widened owning install lost its distinct active target".into()); }
        let target = *target;
        ownership::consume_runtime_array_release(context, state,
            ownership::RuntimeArrayRelease { pointer: target.into(), live: None })?;
        state.local_runtime_extents.remove(&target);
    }
    install_runtime_array_binding_owner(state, target_name, true, true, Some(source_name));
    state.storage.insert(target_name.to_owned(), (base, target_ty.clone()));
    state.local_runtime_extents.remove(&previous);
    state.local_runtime_extents.insert(base, (receiver, LocalRuntimeExtent::WidenedLiteral { length: count, signed: None }, true));
    Ok(())
}

fn unpack_widened_owning_descriptor<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    value: &EmitValue<'ctx>,
) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), String> {
    let EmitValue::Basic(BasicValueEnum::StructValue(pair)) = value else {
        return Err("missing returned widened array descriptor".into());
    };
    if pair.get_type() != array_descriptor_type(context) {
        return Err("invalid returned widened array descriptor".into());
    }
    let base = state.builder.build_extract_value(*pair, 0, "array_descriptor_backing").map_err(builder_error)?.into_pointer_value();
    let count = state.builder.build_extract_value(*pair, 1, "array_descriptor_extent").map_err(builder_error)?.into_int_value();
    Ok((base, count))
}

pub(super) fn fallible_allocation_call_at(state: &EmitState<'_, '_>, expression: &ScalarExpression) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| {
        let calls = &aggregate.plan().fallible_allocating_calls;
        !calls.is_empty() && state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
            .is_some_and(|ordinal| calls.contains(&ordinal))
    })
}

pub(super) fn ordinary_fallible_allocation_call_at(state: &EmitState<'_, '_>, expression: &ScalarExpression) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| {
        let calls = &aggregate.plan().ordinary_fallible_allocating_calls;
        !calls.is_empty() && state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
            .is_some_and(|ordinal| calls.contains(&ordinal))
    })
}

pub(super) fn fallible_allocation_expression_at(state: &EmitState<'_, '_>, expression: &ScalarExpression) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| {
        let choices = &aggregate.plan().fallible_allocating_expressions;
        !choices.is_empty() && state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
            .is_some_and(|ordinal| choices.contains(&ordinal))
    })
}

pub(super) fn ordinary_fallible_allocation_expression_at(state: &EmitState<'_, '_>, expression: &ScalarExpression) -> bool {
    state.aggregate.as_ref().is_some_and(|aggregate| {
        let choices = &aggregate.plan().ordinary_fallible_allocating_expressions;
        !choices.is_empty() && state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
            .is_some_and(|ordinal| choices.contains(&ordinal))
    })
}

fn fallible_allocation_integer_type<'ctx>(
    context: &'ctx Context, payload: &ScalarType,
) -> Result<inkwell::types::IntType<'ctx>, String> {
    match payload {
        ScalarType::U64 => Ok(context.i64_type()),
        ScalarType::I32 => Ok(context.i32_type()),
        ScalarType::Bool => Ok(context.bool_type()),
        _ => Err("invalid sealed fallible allocation payload type".into()),
    }
}

pub(super) fn fallible_allocation_result_type<'ctx>(
    context: &'ctx Context, payload: &ScalarType,
) -> Result<inkwell::types::StructType<'ctx>, String> {
    let integer = fallible_allocation_integer_type(context, payload)?;
    Ok(context.struct_type(&[context.bool_type().into(), integer.into()], false))
}

pub(super) fn present_fallible_allocation_result<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    value: BasicValueEnum<'ctx>,
    payload: &ScalarType,
) -> Result<inkwell::values::StructValue<'ctx>, String> {
    let integer = fallible_allocation_integer_type(context, payload)?;
    if !matches!(value, BasicValueEnum::IntValue(value) if value.get_type() == integer) {
        return Err("invalid sealed fallible allocation result payload".into());
    }
    let pair = fallible_allocation_result_type(context, payload)?;
    let tagged = pair.const_named_struct(&[
        context.bool_type().const_all_ones().into(), integer.const_zero().into()
    ]);
    state.builder.build_insert_value(tagged, value, 1, "fallible.payload")
        .map_err(builder_error).map(|value| value.into_struct_value())
}

pub(super) fn definition_type<'ctx>(
    context: &'ctx Context,
    function: &ScalarFunction,
    cfg: &ScalarReferenceCfg,
    layout: ScalarTargetLayout,
    plan: Option<&ClosedBody>,
) -> Result<FunctionType<'ctx>, String> {
    if plan.is_some_and(|body| body.fallible_allocation_leaf || body.fallible_allocation_relay) {
        let payload = plan.and_then(|body| body.fallible_allocation_payload.as_ref())
            .ok_or("missing sealed fallible allocation payload type")?;
        let ScalarType::Callable { parameters, outputs } = &function.signature else {
            return Err("invalid sealed fallible allocation signature".into());
        };
        if parameters.as_slice() != [ScalarType::U64]
            || !matches!(outputs.outputs.as_slice(), [output] if &output.ty == payload)
        {
            return Err("invalid sealed fallible allocation signature".into());
        }
        return Ok(fallible_allocation_result_type(context, payload)?.fn_type(&[context.i64_type().into()], false));
    }
    if plan.is_some_and(|body| body.private_runtime_access_formal.is_some()) {
        let ScalarType::Callable { parameters, outputs } = &function.signature else {
            return Err("private runtime array reader is not callable".into());
        };
        if crate::scalar::private_runtime_write_signature(parameters, &outputs.outputs)
            && plan.is_some_and(|body| !body.private_runtime_writes.is_empty()
                && body.private_runtime_reads.is_empty() && body.private_runtime_read_forward.is_none())
        {
            return Ok(context.void_type().fn_type(&[borrowed_extent_type(context, layout).into(),
                context.i64_type().into(), basic_type(context, &parameters[2], layout)?.into()], false));
        }
        if !crate::scalar::private_runtime_read_signature(parameters, &outputs.outputs) {
            return Err("private runtime array accessor changed its prepared signature".into());
        }
        return Ok(basic_type(context, &outputs.outputs[0].ty, layout)?
            .fn_type(&[borrowed_extent_type(context, layout).into(), context.i64_type().into()], false));
    }
    if plan.is_some_and(|body| body.borrowed_extent_observer.is_some() || body.borrowed_extent_forward.is_some()) {
        let ScalarType::Callable { parameters, outputs } = &function.signature else {
            return Err("invalid checked borrowed extent declaration".into());
        };
        if !matches!(parameters.as_slice(), [ScalarType::CheckedReference {
            mutability: crate::scalar::ScalarReferenceMutability::Shared, inner,
        }] if matches!(inner.as_ref(), ScalarType::RuntimeArray { .. }))
            || !matches!(outputs.outputs.as_slice(), [crate::ScalarOutput { ty: ScalarType::U64, .. }])
        {
            return Err("invalid checked borrowed extent signature".into());
        }
        return Ok(context.i64_type().fn_type(&[borrowed_extent_type(context, layout).into()], false));
    }
    if plan.is_some_and(|body| body.array_descriptor_identity) {
        let ScalarType::Callable { parameters, outputs } = &function.signature else {
            return Err("invalid checked array identity declaration".into());
        };
        if !matches!(parameters.as_slice(), [ScalarType::RuntimeArray { .. }])
            || !matches!(outputs.outputs.as_slice(), [crate::ScalarOutput { ty: ScalarType::RuntimeArray { .. }, .. }])
        {
            return Err("invalid checked array identity signature".into());
        }
        let descriptor = array_descriptor_type(context);
        return Ok(descriptor.fn_type(&[descriptor.into()], false));
    }
    if let Some(plan) = plan.filter(|body| body.array_descriptor_return) {
        let ScalarType::Callable { parameters, outputs } = &function.signature else {
            return Err("invalid array descriptor declaration".into());
        };
        let forward = plan.guarded_array_descriptor_forward.is_some();
        let choice = plan.guarded_array_descriptor_producer_choice.is_some();
        if (forward && choice)
            || ((forward || choice) && (parameters.as_slice() != [ScalarType::U64] || plan.literal_array.is_some()))
            || (!(forward || choice) && !matches!(plan.literal_array, Some(ClosedLiteralArray::FreshLeaf { .. })))
            || !matches!(outputs.outputs.as_slice(), [crate::ScalarOutput { ty: ScalarType::RuntimeArray { .. }, .. }])
            || !dynamic_return_outputs(function.span, cfg).is_empty()
        {
            return Err("invalid sealed array descriptor producer".into());
        }
        let args = parameters.iter().map(|ty| basic_type(context, ty, layout).map(Into::into))
            .collect::<Result<Vec<BasicMetadataTypeEnum<'ctx>>, _>>()?;
        return Ok(array_descriptor_type(context).fn_type(&args, false));
    }
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
        if !plan.is_some_and(|body| !body.inputs.is_empty() || body.checked_backing_producer.is_some()
            || body.checked_backing_wrapper.is_some()) {
            return Ok(original);
        }
        let mut parameters = original.get_param_types().into_iter().map(Into::into)
            .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
        let sidecars = plan.map_or(0, |body| body.inputs.len())
            + usize::from(plan.is_some_and(|body| body.checked_backing_producer.is_some()
                || body.checked_backing_wrapper.is_some()));
        let sidecar = BasicMetadataTypeEnum::from(context.ptr_type(AddressSpace::default()));
        parameters.extend(std::iter::repeat_n(sidecar, sidecars));
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
    if let Some(plan) = plan.filter(|plan| plan.multi_return_width() != 0) {
        if plan.multi_return_width() != outputs.outputs.len() || !plan.inputs.is_empty()
            || !outputs.outputs.iter().all(|output| matches!(output.ty, ScalarType::Struct(_)))
        {
            return Err("invalid ordered owning return declaration".into());
        }
        let pointer = context.ptr_type(AddressSpace::default());
        let arity = plan.multi_return_width();
        let mut arguments = parameters.iter().map(|ty| basic_type(context, ty, layout).map(Into::into))
            .collect::<Result<Vec<BasicMetadataTypeEnum<'ctx>>, _>>()?;
        arguments.extend(std::iter::repeat_n(BasicMetadataTypeEnum::from(pointer), arity));
        let returned = context.struct_type(&vec![pointer.into(); arity], false);
        return Ok(returned.fn_type(&arguments, false));
    }
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
    let origin = if matches!(plan, ClosedLiteralArray::FreshLeaf { .. })
        && crate::scalar::widened_local_literal(binding).is_some()
    {
        Some(&binding.value)
    } else {
        binding.receivers.first().and_then(|receiver| receiver.allocation_length.as_ref())
    };
    let actual_extent = origin.and_then(|expression|
        state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
    if state.body_call_nodes.and_then(|nodes| nodes.binding_position(binding)) != Some(receipt.allocation)
        || actual_extent != Some(occurrence)
        || !extent_matches
        || storage.is_some() != matches!(plan, ClosedLiteralArray::Replacement { .. })
        || !matches!(aggregate.literal_array_receipt, LiteralArrayReceipt::Awaiting)
    {
        return Err("checked literal array allocation does not match its receipt".into());
    }
    if aggregate.plan.array_descriptor_return {
        aggregate.array_descriptor_extent = Some(length);
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
    if aggregate.plan.conditional_inputs.is_empty() && aggregate.plan.input_exhaustive.is_none() {
        return Ok(());
    }
    let expression = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))
        .ok_or_else(|| "missing conditional aggregate receipt occurrence".to_owned())?;
    if aggregate.plan.input_exhaustive == Some(expression) {
        let formal = aggregate.plan.inputs.keys().next().ok_or("missing exhaustive formal plan")?;
        if aggregate.plan.inputs.len() != 1 || aggregate.plan.input_release_guard.is_some()
            || aggregate.inputs.remove(formal).is_none()
        {
            return Err("exhaustive formal receipt identity mismatch".into());
        }
    }
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
        aggregate.receipts.remove(&(*receipt, None))
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

pub(super) fn authenticate_replacement_index_storage<'ctx>(
    state: &EmitState<'ctx, '_>,
    backing_slot: PointerValue<'ctx>,
    extent_slot: PointerValue<'ctx>,
) -> Result<(), String> {
    let aggregate = state.aggregate.as_ref().ok_or("missing closed replacement index plan")?;
    if matches!(aggregate.plan.literal_array, Some(ClosedLiteralArray::Replacement { unit: Some(_), .. }))
        && !matches!(aggregate.literal_array_receipt,
            LiteralArrayReceipt::Joined(pair) if pair == (backing_slot, extent_slot))
    {
        return Err("returned array index requires the completed backing/extent join".into());
    }
    Ok(())
}

pub(super) fn check_replacement_extent_observation(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
) -> Result<(), String> {
    let Some(aggregate) = &state.aggregate else { return Ok(()) };
    let Some(expected) = aggregate.plan.replacement_extent_observation else { return Ok(()) };
    if state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)) == Some(expected)
        && if aggregate.plan.array_descriptor_rebinds.is_empty() {
            !matches!(aggregate.literal_array_receipt, LiteralArrayReceipt::Fresh { joined_storage: None, .. })
        } else { !aggregate.array_descriptor_rebound }
    {
        return Err("returned extent observation must follow its sealed replacement".into());
    }
    Ok(())
}

pub(super) fn install_literal_array_replacement<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    call: usize,
    name: &str,
    value: &EmitValue<'ctx>,
) -> Result<(), String> {
    let Some(aggregate) = state.aggregate.as_mut() else { return Err("missing checked literal array plan".into()) };
    if aggregate.plan.array_descriptor_rebinds.contains_key(&call) {
        if std::mem::replace(&mut aggregate.array_descriptor_rebound, true) {
            return Err("checked array descriptor already rebound".into());
        }
        let EmitValue::Basic(BasicValueEnum::StructValue(pair)) = value else {
            return Err("missing returned checked array descriptor".into());
        };
        if pair.get_type() != array_descriptor_type(context) {
            return Err("invalid returned checked array descriptor".into());
        }
        if let Some((_, _, owner)) = aggregate.plan.widened_owning_roundtrip.filter(|(ordinal, _, _)| *ordinal == call) {
            let (previous, ty) = state.storage.get(name).cloned().ok_or("missing widened roundtrip backing")?;
            if !matches!(state.local_runtime_extents.get(&previous),
                Some((declaration, LocalRuntimeExtent::WidenedLiteral { .. }, true)) if *declaration == owner)
            { return Err("widened roundtrip installation changed its owner".into()); }
            let base = state.builder.build_extract_value(*pair, 0, "array_descriptor_backing").map_err(builder_error)?.into_pointer_value();
            let count = state.builder.build_extract_value(*pair, 1, "array_descriptor_extent").map_err(builder_error)?.into_int_value();
            state.local_runtime_extents.remove(&previous);
            state.local_runtime_extents.insert(base, (owner, LocalRuntimeExtent::WidenedLiteral { length: count, signed: None }, true));
            state.storage.insert(name.to_owned(), (base, ty));
            return Ok(());
        }
        let (backing, _) = state.storage.get(name).ok_or("missing checked array rebind backing slot")?;
        let length = *state.runtime_array_lengths.get(name).ok_or("missing checked array rebind length slot")?;
        let base = state.builder.build_extract_value(*pair, 0, "array_descriptor_backing").map_err(builder_error)?;
        let count = state.builder.build_extract_value(*pair, 1, "array_descriptor_extent").map_err(builder_error)?;
        state.builder.build_store(*backing, base).map_err(builder_error)?;
        state.builder.build_store(length, count).map_err(builder_error)?;
        return Ok(());
    }
    let Some(ClosedLiteralArray::Replacement { unit, .. }) = aggregate.plan.literal_array
        else { return Err("missing checked literal array replacement".into()) };
    let Some((_, extent, expected_arm)) = literal_array_write(aggregate.plan.literal_array, call)
        else { return Err("checked array writer does not match its call".into()) };
    let descriptor = aggregate.plan.array_descriptor_calls.contains(&call);
    let (fresh, transported_extent) = match (descriptor, value) {
        (false, EmitValue::Basic(BasicValueEnum::PointerValue(fresh))) => (*fresh, None),
        (true, EmitValue::Basic(BasicValueEnum::StructValue(pair))) if pair.get_type() == array_descriptor_type(context) => {
            let fresh = state.builder.build_extract_value(*pair, 0, "array_descriptor_backing")
                .map_err(builder_error)?.into_pointer_value();
            let length = state.builder.build_extract_value(*pair, 1, "array_descriptor_extent")
                .map_err(builder_error)?.into_int_value();
            (fresh, Some(length))
        }
        _ => return Err("checked literal array replacement has no matching backing transport".into()),
    };
    let LiteralArrayReceipt::Original {
        base: original, storage: Some((destination, length_slot)), argument_extent, in_writer,
    } =
        std::mem::replace(&mut aggregate.literal_array_receipt, LiteralArrayReceipt::Finished)
    else { return Err("checked literal array replacement has no original receipt".into()) };
    if in_writer != expected_arm {
        return Err("checked array writer does not match its branch and call".into());
    }
    let length = match (transported_extent, extent, argument_extent) {
        (Some(length), _, None) => length,
        (None, ReplacementArrayExtent::Literal(elements), None) => context.i64_type().const_int(elements, false),
        (None, ReplacementArrayExtent::Argument { .. }, Some(length)) => length,
        _ => return Err("checked array replacement has no matching extent".into()),
    };
    release_literal_array_base(context, state, original)?;
    state.builder.build_store(destination, fresh).map_err(builder_error)?;
    state.builder.build_store(length_slot, length).map_err(builder_error)?;
    state.aggregate.as_mut().expect("checked literal array replacement").literal_array_receipt =
        LiteralArrayReceipt::Fresh { base: fresh, joined_storage: unit.map(|_| (destination, length_slot)) };
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
    if !aggregate.plan.multi_returns.is_empty() {
        if let Some(position) = binding_position() {
            if let Some(slot) = aggregate.multi_allocations.get_mut(&position) {
                if slot.replace((base, name.to_owned())).is_some() {
                    return Err("duplicate ordered original allocation".into());
                }
            }
        }
    }
    let registered = match aggregate.plan.returning {
        Some(ClosedReturn::Allocation { block, item }) => binding_position() == Some((block, item)),
        Some(ClosedReturn::Structured { .. }) => binding_position().is_some_and(|(block, item)|
            aggregate.plan.local_result_arms.get(&block).is_some_and(|&(expected, _)| expected == item)),
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

fn commit_local_choice<'ctx>(
    aggregate: &mut AggregateEmitter<'ctx, '_>, conditional: usize, base: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    if aggregate.local_choice_visited < 2
        || aggregate.plan.local_choice_arms.get(aggregate.local_choice_committed)
            != Some(&(conditional, aggregate.local_choice_visited))
    {
        return Err("invalid local conditional owner commit".into());
    }
    let descriptor = aggregate.local_choice_destination.take().ok_or("missing local conditional descriptor")?;
    if aggregate.receipts.insert((conditional, None), base).is_some() {
        return Err("duplicate local conditional owner receipt".into());
    }
    aggregate.local_choice_committed += 1;
    aggregate.local_choice_visited = 0;
    Ok(descriptor)
}

pub(super) fn store_received_binding<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    if let Some(aggregate) = state.aggregate.as_mut() {
        if !aggregate.plan.local_choice_bindings.is_empty() {
            let nodes = state.body_call_nodes.ok_or("missing local conditional ownership occurrences")?;
            if let Some(&conditional) = nodes.binding_position(binding)
                .and_then(|position| aggregate.plan.local_choice_bindings.get(&position)) {
                let [receiver] = binding.receivers.as_slice() else { return Err("invalid local conditional receiver".into()) };
                let EmitValue::Basic(BasicValueEnum::PointerValue(base)) = value else { return Err("missing selected original backing".into()) };
                if nodes.expression_ordinal(&binding.value) != Some(conditional)
                    || !matches!(receiver.ty, ScalarType::Struct(_))
                {
                    return Err("changed local conditional owner binding".into());
                }
                let descriptor = commit_local_choice(aggregate, conditional, *base)?;
                state.storage.insert(receiver.name.clone(), (descriptor, receiver.ty.clone()));
                return Ok(true);
            }
        }
        if let Some((block, item, conditional)) = aggregate.plan.structured_result_binding {
            let nodes = state.body_call_nodes.ok_or("missing structured result binding occurrences")?;
            if nodes.binding_position(binding) == Some((block, item)) {
                let [receiver] = binding.receivers.as_slice() else { return Err("invalid structured result receiver".into()) };
                let EmitValue::Basic(BasicValueEnum::PointerValue(base)) = value else { return Err("missing joined result backing at binding".into()) };
                if nodes.expression_ordinal(&binding.value) != Some(conditional) || !matches!(receiver.ty, ScalarType::Struct(_)) {
                    return Err("changed structured result binding".into());
                }
                let destination = aggregate.destination.ok_or("missing structured result destination at binding")?;
                aggregate.set_base(*base)?;
                // The conditional arms already wrote the ordinary descriptor.
                // Bind its storage, not the separate original backing SSA token.
                state.storage.insert(receiver.name.clone(), (destination, receiver.ty.clone()));
                return Ok(true);
            }
        }
    }
    if let Some((call, receive)) = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(&binding.value))
        .and_then(|call| state.aggregate.as_ref()?.plan.multi_receives.get(&call).map(|receive| (call, receive)))
    {
        if !binding.receivers.iter().map(|receiver| receiver.name_span).eq(receive.retained.iter().copied()) {
            return Err("ordered owning call has changed its prepared receivers".into());
        }
        for (lane, receiver) in binding.receivers.iter().enumerate() {
            let EmitValue::Basic(BasicValueEnum::PointerValue(destination)) = extract_output(state, value, lane)?
                else { return Err("missing ordered descriptor destination".into()) };
            state.storage.insert(receiver.name.clone(), (destination, receiver.ty.clone()));
        }
        if receive.retained.len() != receive.width {
            declare_core_runtime(context, state.module);
            let free = state.module.get_function(CORE_FREE_SYMBOL).ok_or("missing ordered output discard release")?;
            let aggregate = state.aggregate.as_mut().expect("ordered output discard frame");
            // Retained slots are now installed. Spend just the unbound suffix
            // at this checked binding boundary, never at a later lexical exit.
            for lane in (receive.retained.len()..receive.width).rev() {
                let original = aggregate.receipts.remove(&(call, Some(lane)))
                    .ok_or("discarded owning output lacks its original backing")?;
                state.builder.build_call(free, &[original.into()], "owning_output_discard")
                    .map_err(builder_error)?;
            }
        }
        return Ok(true);
    }
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

pub(super) fn requires_scoped_function_emission(plan: Option<&ClosedBody>) -> bool {
    plan.is_some_and(|plan| plan.fallible_allocation_leaf || plan.fallible_allocation_relay || !plan.inputs.is_empty() || plan.ordinary_result || !plan.ordinary_calls.is_empty()
        || !plan.multi_receives.is_empty() || plan.multi_return_width() != 0
        || plan.array_descriptor_observer.is_some() || plan.guarded_array_descriptor_producer_choice.is_some()
        || matches!(plan.returning, Some(ClosedReturn::Structured { .. })))
}

pub(super) fn extract_observing_array_formal<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    parameter: BasicValueEnum<'ctx>,
) -> Result<(BasicValueEnum<'ctx>, inkwell::values::IntValue<'ctx>), String> {
    let BasicValueEnum::StructValue(parameter) = parameter else {
        return Err("missing checked owning array formal".into());
    };
    if parameter.get_type() != array_descriptor_type(context) {
        return Err("invalid checked owning array formal".into());
    }
    let backing = builder.build_extract_value(parameter, 0, "array_formal_backing").map_err(builder_error)?;
    let extent = builder.build_extract_value(parameter, 1, "array_formal_extent").map_err(builder_error)?;
    Ok((backing, extent.into_int_value()))
}

#[derive(Clone, Copy)]
pub(super) enum ArrayDescriptorForward {
    Owned,
    Borrowed,
    PrivateSharedIndex,
    GuardedProducer,
}

pub(super) fn emit_array_descriptor_forward<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    definition: FunctionValue<'ctx>,
    function: &ScalarFunction,
    call: usize,
    transport: ArrayDescriptorForward,
) -> Result<(), String> {
    let Some((expression, _)) = function.body.final_expressions().next() else {
        return Err("missing checked array forwarding expression".into());
    };
    if state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)) != Some(call) {
        return Err("checked array forwarding occurrence changed".into());
    }
    let selected = state.prepared_call(expression)?.ok_or("missing qualified array forwarding target")?;
    let symbol = selected.symbol().ok_or("checked array forwarding lost its declaration")?;
    let target = *state.functions.get(symbol).ok_or("missing checked array forwarding target")?;
    let owned = array_descriptor_type(context);
    let (argument, result): (BasicTypeEnum<'ctx>, BasicTypeEnum<'ctx>) = match transport {
        ArrayDescriptorForward::Owned => (owned.into(), owned.into()),
        ArrayDescriptorForward::Borrowed => (borrowed_extent_type(context, state.target_layout).into(), context.i64_type().into()),
        ArrayDescriptorForward::PrivateSharedIndex => {
            let ScalarType::Callable { parameters, outputs } = &function.signature else {
                return Err("private shared relay is not a callable".into());
            };
            if !crate::scalar::private_runtime_read_signature(parameters, &outputs.outputs) {
                return Err("private shared relay changed its exact signature".into());
            }
            (borrowed_extent_type(context, state.target_layout).into(), basic_type(context, &outputs.outputs[0].ty, state.target_layout)?)
        }
        ArrayDescriptorForward::GuardedProducer => {
            if !state.aggregate.as_ref().is_some_and(|aggregate| aggregate.plan.array_descriptor_calls.contains(&call)) {
                return Err("guarded producer forwarding lost its descriptor call receipt".into());
            }
            (context.i64_type().into(), owned.into())
        }
    };
    let parameter = definition.get_first_param().ok_or("missing checked array forwarding parameter")?;
    let private_index = matches!(transport, ArrayDescriptorForward::PrivateSharedIndex);
    if private_index && state.aggregate.as_ref().is_some_and(|aggregate| aggregate.plan.private_runtime_portable_formal)
        && !state.project_names.is_some_and(|names| names.private_runtime_portable_symbols.contains(symbol))
    {
        return Err("portable private runtime relay lost its closed target".into());
    }
    let target_type = target.get_type();
    let exact_parameters = if private_index {
        target_type.get_param_types().as_slice() == [argument.into(), context.i64_type().into()]
    } else { target_type.get_param_types().as_slice() == [argument.into()] };
    if parameter.get_type() != argument || target.get_linkage() != Linkage::Internal
        || !exact_parameters || target_type.get_return_type() != Some(result)
    {
        return Err("checked array forwarding types differ from the paired plan".into());
    }
    let singleton = [parameter.into()];
    let indexed;
    let arguments = if private_index {
        let selector = definition.get_nth_param(1).ok_or("private shared relay lacks its original selector")?;
        if selector.get_type() != context.i64_type().into() {
            return Err("private shared relay changed its full-width selector".into());
        }
        indexed = [parameter.into(), selector.into()];
        indexed.as_slice()
    } else { singleton.as_slice() };
    let pair = state.builder.build_call(target, arguments, "array_forward")
        .map_err(builder_error)?.try_as_basic_value().basic().ok_or("missing checked forwarded array result")?;
    state.builder.build_return(Some(&pair)).map_err(builder_error)?;
    Ok(())
}

pub(super) fn emit_array_descriptor_identity<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    definition: FunctionValue<'ctx>,
) -> Result<(), String> {
    let Some(BasicValueEnum::StructValue(parameter)) = definition.get_first_param() else {
        return Err("missing checked array identity parameter".into());
    };
    if parameter.get_type() != array_descriptor_type(context) {
        return Err("invalid checked array identity parameter".into());
    }
    builder.build_return(Some(&parameter)).map_err(builder_error)?;
    Ok(())
}

pub(super) fn emit_identity_relay<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    definition: FunctionValue<'ctx>,
    owner_slot: usize,
) -> Result<(), String> {
    let parameter = definition.get_nth_param(owner_slot as u32)
        .ok_or_else(|| "missing same-owner relay parameter".to_owned())?;
    if !matches!(parameter, BasicValueEnum::PointerValue(_))
        || !matches!(definition.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::PointerType(_)))
    {
        return Err("same-owner relay changed its ordinary descriptor ABI".into());
    }
    builder.build_return(Some(&parameter)).map_err(builder_error)?;
    Ok(())
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
    if aggregate.plan.multi_receives.contains_key(&ordinal) {
        return emit_multiple_owner_call(context, state, expression, arguments, project, ordinal).map(Some);
    }
    let forwards = aggregate.plan.checked_backing_wrapper == Some(ordinal);
    if forwards || aggregate.plan.checked_backing_consumer.is_some_and(|(_, call)| ordinal == call) {
        return emit_checked_backing_call(context, state, expression, arguments, project, forwards).map(Some);
    }
    if let Some(value) = try_emit_witnessed_ordinary_call(context, state, expression, arguments, project, ordinal)? {
        return Ok(Some(value));
    }
    let aggregate = state.aggregate.as_ref().expect("closed aggregate call frame");
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
        let base = aggregate.receipts.remove(&(previous, None))
            .ok_or_else(|| "same-owner recommit lost its original backing".to_owned())?;
        if aggregate.receipts.insert((ordinal, None), base).is_some() {
            return Err("duplicate same-owner recommit receipt".into());
        }
        return Ok(Some(value));
    }
    let action = aggregate.plan.calls.get(&ordinal).copied().map(CallDestination::Body);
    let input = aggregate.plan.input_calls.get(&ordinal).map(|inputs| CallInput::Body(inputs.as_slice()));
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

fn emit_multiple_owner_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
    ordinal: usize,
) -> Result<EmitValue<'ctx>, String> {
    let symbol = state.direct_aggregate_symbol(expression, None)?;
    let function = *state.functions.get(symbol).ok_or("missing ordered owning call definition")?;
    let Some(ScalarType::Callable { parameters, outputs }) = state.signatures.get(symbol) else {
        return Err("missing ordered owning call signature".into());
    };
    let width = state.aggregate.as_ref().expect("owning call frame").plan.multi_receives[&ordinal].width;
    if parameters.len() != arguments.len() || width != outputs.outputs.len()
        || function.get_linkage() != Linkage::Internal || function.count_params() as usize != parameters.len() + width
    {
        return Err("ordered owning call differs from its private convention".into());
    }
    let mut operands = Vec::with_capacity(parameters.len() + width);
    for (argument, parameter) in arguments.iter().zip(parameters) {
        let value = match project {
            Some((module, modules)) => emit_project_typed_expression(context, state, argument, parameter, module, modules)?,
            None => emit_typed_expression(context, state, argument, parameter)?,
        };
        operands.push(take_basic(value)?.into());
    }
    let forwarded = state.aggregate.as_ref().filter(|aggregate|
        aggregate.plan.multi_forward.as_ref().is_some_and(|(call, _)| *call == ordinal));
    let forwarding = forwarded.is_some();
    let mut values = Vec::with_capacity(width);
    if let Some(aggregate) = forwarded {
        if aggregate.multi_destinations.len() != width {
            return Err("missing exact owning relay destinations".into());
        }
        for &destination in &aggregate.multi_destinations {
            operands.push(destination.into());
            values.push(EmitValue::Basic(destination.into()));
        }
    } else {
        for output in &outputs.outputs {
            let destination = entry_storage_alloca(context, state, &output.ty, "owning_output_destination")?;
            operands.push(destination.into());
            values.push(EmitValue::Basic(destination.into()));
        }
    }
    let returned = state.builder.build_call(function, &operands, "owning_output_bases")
        .map_err(builder_error)?.try_as_basic_value().basic().ok_or("missing ordered backing result")?.into_struct_value();
    let aggregate = state.aggregate.as_mut().expect("owning call frame");
    if forwarding {
        if aggregate.mixed_return.replace(returned).is_some() {
            return Err("ordered owning relay emitted more than one transport".into());
        }
        return build_aggregate(context, state, outputs, values);
    }
    for lane in 0..width {
        let index = u32::try_from(lane).map_err(|_| "owning call lane overflow")?;
        let base = state.builder.build_extract_value(returned, index, "owning_output_base").map_err(builder_error)?
            .into_pointer_value();
        if aggregate.receipts.insert((ordinal, Some(lane)), base).is_some() {
            return Err("duplicate ordered call receipt".into());
        }
    }
    build_aggregate(context, state, outputs, values)
}

fn emit_selected_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    arguments: &[ScalarExpression],
    project: Option<(&ScalarModule, &[&ScalarModule])>,
    ordinal: usize,
    action: Option<CallDestination<'ctx>>,
    input: Option<CallInput<'module>>,
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
        Some(CallDestination::Body(ClosedCall::LocalChoiceArm(conditional))) => {
            let [output] = outputs.outputs.as_slice() else { return Err("invalid local conditional output".into()) };
            if !matches!(output.ty, ScalarType::Struct(_)) {
                return Err("invalid local conditional descriptor type".into());
            }
            let aggregate = state.aggregate.as_ref().expect("local conditional owner frame");
            if !aggregate.plan.local_choice_arms.get(aggregate.local_choice_committed)
                .is_some_and(|&(planned, arms)| planned == conditional && aggregate.local_choice_visited < arms)
            {
                return Err("local conditional arm changed its planned choice".into());
            }
            if let Some(storage) = aggregate.local_choice_destination {
                Some(storage)
            } else {
                let storage = entry_storage_alloca(context, state, &output.ty, "local_choice_descriptor")?;
                state.aggregate.as_mut().expect("local conditional owner frame").local_choice_destination = Some(storage);
                Some(storage)
            }
        }
        Some(CallDestination::Body(ClosedCall::Forward | ClosedCall::ResultArm)) => Some(state.aggregate.as_ref().and_then(|aggregate| aggregate.destination)
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
    let local_temporary = state.aggregate.as_ref().and_then(|aggregate| aggregate.plan.local_choice_immediate)
        .and_then(|(consumer, conditional)| (consumer == ordinal).then_some(conditional));
    if let Some(conditional) = local_temporary {
        let [argument] = arguments else { return Err("invalid local conditional temporary arguments".into()) };
        if !matches!(parameters.as_slice(), [ScalarType::Struct(_)])
            || state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(argument)) != Some(conditional)
            || !matches!(input, Some(CallInput::Body(inputs)) if inputs == [(0, ClosedInput::Transfer(conditional))])
        {
            return Err("changed local conditional temporary consumer".into());
        }
    }
    let mut operands = Vec::new();
    for (argument, parameter) in arguments.iter().zip(parameters) {
        let emitted = match project {
            Some((module, modules)) => {
                emit_project_typed_expression(context, state, argument, parameter, module, modules)?
            }
            None => emit_typed_expression(context, state, argument, parameter)?,
        };
        let operand = take_basic(emitted)?;
        let operand = if let Some(conditional) = local_temporary {
            let BasicValueEnum::PointerValue(base) = operand else { return Err("missing local temporary backing".into()) };
            let aggregate = state.aggregate.as_mut().expect("local conditional temporary frame");
            commit_local_choice(aggregate, conditional, base)?.into()
        } else {
            operand
        };
        operands.push(BasicMetadataValueEnum::from(operand));
    }
    if immediate {
        state.initializer_aggregate.as_mut().expect("closed initializer immediate context").immediate_consumer = None;
    }
    if let Some(destination) = destination {
        operands.push(destination.into());
    }
    let owner_slots = super::expression::stage_checked_return_sidecars(context, state, owner_outputs, &mut operands)?;
    match input {
        Some(CallInput::Body(inputs)) => {
            let aggregate = state.aggregate.as_mut().expect("closed aggregate input context");
            for &(slot, input) in inputs {
                if slot >= arguments.len() {
                    return Err("closed aggregate input slot exceeds call arity".into());
                }
                let base = match input {
                    ClosedInput::Transfer(receipt) => {
                        if aggregate.plan.deferred_call_inputs.get(&ordinal) == Some(&receipt) {
                            aggregate.receipts.get(&(receipt, None)).copied()
                                .ok_or_else(|| "missing deferred owner receipt at committed input call".to_owned())?
                        } else {
                            aggregate.receipts.remove(&(receipt, None))
                                .ok_or_else(|| "missing owner receipt at committed input call".to_owned())?
                        }
                    }
                    ClosedInput::ForwardFormal(source) => aggregate.inputs.remove(&source)
                        .ok_or_else(|| "missing original formal base at owner transfer".to_owned())?,
                    ClosedInput::ForwardFormalDeferred(source) => {
                        if aggregate.plan.input_release_guard.is_none() && aggregate.plan.input_exhaustive.is_none() {
                            return Err("conditional formal owner transfer identity mismatch".to_owned());
                        }
                        aggregate.inputs.get(&source).copied()
                            .ok_or_else(|| "missing deferred formal base at owner transfer".to_owned())?
                    }
                    ClosedInput::Plain => context.ptr_type(AddressSpace::default()).const_null(),
                };
                operands.push(base.into());
            }
        }
        Some(CallInput::Initializer(input)) => {
            let frame = state.initializer_aggregate.as_mut().expect("closed initializer input frame");
            frame.visit(ordinal)?;
            let base = match (input, frame.receipt.take()) {
                (ClosedInitializerInput::Consume(expected) | ClosedInitializerInput::Immediate(expected),
                    Some((actual, base))) if expected == actual => base,
                (ClosedInitializerInput::Plain, None) => context.ptr_type(AddressSpace::default()).const_null(),
                _ => return Err("closed initializer original base is not available at its paired input".into()),
            };
            operands.push(base.into());
        }
        None => {}
    }
    if operands.len() != function.count_params() as usize {
        return Err("closed aggregate argument sidecars differ from definition".into());
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
    if matches!(action, Some(CallDestination::Body(ClosedCall::LocalChoiceArm(_)))) {
        let aggregate = state.aggregate.as_mut().expect("local conditional owner frame");
        if mixed.is_some()
            || !matches!(transport, BasicValueEnum::PointerValue(_))
        {
            return Err("invalid local conditional owner transfer".into());
        }
        aggregate.local_choice_visited += 1;
        return Ok(EmitValue::Basic(transport));
    }
    if matches!(action, Some(CallDestination::Body(ClosedCall::ResultArm))) {
        let aggregate = state.aggregate.as_mut().expect("structured aggregate arm frame");
        if !matches!(aggregate.plan.returning, Some(ClosedReturn::Structured { .. }))
            || mixed.is_some() || !matches!(transport, BasicValueEnum::PointerValue(_))
        {
            return Err("structured aggregate result arm changed its ownership ABI".into());
        }
        aggregate.structured_arms_visited += 1;
        // Only the typed final-result tree sees this SSA value. The callee
        // has already written the ordinary descriptor at the shared destination.
        return Ok(EmitValue::Basic(transport));
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
                ClosedCall::ResultArm | ClosedCall::LocalChoiceArm(_) => unreachable!("structured arms return their base to the result join"),
                ClosedCall::Receive => {
                    if aggregate.receipts.insert((ordinal, None), base).is_some() {
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
            (Some(_), LiteralArrayReceipt::Joined((storage, _))) => state.builder
                .build_load(context.ptr_type(AddressSpace::default()), storage, "joined_array_owner")
                .map_err(builder_error)?.into_pointer_value(),
            _ => return Err("checked literal array exit has no matching receipt".into()),
        };
        return release_literal_array_base(context, state, backing);
    }
    let releases = aggregate.plan.releases.get(&exit);
    let release_inputs = aggregate.plan.input_releases.get(&exit);
    if releases.is_none() && release_inputs.is_none() {
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
            .remove(&(*ordinal, planned.output_lane))
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
    for slot in release_inputs.into_iter().flatten().rev() {
        let base = aggregate.inputs.remove(slot).ok_or_else(|| "missing original formal base at owner exit".to_owned())?;
        let mixed = *aggregate.plan.inputs.get(slot).ok_or("formal owner release identity mismatch")?;
        let active = if let Some(guard) = aggregate.plan.input_release_guard {
            Some(aggregate.receipt_guards.get(&guard).copied()
                .ok_or_else(|| "missing guarded formal receipt at scope cleanup".to_owned())?)
        } else if mixed {
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

pub(super) fn holds_registered_return(state: &EmitState<'_, '_>, output: &ScalarExpression) -> bool {
    let Some(aggregate) = state.aggregate.as_ref() else { return false };
    if aggregate.registered_allocation.is_some() {
        return true;
    }
    if let Some((_, outputs)) = &aggregate.plan.multi_forward {
        return aggregate.mixed_return.is_some() && state.body_call_nodes
            .and_then(|nodes| nodes.expression_ordinal(output))
            .is_some_and(|occurrence| outputs.binary_search(&occurrence).is_ok());
    }
    let lanes = &aggregate.plan.multi_returns;
    if lanes.is_empty() {
        return false;
    }
    let Some(occurrence) = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(output))
        else { return false };
    let Ok(lane) = lanes.binary_search_by_key(&occurrence, |&(_, expression)| expression)
        else { return false };
    // Leave the original live allocation for the exact ordered commit. No
    // inference-based transfer for a different or as-yet-unregistered output.
    aggregate.multi_allocations.get(&lanes[lane].0).is_some_and(Option::is_some)
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

pub(super) fn commit_named_result_arm<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>, block: &ScalarBlock, result: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let Some(aggregate) = state.aggregate.as_mut() else { return Ok(result) };
    if !aggregate.plan.local_result_arms.is_empty() {
        let nodes = state.body_call_nodes.ok_or("missing structured local result occurrences")?;
        if let Some(&(_, output)) = nodes.block_ordinal(block)
            .and_then(|ordinal| aggregate.plan.local_result_arms.get(&ordinal))
        {
            let Some((expression, _)) = block.final_expressions().next() else { return Err("missing local result terminal".into()) };
            let ScalarExpression::Name { name, .. } = expression else { return Err("invalid local result terminal".into()) };
            if nodes.expression_ordinal(expression) != Some(output) { return Err("changed local result terminal".into()) }
            let (_, ty) = state.storage.get(name).ok_or("missing local result descriptor storage")?;
            let ty = ty.clone();
            let destination = aggregate.destination.ok_or("missing structured local result destination")?;
            let allocation = aggregate.registered_allocation.take().ok_or("missing structured local allocation")?;
            let base = aggregate.base.take().filter(|_| state.runtime_array_owners.get(&allocation) == Some(&true))
                .ok_or("missing live structured result backing")?;
            state.runtime_array_owners.insert(allocation, false);
            aggregate.structured_arms_visited += 1;
            store_value(context, state, destination, &ty, result)?;
            return Ok(EmitValue::Basic(base.into()));
        }
    }
    if aggregate.plan.named_result_arms.is_empty() { return Ok(result) }
    let nodes = state.body_call_nodes.ok_or("missing structured result occurrences")?;
    let Some(&(output, producer)) = nodes.block_ordinal(block)
        .and_then(|ordinal| aggregate.plan.named_result_arms.get(&ordinal)) else { return Ok(result) };
    let Some((expression, _)) = block.final_expressions().next() else { return Err("missing named terminal".into()) };
    let ScalarExpression::Name { name, .. } = expression else { return Err("invalid named terminal".into()) };
    if nodes.expression_ordinal(expression) != Some(output) { return Err("changed named terminal".into()) }
    let (_, ty) = state.storage.get(name).ok_or("missing named descriptor storage")?;
    let ty = ty.clone();
    let destination = aggregate.destination.ok_or("missing named result destination")?;
    let base = aggregate.receipts.remove(&(producer, None)).ok_or("missing named result backing")?;
    aggregate.structured_arms_visited += 1;
    store_value(context, state, destination, &ty, result)?;
    Ok(EmitValue::Basic(base.into()))
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
    if let Some(occurrence) = aggregate.plan.guarded_array_descriptor_producer_choice {
        let bound = block.final_expressions().next().and_then(|(expression, _)|
            state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
        let EmitValue::Basic(BasicValueEnum::StructValue(pair)) = result else {
            return Err("missing selected guarded array descriptor".into());
        };
        if bound != Some(occurrence) || pair.get_type() != array_descriptor_type(context)
            || aggregate.array_descriptor_result.replace(*pair).is_some()
        {
            return Err("selected guarded array descriptor did not match its exact output".into());
        }
        return Ok(());
    }
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
        if aggregate.plan.array_descriptor_return {
            let extent = aggregate.array_descriptor_extent.take().ok_or("missing original array descriptor extent")?;
            let pair = state.builder.build_insert_value(array_descriptor_type(context).get_undef(), base, 0, "array_return_backing")
                .map_err(builder_error)?.into_struct_value();
            let pair = state.builder.build_insert_value(pair, extent, 1, "array_return_extent")
                .map_err(builder_error)?.into_struct_value();
            aggregate.array_descriptor_result = Some(pair);
        }
        return Ok(());
    }
    if let Some((call, expected)) = &aggregate.plan.multi_forward {
        let nodes = state.body_call_nodes.ok_or("missing ordered owning relay occurrences")?;
        if aggregate.plan.returning != Some(ClosedReturn::MultipleForward)
            || aggregate.committed || aggregate.mixed_return.is_none()
            || aggregate.plan.multi_receives.get(call).is_none_or(|receive| receive.retained.len() != expected.len())
            || expected.len() != outputs.outputs.len()
            || !block.final_expressions().map(|(expression, _)| nodes.expression_ordinal(expression))
                .eq(expected.iter().copied().map(Some))
        {
            return Err("ordered owning relay changed its sealed outputs".into());
        }
        aggregate.committed = true;
        return Ok(());
    }
    if aggregate.plan.returning == Some(ClosedReturn::MultipleAllocations) {
        let nodes = state.body_call_nodes.ok_or("missing ordered owning return occurrences")?;
        let lanes = &aggregate.plan.multi_returns;
        if aggregate.committed || lanes.len() != outputs.outputs.len()
            || !block.final_expressions().map(|(expression, _)| nodes.expression_ordinal(expression))
                .eq(lanes.iter().map(|&(_, expression)| Some(expression)))
        {
            return Err("ordered owning return changed its sealed outputs".into());
        }
        let pointer = context.ptr_type(AddressSpace::default());
        let mut returned = context.struct_type(&vec![pointer.into(); lanes.len()], false).const_zero();
        let mut selected = Vec::with_capacity(lanes.len());
        for ((allocation, _), destination) in lanes.iter().zip(&aggregate.multi_destinations) {
            let (base, name) = aggregate.multi_allocations.remove(allocation).flatten().ok_or("missing ordered original allocation")?;
            if state.runtime_array_owners.get(&name) != Some(&true) {
                return Err("ordered owning return has no live original allocation".into());
            }
            selected.push((*destination, base, name));
        }
        for (lane, (destination, base, name)) in selected.into_iter().enumerate() {
            let descriptor = extract_output(state, result, lane)?;
            store_value(context, state, destination, &outputs.outputs[lane].ty, descriptor)?;
            let index = u32::try_from(lane).map_err(|_| "owning return lane overflow")?;
            returned = state.builder.build_insert_value(returned, base, index, "owning_return_base")
                .map_err(builder_error)?.into_struct_value();
            state.runtime_array_owners.insert(name, false);
        }
        let aggregate = state.aggregate.as_mut().expect("ordered owning return frame");
        aggregate.mixed_return = Some(returned);
        aggregate.committed = true;
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
    if let Some(ClosedReturn::Structured { output }) = aggregate.plan.returning {
        let root = block.final_expressions().next().and_then(|(expression, _)|
            state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
        let EmitValue::Basic(BasicValueEnum::PointerValue(base)) = result else {
            return Err("structured aggregate result has no joined backing value".into());
        };
        if root != Some(output) || aggregate.committed
            || aggregate.structured_arms_visited != aggregate.plan.calls.values()
                .filter(|&&action| action == ClosedCall::ResultArm).count() + aggregate.plan.named_result_arms.len() + aggregate.plan.local_result_arms.len()
            || !matches!(outputs.outputs.as_slice(), [result] if matches!(result.ty, ScalarType::Struct(_)))
        {
            return Err("structured aggregate result changed its proved output tree".into());
        }
        if aggregate.plan.structured_result_binding.is_some() {
            if aggregate.destination != Some(*base) || aggregate.base.is_none() {
                return Err("structured result name did not read its bound descriptor".into());
            }
        } else {
            aggregate.set_base(*base)?;
        }
        aggregate.committed = true;
        return Ok(());
    }
    if let Some(ClosedReturn::MixedSelected { lanes, call, outputs: exact_outputs }) = aggregate.plan.returning {
        let nodes = state.body_call_nodes.ok_or("missing mixed selected output occurrences")?;
        if aggregate.committed || mixed_owned_struct_result(outputs) != Some(lanes)
            || !block.final_expressions().map(|(value, _)| nodes.expression_ordinal(value))
                .eq(exact_outputs.map(Some))
        {
            return Err("mixed selected return changed its sealed outputs".into());
        }
        let destination = aggregate.destination.ok_or("missing mixed selected result destination")?;
        let base = aggregate.receipts.remove(&(call, None)).ok_or("missing mixed selected original backing")?;
        let pair = mixed_allocation_pair(context, state, result, outputs, lanes.owner, destination, base)?;
        let aggregate = state.aggregate.as_mut().expect("mixed selected return frame");
        aggregate.mixed_return = Some(pair);
        aggregate.committed = true;
        return Ok(());
    }
    if let Some(ClosedReturn::Recommit { call, output } | ClosedReturn::NamedRoot { call, output }) = aggregate.plan.returning {
        let returned = block.final_expressions().next().and_then(|(expression, _)|
            state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression)));
        let [descriptor] = outputs.outputs.as_slice() else {
            return Err("named aggregate return is not a single descriptor".into());
        };
        if returned != Some(output) || !matches!(descriptor.ty, ScalarType::Struct(_)) || aggregate.committed {
            return Err("named aggregate return changed its proved root output".into());
        }
        let destination = aggregate.destination.ok_or_else(|| "missing named aggregate destination".to_owned())?;
        let base = aggregate.receipts.remove(&(call, None))
            .ok_or_else(|| "missing original backing at named aggregate return".to_owned())?;
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
    let pointer = memory::checked_ordinary_snapshot_element_pointer(
        context, state, ty, global.as_pointer_value(), proof.length, index, project,
    )?;
    state.aggregate.as_mut().expect("checked snapshot frame").ordinary_indexed_emitted |= bit;
    Ok(Some(EmitValue::Basic(pointer.into())))
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
    if aggregate.local_choice_committed != aggregate.plan.local_choice_arms.len()
        || aggregate.local_choice_visited != 0 || aggregate.local_choice_destination.is_some()
    {
        return Err("local conditional owner was not committed to lexical storage".into());
    }
    if !aggregate.receipts.is_empty() || !aggregate.multi_allocations.is_empty() || aggregate.receipt_guards.len() != aggregate.plan.conditional_guard_count || !aggregate.inputs.is_empty() {
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
    if aggregate.plan.array_descriptor_return {
        let pair = aggregate.array_descriptor_result.take().ok_or("missing committed array descriptor return")?;
        state.builder.build_return(Some(&pair)).map_err(builder_error)?;
        return Ok(Some(()));
    }
    let Some(route) = aggregate.plan.returning else {
        return Ok(None);
    };
    if matches!(route, ClosedReturn::MultipleAllocations | ClosedReturn::MultipleForward | ClosedReturn::Allocation { .. } | ClosedReturn::Recommit { .. } | ClosedReturn::NamedRoot { .. } | ClosedReturn::Structured { .. }
        | ClosedReturn::MixedSelected { .. }) && !aggregate.committed {
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
