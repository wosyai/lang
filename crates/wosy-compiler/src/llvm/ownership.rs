use super::memory::has_callable_holders;
use super::*;
use crate::scalar::{
    RuntimeBackingFact, RuntimeBackingKind, RuntimeReturnBackingAction, RuntimeValueId,
};
use inkwell::values::BasicValue;
use inkwell::AtomicOrdering;

const GRAPH_MUTATION_LOCK: &str = "__wosy_graph_mutation_lock";
const BEGIN_GRAPH_MUTATION: &str = "__wosy_begin_graph_mutation";
const END_GRAPH_MUTATION: &str = "__wosy_end_graph_mutation";

/// Call begin/end once around a complete outer graph mutation after staging the RHS.
/// Nested count/drop/dispose code must run inside that boundary without reacquiring it.
/// User code and indirect Wosy calls must run outside the boundary.
pub(super) fn emit_graph_mutation_primitive<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_layout: ScalarTargetLayout,
    collector: bool,
) -> Result<(), String> {
    if collector {
        emit_gc_registry(context, module, target_layout)?;
        emit_collector_runtime(context, module, target_layout)?;
    }
    let builder = context.create_builder();
    let signature = context.void_type().fn_type(&[], false);
    let begin = module.add_function(BEGIN_GRAPH_MUTATION, signature, Some(Linkage::Internal));
    let end = module.add_function(END_GRAPH_MUTATION, signature, Some(Linkage::Internal));

    if target_layout == ScalarTargetLayout::NATIVE64 {
        let lock = module.add_global(context.i32_type(), None, GRAPH_MUTATION_LOCK);
        lock.set_linkage(Linkage::Private);
        lock.set_initializer(&context.i32_type().const_zero());
        lock.set_alignment(4);

        let entry = context.append_basic_block(begin, "entry");
        let attempt = context.append_basic_block(begin, "graph_mutation.attempt");
        let acquired = context.append_basic_block(begin, "graph_mutation.acquired");
        builder.position_at_end(entry);
        builder
            .build_unconditional_branch(attempt)
            .map_err(builder_error)?;
        builder.position_at_end(attempt);
        let exchange = builder
            .build_cmpxchg(
                lock.as_pointer_value(),
                context.i32_type().const_zero(),
                context.i32_type().const_int(1, false),
                AtomicOrdering::Acquire,
                AtomicOrdering::Monotonic,
            )
            .map_err(builder_error)?;
        let success = builder
            .build_extract_value(exchange, 1, "graph_mutation_locked")
            .map_err(builder_error)?
            .into_int_value();
        builder
            .build_conditional_branch(success, acquired, attempt)
            .map_err(builder_error)?;
        builder.position_at_end(acquired);
        builder.build_return(None).map_err(builder_error)?;

        let end_entry = context.append_basic_block(end, "entry");
        builder.position_at_end(end_entry);
        if collector {
            let pointer = context.ptr_type(AddressSpace::default());
            let env_head = module.get_global(GC_ENV_HEAD).expect("collector env head");
            let cell_head = module
                .get_global(GC_CELL_HEAD)
                .expect("collector cell head");
            let envs = builder
                .build_load(pointer, env_head.as_pointer_value(), "gc_trial_envs")
                .map_err(builder_error)?
                .into_pointer_value();
            let cells = builder
                .build_load(pointer, cell_head.as_pointer_value(), "gc_trial_cells")
                .map_err(builder_error)?
                .into_pointer_value();
            let env_empty = builder
                .build_is_null(envs, "gc_trial_env_empty")
                .map_err(builder_error)?;
            let cell_empty = builder
                .build_is_null(cells, "gc_trial_cell_empty")
                .map_err(builder_error)?;
            let idle = builder
                .build_and(env_empty, cell_empty, "gc_trial_idle")
                .map_err(builder_error)?;
            let collect = context.append_basic_block(end, "graph_mutation.collect");
            let publish = context.append_basic_block(end, "graph_mutation.publish");
            builder
                .build_conditional_branch(idle, publish, collect)
                .map_err(builder_error)?;
            builder.position_at_end(collect);
            builder
                .build_call(
                    module
                        .get_function(GC_TRIAL_SYMBOL)
                        .expect("collector trial"),
                    &[],
                    "gc_trial_ran",
                )
                .map_err(builder_error)?;
            builder
                .build_unconditional_branch(publish)
                .map_err(builder_error)?;
            builder.position_at_end(publish);
        } else {
            builder.position_at_end(end_entry);
        }
        let release = builder
            .build_store(lock.as_pointer_value(), context.i32_type().const_zero())
            .map_err(builder_error)?;
        release
            .set_atomic_ordering(AtomicOrdering::Release)
            .map_err(|error| error.to_string())?;
        release
            .set_alignment(4)
            .map_err(|error| error.to_string())?;
        builder.build_return(None).map_err(builder_error)?;
    } else if target_layout == ScalarTargetLayout::WASM32 {
        builder.position_at_end(context.append_basic_block(begin, "entry"));
        builder.build_return(None).map_err(builder_error)?;
        let end_entry = context.append_basic_block(end, "entry");
        builder.position_at_end(end_entry);
        if collector {
            let pointer = context.ptr_type(AddressSpace::default());
            let env_head = module.get_global(GC_ENV_HEAD).expect("collector env head");
            let cell_head = module
                .get_global(GC_CELL_HEAD)
                .expect("collector cell head");
            let envs = builder
                .build_load(pointer, env_head.as_pointer_value(), "gc_trial_envs")
                .map_err(builder_error)?
                .into_pointer_value();
            let cells = builder
                .build_load(pointer, cell_head.as_pointer_value(), "gc_trial_cells")
                .map_err(builder_error)?
                .into_pointer_value();
            let env_empty = builder
                .build_is_null(envs, "gc_trial_env_empty")
                .map_err(builder_error)?;
            let cell_empty = builder
                .build_is_null(cells, "gc_trial_cell_empty")
                .map_err(builder_error)?;
            let idle = builder
                .build_and(env_empty, cell_empty, "gc_trial_idle")
                .map_err(builder_error)?;
            let collect = context.append_basic_block(end, "graph_mutation.collect");
            let publish = context.append_basic_block(end, "graph_mutation.publish");
            builder
                .build_conditional_branch(idle, publish, collect)
                .map_err(builder_error)?;
            builder.position_at_end(collect);
            builder
                .build_call(
                    module
                        .get_function(GC_TRIAL_SYMBOL)
                        .expect("collector trial"),
                    &[],
                    "gc_trial_ran",
                )
                .map_err(builder_error)?;
            builder
                .build_unconditional_branch(publish)
                .map_err(builder_error)?;
            builder.position_at_end(publish);
        }
        builder.build_return(None).map_err(builder_error)?;
    } else {
        return Err(format!(
            "unsupported graph mutation target layout: {target_layout:?}"
        ));
    }
    Ok(())
}

pub(super) fn begin_graph_mutation<'ctx>(
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
) -> Result<(), String> {
    builder
        .build_call(
            module
                .get_function(BEGIN_GRAPH_MUTATION)
                .expect("graph mutation primitive"),
            &[],
            "",
        )
        .map_err(builder_error)?;
    Ok(())
}

pub(super) fn end_graph_mutation<'ctx>(
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
) -> Result<(), String> {
    builder
        .build_call(
            module
                .get_function(END_GRAPH_MUTATION)
                .expect("graph mutation primitive"),
            &[],
            "",
        )
        .map_err(builder_error)?;
    Ok(())
}

pub(super) use super::collector::{
    capture_cell_gc_mark_address, emit_collector_runtime, emit_gc_registry,
    environment_gc_mark_address, init_capture_cell_gc_header, init_environment_gc_header,
    register_capture_cell, register_environment, unregister_capture_cell, unregister_environment,
    CELL_GC_PREFIX, ENVIRONMENT_GC_PREFIX, GC_CELL_HEAD, GC_ENV_HEAD, GC_TRIAL_SYMBOL,
};

pub(super) struct EnvironmentOwnedBacking {
    pub fact: RuntimeBackingFact,
    pub cell_offset: u64,
    pub element: ScalarType,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct EnvironmentCaptureCell {
    pub cell_offset: u64,
    pub element: ScalarType,
    pub residence: CaptureCellResidence,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImportedHolderKey {
    pub source: wosy_syntax::SourceIdentity,
    pub function_span: wosy_syntax::ByteSpan,
    pub call_point: crate::scalar::CfgPointId,
    pub output: usize,
    pub fields: Vec<usize>,
}

pub(super) struct ImportedCaptureCandidate {
    pub literal: wosy_syntax::SourceSpan,
    pub backing: EnvironmentOwnedBacking,
    pub ownership: ImportedCaptureOwnership,
}

pub(super) enum ImportedCaptureOwnership {
    Counted(ImportedCaptureCoverage),
    CountedCell,
    Synthetic,
}

pub(super) struct ImportedCaptureCoverage {
    pub key: ImportedHolderKey,
    pub environment: crate::scalar::CallableEnvironmentId,
    pub backing: RuntimeBackingFact,
    pub producer: super::holder_events::CaptureDescriptor,
    pub binding: crate::scalar::ReferenceBindingId,
    pub actions: Vec<(
        super::holder_events::HolderEmissionEdge,
        crate::scalar::CallableHolderAction,
    )>,
    pub release_edges: Vec<super::holder_events::HolderEmissionEdge>,
}

pub(super) struct ImportedCaptureHolder<'ctx> {
    pub holder: crate::scalar::CallableHolderPlace,
    pub code: PointerValue<'ctx>,
    pub environment: PointerValue<'ctx>,
    pub present: PointerValue<'ctx>,
    pub owned: Vec<ImportedCaptureCandidate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CaptureCellResidence {
    Promoted,
    Global,
    External,
}

fn capture_cell_header_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    offset: i64,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            cell,
            &[context.i64_type().const_int(offset as u64, false)],
            name,
        )
    }
    .map_err(builder_error)
}

pub(super) fn capture_cell_descriptor_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    capture_cell_header_address(context, state, cell, -32, "capture_cell_descriptor_address")
}

pub(super) fn capture_cell_count_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    capture_cell_header_address(
        context,
        state,
        cell,
        -24,
        "capture_cell_owner_count_address",
    )
}

pub(super) fn capture_cell_base_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    capture_cell_header_address(
        context,
        state,
        cell,
        -16,
        "capture_cell_allocation_base_address",
    )
}

pub(super) fn capture_cell_owned_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    capture_cell_header_address(
        context,
        state,
        cell,
        -8,
        "capture_cell_owns_backing_address",
    )
}

pub(super) fn capture_cell_length_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            cell,
            &[context.i64_type().const_int(8, false)],
            "capture_cell_current_length_address",
        )
    }
    .map_err(builder_error)
}

pub(super) fn retain_capture_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    residence: CaptureCellResidence,
) -> Result<(), String> {
    if !matches!(residence, CaptureCellResidence::Promoted) {
        return Ok(());
    }
    let address = capture_cell_count_address(context, state, cell)?;
    let count = state
        .builder
        .build_load(context.i64_type(), address, "capture_cell_owner_count")
        .map_err(builder_error)?
        .into_int_value();
    count
        .as_instruction_value()
        .expect("capture cell count load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    let zero = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            count,
            context.i64_type().const_zero(),
            "capture_cell_unowned",
        )
        .map_err(builder_error)?;
    let full = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            count,
            context.i64_type().const_all_ones(),
            "capture_cell_count_full",
        )
        .map_err(builder_error)?;
    let invalid = state
        .builder
        .build_or(zero, full, "capture_cell_retain_invalid")
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("capture retain block")
        .get_parent()
        .expect("capture retain function");
    let fail = context.append_basic_block(function, "capture_cell.retain.panic");
    let ready = context.append_basic_block(function, "capture_cell.retain.ready");
    state
        .builder
        .build_conditional_branch(invalid, fail, ready)
        .map_err(builder_error)?;
    state.builder.position_at_end(fail);
    declare_core_runtime(context, state.module);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .expect("core panic declaration");
    state
        .builder
        .build_call(panic, &[], "capture_cell_retain_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(ready);
    let next = state
        .builder
        .build_int_add(
            count,
            context.i64_type().const_int(1, false),
            "capture_cell_owner_retained",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(address, next)
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn drop_capture_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    residence: CaptureCellResidence,
    element: &ScalarType,
) -> Result<(), String> {
    if !matches!(residence, CaptureCellResidence::Promoted) {
        return Ok(());
    }
    let drop_function = state
        .builder
        .get_insert_block()
        .expect("capture drop block")
        .get_parent()
        .expect("capture drop function");
    let resident = context.append_basic_block(drop_function, "capture_cell.drop.resident");
    let settled_done = context.append_basic_block(drop_function, "capture_cell.drop.done");
    let gc_mark = state
        .builder
        .build_load(
            context.i64_type(),
            capture_cell_gc_mark_address(context, state, cell)?,
            "capture_cell_gc_mark",
        )
        .map_err(builder_error)?
        .into_int_value();
    gc_mark
        .as_instruction_value()
        .expect("capture collector mark load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    let gc_retiring = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            gc_mark,
            context.i64_type().const_int(3, false),
            "capture_cell_gc_retiring",
        )
        .map_err(builder_error)?;
    let gc_disposed = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            gc_mark,
            context.i64_type().const_int(4, false),
            "capture_cell_gc_disposed",
        )
        .map_err(builder_error)?;
    let gc_settled = state
        .builder
        .build_or(gc_retiring, gc_disposed, "capture_cell_gc_settled")
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(gc_settled, settled_done, resident)
        .map_err(builder_error)?;
    state.builder.position_at_end(resident);
    let address = capture_cell_count_address(context, state, cell)?;
    let count = state
        .builder
        .build_load(context.i64_type(), address, "capture_cell_owner_count")
        .map_err(builder_error)?
        .into_int_value();
    count
        .as_instruction_value()
        .expect("capture cell count load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    emit_checked_dereference_guard(context, state, count)?;
    let next = state
        .builder
        .build_int_sub(
            count,
            context.i64_type().const_int(1, false),
            "capture_cell_owner_dropped",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(address, next)
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    let last = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            next,
            context.i64_type().const_zero(),
            "capture_cell_last_owner",
        )
        .map_err(builder_error)?;
    let retire = context.append_basic_block(drop_function, "capture_cell.drop.final");
    let done = settled_done;
    state
        .builder
        .build_conditional_branch(last, retire, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(retire);
    unregister_capture_cell(state, cell)?;
    declare_core_runtime(context, state.module);
    let free = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .expect("core free declaration");
    let owns = state
        .builder
        .build_load(
            context.i64_type(),
            capture_cell_owned_address(context, state, cell)?,
            "capture_cell_owns_backing",
        )
        .map_err(builder_error)?
        .into_int_value();
    let owned = state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            owns,
            context.i64_type().const_zero(),
            "capture_cell_owned",
        )
        .map_err(builder_error)?;
    let backing_block = context.append_basic_block(drop_function, "capture_cell.drop.backing");
    let base_block = context.append_basic_block(drop_function, "capture_cell.drop.base");
    state
        .builder
        .build_conditional_branch(owned, backing_block, base_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(backing_block);
    if captured_callable_backing(element, state.structs)? {
        call_capture_cell_dispose(context, state, cell)?;
    } else {
        let backing = state
            .builder
            .build_load(
                context.ptr_type(AddressSpace::default()),
                cell,
                "capture_cell_backing",
            )
            .map_err(builder_error)?
            .into_pointer_value();
        state
            .builder
            .build_call(free, &[backing.into()], "capture_cell_backing_release")
            .map_err(builder_error)?;
    }
    state
        .builder
        .build_unconditional_branch(base_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(base_block);
    let base = state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            capture_cell_base_address(context, state, cell)?,
            "capture_cell_allocation_base",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    state
        .builder
        .build_call(free, &[base.into()], "capture_cell_allocation_release")
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn replace_capture_cell_backing<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    element: &ScalarType,
) -> Result<(), String> {
    let owned = state
        .builder
        .build_load(
            context.i64_type(),
            capture_cell_owned_address(context, state, cell)?,
            "capture_cell_previous_owned",
        )
        .map_err(builder_error)?
        .into_int_value();
    let has_backing = state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            owned,
            context.i64_type().const_zero(),
            "capture_cell_previous_present",
        )
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("cell replacement block")
        .get_parent()
        .expect("cell replacement function");
    let free_backing = context.append_basic_block(function, "capture_cell.replace.old");
    let next = context.append_basic_block(function, "capture_cell.replace.next");
    state
        .builder
        .build_conditional_branch(has_backing, free_backing, next)
        .map_err(builder_error)?;
    state.builder.position_at_end(free_backing);
    if captured_callable_backing(element, state.structs)? {
        call_capture_cell_dispose(context, state, cell)?;
    } else {
        declare_core_runtime(context, state.module);
        let previous = state
            .builder
            .build_load(
                context.ptr_type(AddressSpace::default()),
                cell,
                "capture_cell_previous_backing",
            )
            .map_err(builder_error)?
            .into_pointer_value();
        let free = state
            .module
            .get_function(CORE_FREE_SYMBOL)
            .expect("core free declaration");
        state
            .builder
            .build_call(free, &[previous.into()], "capture_cell_previous_release")
            .map_err(builder_error)?;
    }
    state
        .builder
        .build_unconditional_branch(next)
        .map_err(builder_error)?;
    state.builder.position_at_end(next);
    Ok(())
}

fn captured_callable_backing(
    element: &ScalarType,
    structs: &[ScalarStruct],
) -> Result<bool, String> {
    match element {
        ScalarType::RuntimeArray { element, .. } => has_callable_holders(element, structs),
        _ => has_callable_holders(element, structs),
    }
}

fn call_capture_cell_dispose<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let descriptor = state
        .builder
        .build_load(
            pointer,
            capture_cell_descriptor_address(context, state, cell)?,
            "capture_cell_descriptor",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    let missing = state
        .builder
        .build_is_null(descriptor, "capture_cell_undescribed")
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("capture dispose block")
        .get_parent()
        .expect("capture dispose function");
    let described = context.append_basic_block(function, "capture_cell.dispose.described");
    let plain = context.append_basic_block(function, "capture_cell.dispose.plain");
    let done = context.append_basic_block(function, "capture_cell.dispose.done");
    state
        .builder
        .build_conditional_branch(missing, plain, described)
        .map_err(builder_error)?;
    state.builder.position_at_end(described);
    let address = unsafe {
        state.builder.build_in_bounds_gep(
            pointer,
            descriptor,
            &[context.i32_type().const_int(1, false)],
            "capture_cell_dispose_address",
        )
    }
    .map_err(builder_error)?;
    let dispose = state
        .builder
        .build_load(pointer, address, "capture_cell_dispose")
        .map_err(builder_error)?
        .into_pointer_value();
    state
        .builder
        .build_indirect_call(
            context.void_type().fn_type(&[pointer.into()], false),
            dispose,
            &[cell.into()],
            "capture_cell_final_dispose",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(plain);
    declare_core_runtime(context, state.module);
    let backing = state
        .builder
        .build_load(pointer, cell, "capture_cell_undescribed_backing")
        .map_err(builder_error)?
        .into_pointer_value();
    state
        .builder
        .build_call(
            state
                .module
                .get_function(CORE_FREE_SYMBOL)
                .expect("core free declaration"),
            &[backing.into()],
            "capture_cell_undescribed_release",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn install_capture_cell_descriptor<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    descriptor: &super::holder_events::CaptureDescriptorCell,
) -> Result<(), String> {
    let ScalarType::RuntimeArray { element, .. } = &descriptor.ty else {
        return Err(format!(
            "captured callable cell {:?} lacks a runtime array type",
            descriptor.binding
        ));
    };
    if descriptor.residence != CaptureCellResidence::Promoted
        || !matches!(element.as_ref(), ScalarType::Callable { .. })
        || descriptor.backings.is_empty()
        || descriptor.backings.iter().any(|backing| {
            backing.kind != RuntimeBackingKind::OwnedAllocation
                || backing.callable_family.len() != 1
                || !crate::scalar::scalar_type_equal(&backing.callable_family[0].element, element)
        })
        || descriptor.initialized_cases.iter().any(|case| {
            !matches!(
                case,
                crate::scalar::CallableCellCaseFact::Initialized { .. }
            )
        })
    {
        return Err(format!(
            "captured callable cell {:?} lacks source-qualified owned element coverage",
            descriptor.binding
        ));
    }
    for case in &descriptor.initialized_cases {
        let crate::scalar::CallableCellCaseFact::Initialized {
            environment_origin, ..
        } = case
        else {
            unreachable!("validated initialized case")
        };
        if !state
            .literal_captures
            .contains_key(&environment_origin.literal)
        {
            return Err(format!(
                "captured callable cell {:?} lacks source-qualified element producer {:?}",
                descriptor.binding, environment_origin.literal
            ));
        }
    }
    let source = wosy_syntax::SourceSpan::new(
        descriptor.binding.source.clone(),
        descriptor.binding.declaration_span,
    );
    let symbol = super::private_literal_symbol("__wosy_capture_cell", &source);
    let global = if let Some(global) = state.module.get_global(&symbol) {
        global
    } else {
        emit_array_cell_descriptor(context, state, &symbol, element)?
    };
    state
        .builder
        .build_store(
            capture_cell_descriptor_address(context, state, cell)?,
            global.as_pointer_value(),
        )
        .map_err(builder_error)?;
    Ok(())
}

fn emit_array_cell_descriptor<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    symbol: &str,
    element: &ScalarType,
) -> Result<inkwell::values::GlobalValue<'ctx>, String> {
    let pointer = context.ptr_type(AddressSpace::default());
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
    let global = state.module.add_global(descriptor_type, None, symbol);
    global.set_linkage(Linkage::Private);
    global.set_constant(true);
    global.set_initializer(&descriptor_type.const_named_struct(&[
        trace.as_global_value().as_pointer_value().into(),
        dispose.as_global_value().as_pointer_value().into(),
    ]));
    let caller = state
        .builder
        .get_insert_block()
        .expect("capture descriptor caller");
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
    walk_capture_cell_elements(context, state, current, element, Some(visitor))?;
    state.builder.build_return(None).map_err(builder_error)?;

    state
        .builder
        .position_at_end(context.append_basic_block(dispose, "entry"));
    let current = dispose
        .get_first_param()
        .expect("dispose cell")
        .into_pointer_value();
    walk_capture_cell_elements(context, state, current, element, None)?;
    declare_core_runtime(context, state.module);
    let data = state
        .builder
        .build_load(pointer, current, "capture_cell_current_backing")
        .map_err(builder_error)?
        .into_pointer_value();
    let base = callable_array_allocation_base(context, state, data)?;
    state
        .builder
        .build_call(
            state
                .module
                .get_function(CORE_FREE_SYMBOL)
                .expect("core free declaration"),
            &[base.into()],
            "capture_cell_backing_release",
        )
        .map_err(builder_error)?;
    state.builder.build_return(None).map_err(builder_error)?;
    state.builder.position_at_end(caller);
    Ok(global)
}

pub(super) fn install_fixed_array_cell_descriptor<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
    cell: PointerValue<'ctx>,
    element: &ScalarType,
) -> Result<(), String> {
    if !matches!(element, ScalarType::Callable { .. }) {
        return Err(format!(
            "captured fixed callable cell {binding:?} at {key:?} lacks a callable element type"
        ));
    }
    let symbol = super::private_literal_symbol("__wosy_fixed_cell", key);
    let global = if let Some(global) = state.module.get_global(&symbol) {
        global
    } else {
        emit_array_cell_descriptor(context, state, &symbol, element)?
    };
    state
        .builder
        .build_store(
            capture_cell_descriptor_address(context, state, cell)?,
            global.as_pointer_value(),
        )
        .map_err(builder_error)?;
    Ok(())
}

fn walk_capture_cell_elements<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    element: &ScalarType,
    visitor: Option<PointerValue<'ctx>>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let data = state
        .builder
        .build_load(pointer, cell, "capture_cell_current_array")
        .map_err(builder_error)?
        .into_pointer_value();
    let length = state
        .builder
        .build_load(
            context.i64_type(),
            capture_cell_length_address(context, state, cell)?,
            "capture_cell_current_length",
        )
        .map_err(builder_error)?
        .into_int_value();
    let index_slot = entry_alloca(state, context.i64_type().into(), "capture_cell_walk_index")?;
    state
        .builder
        .build_store(index_slot, context.i64_type().const_zero())
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("capture walk block")
        .get_parent()
        .expect("capture walk function");
    let test = context.append_basic_block(function, "capture_cell.walk.test");
    let visit = context.append_basic_block(function, "capture_cell.walk.visit");
    let installed = context.append_basic_block(function, "capture_cell.walk.installed");
    let step = context.append_basic_block(function, "capture_cell.walk.step");
    let done = context.append_basic_block(function, "capture_cell.walk.done");
    state
        .builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    state.builder.position_at_end(test);
    let index = state
        .builder
        .build_load(context.i64_type(), index_slot, "capture_cell_walk_index")
        .map_err(builder_error)?
        .into_int_value();
    let in_bounds = state
        .builder
        .build_int_compare(
            IntPredicate::ULT,
            index,
            length,
            "capture_cell_walk_in_bounds",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(in_bounds, visit, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(visit);
    let marked = callable_array_element_installed(context, state, data, index)?;
    state
        .builder
        .build_conditional_branch(marked, installed, step)
        .map_err(builder_error)?;
    state.builder.position_at_end(installed);
    let (size, _) = allocation_layout(element, state.structs, state.target_layout)?;
    let offset = state
        .builder
        .build_int_mul(
            index,
            context.i64_type().const_int(size, false),
            "capture_cell_element_offset",
        )
        .map_err(builder_error)?;
    let address = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            data,
            &[offset],
            "capture_cell_element",
        )
    }
    .map_err(builder_error)?;
    let layout = storage_type(context, element, state.structs, state.target_layout)?;
    let callable = state
        .builder
        .build_load(layout, address, "capture_cell_installed_holder")
        .map_err(builder_error)?
        .into_struct_value();
    let environment = state
        .builder
        .build_extract_value(callable, 1, "capture_cell_nested_environment")
        .map_err(builder_error)?
        .into_pointer_value();
    if let Some(visitor) = visitor {
        state
            .builder
            .build_indirect_call(
                context.void_type().fn_type(&[pointer.into()], false),
                visitor,
                &[environment.into()],
                "capture_cell_trace_environment",
            )
            .map_err(builder_error)?;
    } else {
        drop_environment(context, state, environment)?;
    }
    state
        .builder
        .build_unconditional_branch(step)
        .map_err(builder_error)?;
    state.builder.position_at_end(step);
    let next = state
        .builder
        .build_int_add(
            index,
            context.i64_type().const_int(1, false),
            "capture_cell_next_index",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(index_slot, next)
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn retain_installed_capture_elements<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    source: PointerValue<'ctx>,
    backing: PointerValue<'ctx>,
    element: &ScalarType,
    installed: &[u64],
) -> Result<(), String> {
    let callable = context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            context.ptr_type(AddressSpace::default()).into(),
        ],
        false,
    );
    let (size, _) = super::memory::allocation_layout(element, state.structs, state.target_layout)?;
    for index in installed {
        let offset = state
            .builder
            .build_int_mul(
                context.i64_type().const_int(*index, false),
                context.i64_type().const_int(size, false),
                "captured_install_element_offset",
            )
            .map_err(builder_error)?;
        let from = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                source,
                &[offset],
                "captured_install_source",
            )
        }
        .map_err(builder_error)?;
        let holder = state
            .builder
            .build_load(callable, from, "captured_install_holder")
            .map_err(builder_error)?
            .into_struct_value();
        let environment = state
            .builder
            .build_extract_value(holder, 1, "captured_install_environment")
            .map_err(builder_error)?
            .into_pointer_value();
        let present = state
            .builder
            .build_is_not_null(environment, "captured_install_present")
            .map_err(builder_error)?;
        let function = state
            .builder
            .get_insert_block()
            .expect("captured install block")
            .get_parent()
            .expect("captured install function");
        let live = context.append_basic_block(function, "captured_install.live");
        let dead = context.append_basic_block(function, "captured_install.dead");
        state
            .builder
            .build_conditional_branch(present, live, dead)
            .map_err(builder_error)?;
        state.builder.position_at_end(dead);
        declare_core_runtime(context, state.module);
        state
            .builder
            .build_call(
                state
                    .module
                    .get_function(CORE_SYSTEM_PANIC_SYMBOL)
                    .expect("core panic declaration"),
                &[],
                "captured_install_panic",
            )
            .map_err(builder_error)?;
        state.builder.build_unreachable().map_err(builder_error)?;
        state.builder.position_at_end(live);
        let into = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                backing,
                &[offset],
                "captured_install_destination",
            )
        }
        .map_err(builder_error)?;
        state
            .builder
            .build_store(into, holder)
            .map_err(builder_error)?;
        retain_environment(context, state, environment)?;
        install_callable_array_element(
            context,
            state,
            backing,
            context.i64_type().const_int(*index, false),
        )?;
    }
    Ok(())
}

pub(super) fn environment_count_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            environment,
            &[context.i64_type().const_int((-24i64) as u64, false)],
            "environment_holder_count_address",
        )
    }
    .map_err(builder_error)
}

pub(super) fn environment_base_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            environment,
            &[context.i64_type().const_int((-16i64) as u64, false)],
            "environment_allocation_base_address",
        )
    }
    .map_err(builder_error)
}

pub(super) fn environment_descriptor_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            environment,
            &[context.i64_type().const_int((-8i64) as u64, false)],
            "environment_descriptor_address",
        )
    }
    .map_err(builder_error)
}

fn environment_count<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), String> {
    let address = environment_count_address(context, state, environment)?;
    let count = state
        .builder
        .build_load(context.i64_type(), address, "environment_holder_count")
        .map_err(builder_error)?
        .into_int_value();
    count
        .as_instruction_value()
        .expect("environment count load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    Ok((address, count))
}

pub(super) fn retain_environment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<(), String> {
    let function = state
        .builder
        .get_insert_block()
        .expect("environment retain block")
        .get_parent()
        .expect("environment retain function");
    let present = context.append_basic_block(function, "environment.retain.present");
    let done = context.append_basic_block(function, "environment.retain.done");
    let nonnull = state
        .builder
        .build_is_not_null(environment, "environment_present")
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(nonnull, present, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(present);
    let (address, count) = environment_count(context, state, environment)?;
    let saturated = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            count,
            context.i64_type().const_all_ones(),
            "environment_count_saturated",
        )
        .map_err(builder_error)?;
    let invalid = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            count,
            context.i64_type().const_zero(),
            "environment_count_invalid",
        )
        .map_err(builder_error)?;
    let overflow = state
        .builder
        .build_or(saturated, invalid, "environment_retain_invalid")
        .map_err(builder_error)?;
    let panic_block = context.append_basic_block(function, "environment.retain.panic");
    let increment = context.append_basic_block(function, "environment.retain.increment");
    state
        .builder
        .build_conditional_branch(overflow, panic_block, increment)
        .map_err(builder_error)?;
    state.builder.position_at_end(panic_block);
    declare_core_runtime(context, state.module);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .expect("core panic declaration");
    state
        .builder
        .build_call(panic, &[], "environment_retain_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(increment);
    let next = state
        .builder
        .build_int_add(
            count,
            context.i64_type().const_int(1, false),
            "environment_holder_retained",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(address, next)
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn drop_environment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<(), String> {
    let function = state
        .builder
        .get_insert_block()
        .expect("environment drop block")
        .get_parent()
        .expect("environment drop function");
    let present = context.append_basic_block(function, "environment.drop.present");
    let done = context.append_basic_block(function, "environment.drop.done");
    let nonnull = state
        .builder
        .build_is_not_null(environment, "environment_present")
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(nonnull, present, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(present);
    let gc_mark = state
        .builder
        .build_load(
            context.i64_type(),
            environment_gc_mark_address(context, state, environment)?,
            "environment_gc_mark",
        )
        .map_err(builder_error)?
        .into_int_value();
    gc_mark
        .as_instruction_value()
        .expect("environment collector mark load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    let gc_retiring = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            gc_mark,
            context.i64_type().const_int(3, false),
            "environment_gc_retiring",
        )
        .map_err(builder_error)?;
    let gc_disposed = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            gc_mark,
            context.i64_type().const_int(4, false),
            "environment_gc_disposed",
        )
        .map_err(builder_error)?;
    let gc_settled = state
        .builder
        .build_or(gc_retiring, gc_disposed, "environment_gc_settled")
        .map_err(builder_error)?;
    let resident = context.append_basic_block(function, "environment.drop.resident");
    state
        .builder
        .build_conditional_branch(gc_settled, done, resident)
        .map_err(builder_error)?;
    state.builder.position_at_end(resident);
    let (address, count) = environment_count(context, state, environment)?;
    super::emit_checked_dereference_guard(context, state, count)?;
    let next = state
        .builder
        .build_int_sub(
            count,
            context.i64_type().const_int(1, false),
            "environment_holder_dropped",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(address, next)
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    let last = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            next,
            context.i64_type().const_zero(),
            "environment_last_holder",
        )
        .map_err(builder_error)?;
    let release = context.append_basic_block(function, "environment.drop.final");
    state
        .builder
        .build_conditional_branch(last, release, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(release);
    unregister_environment(state, environment)?;
    let descriptor = state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            environment_descriptor_address(context, state, environment)?,
            "environment_descriptor",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    let dispose_address = unsafe {
        state.builder.build_in_bounds_gep(
            context.ptr_type(AddressSpace::default()),
            descriptor,
            &[context.i32_type().const_int(1, false)],
            "environment_dispose_address",
        )
    }
    .map_err(builder_error)?;
    let dispose = state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            dispose_address,
            "environment_dispose",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    state
        .builder
        .build_indirect_call(
            context
                .void_type()
                .fn_type(&[context.ptr_type(AddressSpace::default()).into()], false),
            dispose,
            &[environment.into()],
            "environment_final_dispose",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn install_environment_descriptor<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    descriptor: &super::holder_events::CaptureDescriptor,
) -> Result<inkwell::values::GlobalValue<'ctx>, String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let symbol = super::private_literal_symbol("__wosy_environment", &descriptor.literal);
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
        .expect("environment descriptor caller");
    state
        .builder
        .position_at_end(context.append_basic_block(trace, "entry"));
    let environment = trace
        .get_nth_param(0)
        .expect("trace environment")
        .into_pointer_value();
    let visitor = trace
        .get_nth_param(1)
        .expect("trace visitor")
        .into_pointer_value();
    let mut cells = Vec::new();
    for cell in &descriptor.cells {
        if cell.residence != CaptureCellResidence::Promoted {
            continue;
        }
        if state
            .holder_events
            .covered_cells
            .contains_key(&cell.binding)
        {
            cells.push(cell);
        } else if matches!(cell.ty, ScalarType::Callable { .. })
            && state
                .literal_captures
                .get(&descriptor.literal)
                .is_some_and(|producer| super::cyclic_callable_backedge(producer, &cell.binding))
        {
            cells.push(cell);
        } else if matches!(&cell.ty, ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } if has_callable_holders(element, state.structs).is_ok_and(|holders| holders))
            && super::fixed_cycle_cell_covered(
                state.literal_captures,
                &descriptor.literal,
                &cell.binding,
            )
        {
            cells.push(cell);
        } else if !matches!(
            cell.ty,
            ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
        ) && !has_callable_holders(&cell.ty, state.structs)?
        {
            cells.push(cell);
        }
    }
    let mut seen = Vec::new();
    for capture in &cells {
        let cell = descriptor_cell_pointer(context, state, environment, capture.cell_offset)?;
        let unique = distinct_environment_cell(context, state, cell, &seen)?;
        seen.push(cell);
        let function = trace;
        let visit = context.append_basic_block(function, "environment.trace.unique");
        let next = context.append_basic_block(function, "environment.trace.next");
        state
            .builder
            .build_conditional_branch(unique, visit, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(visit);
        state
            .builder
            .build_indirect_call(
                context.void_type().fn_type(&[pointer.into()], false),
                visitor,
                &[cell.into()],
                "environment_trace_cell",
            )
            .map_err(builder_error)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
    }
    state.builder.build_return(None).map_err(builder_error)?;

    state
        .builder
        .position_at_end(context.append_basic_block(dispose, "entry"));
    let environment = dispose
        .get_first_param()
        .expect("dispose environment")
        .into_pointer_value();
    let mut seen = Vec::new();
    for capture in &cells {
        let cell = descriptor_cell_pointer(context, state, environment, capture.cell_offset)?;
        let unique = distinct_environment_cell(context, state, cell, &seen)?;
        seen.push(cell);
        let release = context.append_basic_block(dispose, "environment.cell.unique");
        let next = context.append_basic_block(dispose, "environment.cell.next");
        state
            .builder
            .build_conditional_branch(unique, release, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(release);
        drop_capture_cell(context, state, cell, capture.residence, &capture.ty)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
    }
    declare_core_runtime(context, state.module);
    let base = state
        .builder
        .build_load(
            pointer,
            environment_base_address(context, state, environment)?,
            "environment_allocation_base",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    state
        .builder
        .build_call(
            state
                .module
                .get_function(CORE_FREE_SYMBOL)
                .expect("core free declaration"),
            &[base.into()],
            "environment_allocation_release",
        )
        .map_err(builder_error)?;
    state.builder.build_return(None).map_err(builder_error)?;
    state.builder.position_at_end(caller);
    Ok(global)
}

use super::descriptors::{descriptor_cell_pointer, distinct_environment_cell};
pub(super) use super::descriptors::{
    validated_backing_maps, ValidatedBackingMaps, ValidatedCapture, ValidatedEscape,
};

pub(super) fn capture_value_key(value: &RuntimeValueId) -> String {
    format!("capture:{value:?}")
}

pub(super) fn missing_backing_transport(what: &str, span: (u32, u32)) -> String {
    format!(
        "missing validated backing transport for {what} at {}..{}",
        span.0, span.1
    )
}

pub(super) enum FixedCopyPublication {
    ConnectedHolder,
    RequiresHolderAction,
}

pub(super) fn materialize_fixed_copy<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    source: PointerValue<'ctx>,
    element: &ScalarType,
    length: u64,
    publication: FixedCopyPublication,
) -> Result<PointerValue<'ctx>, String> {
    if has_callable_holders(element, state.structs)?
        && matches!(publication, FixedCopyPublication::RequiresHolderAction)
    {
        return Err(format!(
            "callable fixed-array copy requires a connected source-qualified RetainArrayElements before publication at {:?}",
            state.source
        ));
    }
    let (element_size, element_alignment) =
        allocation_layout(element, state.structs, state.target_layout)?;
    let total = element_size
        .checked_mul(length)
        .ok_or_else(|| "runtime backing copy size overflows u64".to_owned())?;
    let destination = emit_runtime_array_allocation(
        context,
        state,
        element,
        context.i64_type().const_int(length, false),
    )?;
    state
        .builder
        .build_memcpy(
            destination,
            u32::try_from(element_alignment)
                .map_err(|_| "runtime backing copy alignment exceeds u32".to_owned())?,
            source,
            u32::try_from(element_alignment)
                .map_err(|_| "runtime backing copy alignment exceeds u32".to_owned())?,
            context.i64_type().const_int(total, false),
        )
        .map_err(builder_error)?;
    Ok(destination)
}

pub(super) fn retain_fixed_copy_elements<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    data: PointerValue<'ctx>,
    length: u64,
) -> Result<(), String> {
    let index_slot = entry_alloca(state, context.i64_type().into(), "fixed_copy_index_slot")?;
    state
        .builder
        .build_store(index_slot, context.i64_type().const_zero())
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("fixed copy block")
        .get_parent()
        .expect("fixed copy function");
    let test = context.append_basic_block(function, "fixed_copy.retain.test");
    let visit = context.append_basic_block(function, "fixed_copy.retain.visit");
    let done = context.append_basic_block(function, "fixed_copy.retain.done");
    state
        .builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    state.builder.position_at_end(test);
    let index = state
        .builder
        .build_load(context.i64_type(), index_slot, "fixed_copy_walk_index")
        .map_err(builder_error)?
        .into_int_value();
    let inside = state
        .builder
        .build_int_compare(
            IntPredicate::ULT,
            index,
            context.i64_type().const_int(length, false),
            "fixed_copy_in_bounds",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(inside, visit, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(visit);
    let callable_type = context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            context.ptr_type(AddressSpace::default()).into(),
        ],
        false,
    );
    let element = unsafe {
        state
            .builder
            .build_in_bounds_gep(callable_type, data, &[index], "fixed_copy_element")
    }
    .map_err(builder_error)?;
    let callable = state
        .builder
        .build_load(callable_type, element, "fixed_copy_callable")
        .map_err(builder_error)?
        .into_struct_value();
    let environment = state
        .builder
        .build_extract_value(callable, 1, "fixed_copy_retained_environment")
        .map_err(builder_error)?
        .into_pointer_value();
    retain_environment(context, state, environment)?;
    install_callable_array_element(context, state, data, index)?;
    let next = state
        .builder
        .build_int_add(
            index,
            context.i64_type().const_int(1, false),
            "fixed_copy_next_index",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(index_slot, next)
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn callable_array_bitmap<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    data: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    let pointer_size = state.target_layout.pointer_size;
    let header = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            data,
            &[context
                .i64_type()
                .const_int((pointer_size * 2).wrapping_neg(), false)],
            "callable_array_header",
        )
    }
    .map_err(builder_error)?;
    let loaded = state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            unsafe {
                state.builder.build_in_bounds_gep(
                    context.i8_type(),
                    header,
                    &[context.i64_type().const_int(pointer_size, false)],
                    "callable_array_bitmap_field",
                )
            }
            .map_err(builder_error)?,
            "callable_array_bitmap",
        )
        .map_err(builder_error)?;
    loaded
        .as_instruction_value()
        .expect("bitmap load instruction")
        .set_alignment(pointer_size as u32)
        .map_err(|error| error.to_string())?;
    Ok(loaded.into_pointer_value())
}

pub(super) fn callable_array_element_flag<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    data: PointerValue<'ctx>,
    index: inkwell::values::IntValue<'ctx>,
) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), String> {
    let bitmap = callable_array_bitmap(context, state, data)?;
    let byte_index = state
        .builder
        .build_right_shift(
            index,
            context.i64_type().const_int(3, false),
            false,
            "callable_flag_byte_index",
        )
        .map_err(builder_error)?;
    let bit_index = state
        .builder
        .build_and(
            index,
            context.i64_type().const_int(7, false),
            "callable_flag_bit_index",
        )
        .map_err(builder_error)?;
    let byte = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            bitmap,
            &[byte_index],
            "callable_flag_byte",
        )
    }
    .map_err(builder_error)?;
    let mask = state
        .builder
        .build_left_shift(
            context.i8_type().const_int(1, false),
            state
                .builder
                .build_int_truncate(bit_index, context.i8_type(), "callable_flag_bit")
                .map_err(builder_error)?,
            "callable_flag_mask",
        )
        .map_err(builder_error)?;
    Ok((byte, mask))
}

pub(super) fn callable_array_element_installed<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    data: PointerValue<'ctx>,
    index: inkwell::values::IntValue<'ctx>,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let (byte, mask) = callable_array_element_flag(context, state, data, index)?;
    let bits = state
        .builder
        .build_load(context.i8_type(), byte, "callable_flag_bits")
        .map_err(builder_error)?
        .into_int_value();
    let marked = state
        .builder
        .build_and(bits, mask, "callable_flag_marked")
        .map_err(builder_error)?;
    state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            marked,
            context.i8_type().const_zero(),
            "callable_element_installed",
        )
        .map_err(builder_error)
}

pub(super) fn install_callable_array_element<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    data: PointerValue<'ctx>,
    index: inkwell::values::IntValue<'ctx>,
) -> Result<(), String> {
    let (byte, mask) = callable_array_element_flag(context, state, data, index)?;
    let bits = state
        .builder
        .build_load(context.i8_type(), byte, "callable_flag_old_bits")
        .map_err(builder_error)?
        .into_int_value();
    let installed = state
        .builder
        .build_or(bits, mask, "callable_flag_installed")
        .map_err(builder_error)?;
    state
        .builder
        .build_store(byte, installed)
        .map_err(builder_error)?;
    Ok(())
}

pub(super) fn callable_array_allocation_base<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    data: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    let header = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            data,
            &[context
                .i64_type()
                .const_int((state.target_layout.pointer_size * 2).wrapping_neg(), false)],
            "callable_array_header",
        )
    }
    .map_err(builder_error)?;
    let loaded = state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            header,
            "callable_array_allocation_base",
        )
        .map_err(builder_error)?;
    loaded
        .as_instruction_value()
        .expect("allocation-base load instruction")
        .set_alignment(state.target_layout.pointer_alignment as u32)
        .map_err(|error| error.to_string())?;
    Ok(loaded.into_pointer_value())
}

pub(super) fn owned_backing_value_facts(facts: &[RuntimeBackingFact]) -> Vec<RuntimeValueId> {
    facts
        .iter()
        .filter(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
        .map(|fact| fact.value.clone())
        .collect()
}

pub(super) fn fresh_return_outputs(
    function: &ScalarFunction,
) -> Result<
    (
        BTreeSet<usize>,
        BTreeMap<(u32, u32), (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>)>,
    ),
    String,
> {
    let mut result = BTreeSet::new();
    let mut external_outputs = BTreeSet::new();
    for point in &function.reference_cfg.points {
        let CfgPointKind::ReturnOutput {
            output,
            checked: true,
            ..
        } = point.kind
        else {
            continue;
        };
        let origins = point
            .possible_origins
            .as_ref()
            .ok_or_else(|| format!("missing validated return origin at {:?}", point.source_span))?;
        let mut local = false;
        let mut external = false;
        for origin in origins {
            match &function.reference_cfg.facts[origin.0] {
                ReferenceOrigin::Fresh { allocation, .. }
                    if allocation.function_span == function.span
                        && allocation.source == point.source_span.source =>
                {
                    local = true
                }
                ReferenceOrigin::Fresh { .. } | ReferenceOrigin::BorrowedFrom { .. } => {
                    external = true;
                }
                ReferenceOrigin::Null { .. } => {}
                ReferenceOrigin::Invalid { .. } => {
                    return Err(format!(
                        "invalid validated return origin at {:?}",
                        point.source_span
                    ));
                }
            }
        }
        if local {
            result.insert(output);
        }
        if external {
            external_outputs.insert(output);
        }
    }
    let mixed = result
        .intersection(&external_outputs)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut branches = BTreeMap::new();
    for output in &mixed {
        let final_output = function
            .body
            .final_output_values
            .get(*output)
            .ok_or_else(|| format!("missing return output {output}"))?;
        record_return_branches(function, &final_output.value, *output, &mut branches)?;
    }
    result.retain(|output| !mixed.contains(output));
    Ok((result, branches))
}

pub(super) fn dynamic_return_outputs(function: &ScalarFunction) -> BTreeSet<usize> {
    function
        .reference_cfg
        .points
        .iter()
        .filter_map(|point| {
            let CfgPointKind::ReturnOutput {
                output,
                checked: true,
                ..
            } = point.kind
            else {
                return None;
            };
            let Some(origins) = &point.possible_origins else {
                return None;
            };
            let fresh = origins.iter().any(|id| {
                matches!(
                    function.reference_cfg.facts[id.0],
                    ReferenceOrigin::Fresh { .. }
                )
            });
            let borrowed = origins.iter().any(|id| matches!(function.reference_cfg.facts[id.0], ReferenceOrigin::BorrowedFrom { .. }));
            let local = origins.iter().any(|id| matches!(&function.reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span == function.span && allocation.source == point.source_span.source));
            let external_fresh = origins.iter().any(|id| matches!(&function.reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span != function.span || allocation.source != point.source_span.source));
            (fresh && (borrowed || (local && external_fresh))).then_some(output)
        })
        .collect()
}

pub(super) fn dynamic_return_owner(function: &ScalarFunction) -> bool {
    !dynamic_return_outputs(function).is_empty()
}

pub(super) fn joined_return_binding_types(
    function: &ScalarFunction,
    branches: &BTreeMap<(u32, u32), (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>)>,
) -> Result<BTreeMap<String, ScalarType>, String> {
    let mut bindings = BTreeMap::new();
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return Err("function has no callable signature".to_owned());
    };
    for (_, _, joined) in branches.values() {
        for (output, name) in joined {
            let ty = outputs
                .outputs
                .get(*output)
                .ok_or_else(|| format!("missing joined return output {output}"))?
                .ty
                .clone();
            bindings.insert(name.clone(), ty);
        }
    }
    Ok(bindings)
}

pub(super) fn joined_assignment_owners(
    function: &ScalarFunction,
    bindings: &BTreeMap<String, ScalarType>,
) -> Result<BTreeMap<(u32, u32), CheckedAssignmentOwner>, String> {
    let mut assignments = BTreeMap::new();
    for point in &function.reference_cfg.points {
        let CfgPointKind::Assign { target, .. } = &point.kind else {
            continue;
        };
        let crate::ScalarPlace::Name { name, .. } = &target.place else {
            continue;
        };
        if !bindings.contains_key(name) {
            continue;
        }
        let origins = point.possible_origins.as_ref().ok_or_else(|| {
            format!(
                "missing validated assignment owner at {:?}",
                point.source_span
            )
        })?;
        let local = origins.iter().any(|id| matches!(&function.reference_cfg.facts[id.0],
            ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span == function.span && allocation.source == point.source_span.source));
        let external = origins.iter().any(|id| matches!(&function.reference_cfg.facts[id.0],
            ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span != function.span || allocation.source != point.source_span.source));
        let owner = match (local, external) {
            (true, false) => CheckedAssignmentOwner::Local,
            (false, true) => CheckedAssignmentOwner::External,
            (false, false) => CheckedAssignmentOwner::Borrowed,
            (true, true) => {
                return Err(format!(
                    "assignment at {:?} combines local and external owners",
                    point.source_span
                ))
            }
        };
        assignments.insert(
            (point.source_span.range.start, point.source_span.range.end),
            owner,
        );
    }
    Ok(assignments)
}

pub(super) fn initialize_joined_return_bindings<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    bindings: BTreeMap<String, ScalarType>,
) -> Result<(), String> {
    for (name, ty) in bindings {
        let slot = entry_alloca(
            state,
            context.bool_type().into(),
            &format!("{name}_return_owner"),
        )?;
        state
            .builder
            .build_store(slot, context.bool_type().const_zero())
            .map_err(builder_error)?;
        state.joined_return_bindings.insert(name, (slot, ty));
    }
    Ok(())
}

fn record_return_branches(
    function: &ScalarFunction,
    expression: &ScalarExpression,
    output: usize,
    branches: &mut BTreeMap<
        (u32, u32),
        (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>),
    >,
) -> Result<(), String> {
    let ScalarExpression::If {
        then_branch,
        else_branch,
        ..
    } = expression
    else {
        if let ScalarExpression::Name { name, .. } = expression {
            let point = function
                .reference_cfg
                .points
                .iter()
                .find(|point| {
                    matches!(point.kind,
                CfgPointKind::ReturnOutput { output: index, checked: true, .. } if index == output)
                })
                .ok_or_else(|| format!("missing checked return output {output}"))?;
            let origins = point
                .possible_origins
                .as_ref()
                .ok_or_else(|| format!("missing checked return origins for output {output}"))?;
            let local = origins.iter().any(|id| matches!(&function.reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { allocation, .. } if allocation.function_span == function.span && allocation.source == point.source_span.source));
            let external = origins
                .iter()
                .any(|id| match &function.reference_cfg.facts[id.0] {
                    ReferenceOrigin::BorrowedFrom { .. } => true,
                    ReferenceOrigin::Fresh { allocation, .. } => {
                        allocation.function_span != function.span
                            || allocation.source != point.source_span.source
                    }
                    ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
                });
            if local && external {
                let selected = branches
                    .entry((function.body.span.start, function.body.span.end))
                    .or_default();
                selected.1.insert(output);
                selected.2.insert(output, name.clone());
                return Ok(());
            }
        }
        return Err(format!(
            "return output {output} has no typed branch for its joined owners"
        ));
    };
    for branch in [then_branch, else_branch] {
        let [value] = branch.final_output_values.as_slice() else {
            return Err(format!("return branch has no unique output {output}"));
        };
        if matches!(value.value, ScalarExpression::If { .. }) {
            record_return_branches(function, &value.value, output, branches)?;
            continue;
        }
        let points = function
            .reference_cfg
            .points
            .iter()
            .filter(|point| {
                point.source_span.range == value.span
                    && match &value.value {
                        ScalarExpression::CheckedAddress { .. } => {
                            matches!(point.kind, CfgPointKind::Borrow { .. })
                        }
                        ScalarExpression::Name { name, .. } if name == "null" => {
                            matches!(point.kind, CfgPointKind::Evaluate(_))
                        }
                        ScalarExpression::Name { .. } => {
                            matches!(point.kind, CfgPointKind::Read { .. })
                        }
                        ScalarExpression::Call { .. } => {
                            matches!(point.kind, CfgPointKind::Call { output: 0, .. })
                        }
                        _ => matches!(point.kind, CfgPointKind::Evaluate(_)),
                    }
                    && point
                        .possible_origins
                        .as_ref()
                        .is_some_and(|origins| !origins.is_empty())
            })
            .collect::<Vec<_>>();
        let [point] = points.as_slice() else {
            return Err(format!(
                "return branch at {:?} has {} typed origin points",
                value.span,
                points.len()
            ));
        };
        let origins = point
            .possible_origins
            .as_ref()
            .expect("checked typed point");
        let local = origins.iter().any(|id| matches!(
            &function.reference_cfg.facts[id.0],
            ReferenceOrigin::Fresh { allocation, .. }
                if allocation.function_span == function.span && allocation.source == point.source_span.source
        ));
        let external = origins
            .iter()
            .any(|id| match &function.reference_cfg.facts[id.0] {
                ReferenceOrigin::BorrowedFrom { .. } => true,
                ReferenceOrigin::Fresh { allocation, .. } => {
                    allocation.function_span != function.span
                        || allocation.source != point.source_span.source
                }
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
            });
        let selected = branches
            .entry((branch.span.start, branch.span.end))
            .or_default();
        selected.1.insert(output);
        if local && external {
            let ScalarExpression::Name { name, .. } = &value.value else {
                return Err(format!(
                    "return branch at {:?} has joined owners without a binding",
                    value.span
                ));
            };
            selected.2.insert(output, name.clone());
            continue;
        }
        if local {
            selected.0.insert(output);
        }
    }
    Ok(())
}

pub(super) fn checked_call_owners(
    function: &ScalarFunction,
) -> Result<BTreeMap<(u32, u32, usize), CheckedCallOwner>, String> {
    let mut owners = BTreeMap::new();
    for point in &function.reference_cfg.points {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { span, .. },
            output,
            ..
        } = &point.kind
        else {
            continue;
        };
        let Some(origins) = &point.possible_origins else {
            continue;
        };
        let owned = |id: &crate::scalar::ReferenceOriginId| {
            matches!(
                &function.reference_cfg.facts[id.0],
                ReferenceOrigin::Fresh { loan, .. }
                    if matches!(loan.origin_place.binding.kind, ReferenceBindingKind::CallOutput { point: origin, output: call_output } if origin == point.id && call_output == *output)
            )
        };
        let fresh = origins.iter().any(owned);
        let borrowed = origins
            .iter()
            .any(|id| match &function.reference_cfg.facts[id.0] {
                ReferenceOrigin::Null { .. } => false,
                _ => !owned(id),
            });
        let owner = match (fresh, borrowed) {
            (true, true) => CheckedCallOwner::Mixed,
            (true, false) => CheckedCallOwner::Fresh,
            (false, _) => CheckedCallOwner::Borrowed,
        };
        owners.insert((span.start, span.end, *output), owner);
    }
    Ok(owners)
}

pub(super) fn return_transport(
    state: &EmitState<'_, '_>,
    output: usize,
    shared: bool,
) -> Result<(RuntimeReturnBackingAction, Vec<RuntimeBackingFact>), String> {
    if let Some(action) = state.backing.returns.get(&output) {
        return Ok((
            action.clone(),
            state
                .backing
                .return_facts
                .get(&output)
                .cloned()
                .unwrap_or_default(),
        ));
    }
    if shared {
        if let Some(action) = state.backing.returns.get(&0) {
            return Ok((
                action.clone(),
                state
                    .backing
                    .return_facts
                    .get(&0)
                    .cloned()
                    .unwrap_or_default(),
            ));
        }
    }
    Err(missing_backing_transport(
        "return output",
        (output as u32, output as u32),
    ))
}

fn drop_local_callable_array_elements<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
    data: PointerValue<'ctx>,
    element: &ScalarType,
) -> Result<(), String> {
    let (slot, _) = state
        .storage
        .get(name)
        .ok_or_else(|| format!("local callable array `{name}` lacks bound storage"))?;
    let owners = state
        .holder_events
        .bindings
        .iter()
        .filter(|(_, address)| *address == slot)
        .map(|(binding, _)| binding)
        .collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err(format!(
            "local callable array `{name}` lacks unique source-qualified owner"
        ));
    };
    let backings = state.holder_events.array_cleanup_owner(owner)?;
    let mut literals = Vec::new();
    for backing in &backings {
        if backing.callable_family.len() != 1
            || !matches!(
                (&backing.callable_family[0].element, element),
                (ScalarType::Callable { .. }, ScalarType::Callable { .. })
            )
        {
            return Err(format!(
                "local callable array `{name}` has unrelated connected element cleanup at {:?}",
                backing.origin
            ));
        }
        for partition in &backing.callable_family[0].partitions {
            for case in partition.remainder.alternatives.iter().chain(
                partition
                    .representatives
                    .iter()
                    .flat_map(|representative| &representative.cell.alternatives),
            ) {
                if let crate::scalar::CallableCellCaseFact::Initialized {
                    environment_origin, ..
                } = case
                {
                    if !literals.contains(&environment_origin.literal) {
                        literals.push(environment_origin.literal.clone());
                    }
                }
            }
        }
    }
    for literal in &literals {
        if !state.literal_captures.contains_key(literal) {
            return Err(format!(
                "source-qualified callable array lacks producer at {literal:?}"
            ));
        }
    }
    let length_slot = *state
        .runtime_array_lengths
        .get(name)
        .ok_or_else(|| format!("local callable array `{name}` lacks evaluated length"))?;
    let length = state
        .builder
        .build_load(
            context.i64_type(),
            length_slot,
            "callable_array_private_length",
        )
        .map_err(builder_error)?
        .into_int_value();
    let index_slot = entry_alloca(
        state,
        context.i64_type().into(),
        "callable_array_drop_index",
    )?;
    state
        .builder
        .build_store(index_slot, context.i64_type().const_zero())
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .expect("array drop block")
        .get_parent()
        .expect("array drop function");
    let test = context.append_basic_block(function, "callable_array.drop.test");
    let visit = context.append_basic_block(function, "callable_array.drop.visit");
    let installed = context.append_basic_block(function, "callable_array.drop.installed");
    let step = context.append_basic_block(function, "callable_array.drop.step");
    let done = context.append_basic_block(function, "callable_array.drop.done");
    state
        .builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    state.builder.position_at_end(test);
    let index = state
        .builder
        .build_load(context.i64_type(), index_slot, "callable_array_walk_index")
        .map_err(builder_error)?
        .into_int_value();
    let in_bounds = state
        .builder
        .build_int_compare(
            IntPredicate::ULT,
            index,
            length,
            "callable_array_walk_in_bounds",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(in_bounds, visit, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(visit);
    let marked = callable_array_element_installed(context, state, data, index)?;
    state
        .builder
        .build_conditional_branch(marked, installed, step)
        .map_err(builder_error)?;
    state.builder.position_at_end(installed);
    let (size, _) = allocation_layout(element, state.structs, state.target_layout)?;
    let offset = state
        .builder
        .build_int_mul(
            index,
            context.i64_type().const_int(size, false),
            "callable_array_element_offset",
        )
        .map_err(builder_error)?;
    let address = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            data,
            &[offset],
            "callable_array_installed_address",
        )
    }
    .map_err(builder_error)?;
    let ty = context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            context.ptr_type(AddressSpace::default()).into(),
        ],
        false,
    );
    let callable = state
        .builder
        .build_load(ty, address, "callable_array_installed_holder")
        .map_err(builder_error)?
        .into_struct_value();
    let environment = state
        .builder
        .build_extract_value(callable, 1, "callable_array_installed_environment")
        .map_err(builder_error)?
        .into_pointer_value();
    drop_environment(context, state, environment)?;
    state
        .builder
        .build_unconditional_branch(step)
        .map_err(builder_error)?;
    state.builder.position_at_end(step);
    let next = state
        .builder
        .build_int_add(
            index,
            context.i64_type().const_int(1, false),
            "callable_array_next_index",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(index_slot, next)
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn release_runtime_array_owner<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
) -> Result<(), String> {
    if !state
        .runtime_array_owners
        .get(name)
        .copied()
        .unwrap_or(false)
    {
        return Ok(());
    }
    let (slot, ty) = state
        .storage
        .get(name)
        .cloned()
        .or_else(|| state.backing_slots.get(name).cloned())
        .ok_or_else(|| format!("missing runtime-array storage for {name}"))?;
    let ScalarType::RuntimeArray { element, .. } = &ty else {
        return Err(format!("runtime-array owner {name} has an invalid type"));
    };
    let prefixed = has_callable_holders(element, state.structs)?;
    if prefixed
        && !matches!(element.as_ref(), ScalarType::Callable { .. })
        && !state.backing_captured.contains(name)
    {
        return Err(format!(
            "local callable array `{name}` requires a connected nested holder walk"
        ));
    }
    declare_core_runtime(context, state.module);
    if state.backing_captured.contains(name) {
        let slot = if let Some(imported) = state.imported_cell_slots.get(name) {
            state
                .builder
                .build_load(
                    context.ptr_type(AddressSpace::default()),
                    *imported,
                    "imported_capture_cell",
                )
                .map_err(builder_error)?
                .into_pointer_value()
        } else {
            slot
        };
        let block = state
            .builder
            .get_insert_block()
            .ok_or("missing captured release block")?;
        let function = block
            .get_parent()
            .ok_or("missing captured release function")?;
        let check = context.append_basic_block(function, "capture_release.check");
        let free = context.append_basic_block(function, "capture_release.free");
        let next = context.append_basic_block(function, "capture_release.next");
        let cell_present = state
            .builder
            .build_is_not_null(slot, "capture_cell_present")
            .map_err(builder_error)?;
        state
            .builder
            .build_conditional_branch(cell_present, check, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(check);
        let backing = state
            .builder
            .build_load(
                context.ptr_type(AddressSpace::default()),
                slot,
                "captured_backing",
            )
            .map_err(builder_error)?
            .into_pointer_value();
        let backing_present = state
            .builder
            .build_is_not_null(backing, "capture_backing_present")
            .map_err(builder_error)?;
        state
            .builder
            .build_conditional_branch(backing_present, free, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(free);
        let backing = if prefixed {
            callable_array_allocation_base(context, state, backing)?
        } else {
            backing
        };
        let release = state
            .module
            .get_function(CORE_FREE_SYMBOL)
            .ok_or("core free runtime declaration is missing")?;
        state
            .builder
            .build_call(release, &[backing.into()], "runtime_array_release")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(slot, context.ptr_type(AddressSpace::default()).const_null())
            .map_err(builder_error)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
        state.runtime_array_owners.insert(name.to_owned(), false);
        return Ok(());
    }
    let pointer = if state
        .runtime_array_allocations
        .get(name)
        .copied()
        .unwrap_or(false)
    {
        slot.into()
    } else {
        state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                slot,
                "runtime_array_owner",
            )
            .map_err(builder_error)?
    };
    let release = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    if let Some(live) = state.runtime_array_live.get(name) {
        let live = state
            .builder
            .build_load(context.bool_type(), *live, "allocation_live")
            .map_err(builder_error)?
            .into_int_value();
        let block = state
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing allocation release block".to_owned())?;
        let function = block
            .get_parent()
            .ok_or_else(|| "missing allocation release function".to_owned())?;
        let free = context.append_basic_block(function, "release.live");
        let next = context.append_basic_block(function, "release.next");
        state
            .builder
            .build_conditional_branch(live, free, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(free);
        if prefixed {
            drop_local_callable_array_elements(
                context,
                state,
                name,
                pointer.into_pointer_value(),
                element,
            )?;
        }
        let pointer = if prefixed {
            callable_array_allocation_base(context, state, pointer.into_pointer_value())?
        } else {
            pointer.into_pointer_value()
        };
        state
            .builder
            .build_call(release, &[pointer.into()], "runtime_array_release")
            .map_err(builder_error)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
        state.runtime_array_owners.insert(name.to_owned(), false);
        return Ok(());
    }
    let pointer = if prefixed {
        drop_local_callable_array_elements(
            context,
            state,
            name,
            pointer.into_pointer_value(),
            element,
        )?;
        callable_array_allocation_base(context, state, pointer.into_pointer_value())?
    } else {
        pointer.into_pointer_value()
    };
    state
        .builder
        .build_call(release, &[pointer.into()], "runtime_array_release")
        .map_err(builder_error)?;
    state.runtime_array_owners.insert(name.to_owned(), false);
    Ok(())
}

pub(super) fn release_captured_backings<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
) -> Result<(), String> {
    loop {
        let next = state
            .runtime_array_owners
            .iter()
            .filter(|(name, owned)| **owned && state.backing_captured.contains(*name))
            .map(|(name, _)| name.clone())
            .next();
        let Some(name) = next else {
            break;
        };
        super::release_backing_entry(context, state, &name)?;
    }
    Ok(())
}

pub(super) fn release_scope_runtime_array_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outer_owners: &BTreeMap<String, bool>,
    outer_names: &BTreeSet<String>,
) -> Result<(), String> {
    loop {
        let next = state
            .runtime_array_owners
            .iter()
            .filter(|(name, owned)| {
                **owned
                    && !outer_owners.contains_key(*name)
                    && !state.backing_captured.contains(*name)
                    && !outer_names.contains(super::backing_entry_root(name))
            })
            .map(|(name, _)| name.clone())
            .next();
        let Some(name) = next else {
            break;
        };
        super::release_backing_entry(context, state, &name)?;
    }
    Ok(())
}

pub(super) fn clear_transferred_backings(
    state: &mut EmitState<'_, '_>,
    facts: &[RuntimeBackingFact],
) {
    let values = facts
        .iter()
        .filter(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation)
        .map(|fact| &fact.value)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    for name in state
        .runtime_array_owners
        .keys()
        .cloned()
        .collect::<Vec<_>>()
    {
        let owned = state
            .runtime_array_owners
            .get(&name)
            .copied()
            .unwrap_or(false);
        if !owned {
            continue;
        }
        let tracked = state
            .backing_values
            .get(&name)
            .map(|entry| entry.iter().any(|value| values.contains(&value)))
            .unwrap_or(false);
        if tracked {
            super::drop_backing_entry(state, &name);
        }
    }
}

pub(super) fn transfer_runtime_array_return(
    state: &mut EmitState<'_, '_>,
    output: usize,
    ty: &ScalarType,
    shared: bool,
    block: &ScalarBlock,
    expression: &ScalarExpression,
) -> Result<(), String> {
    let (action, facts) = return_transport(state, output, shared)?;
    match action {
        RuntimeReturnBackingAction::Transfer => {
            if facts.is_empty() {
                fallback_escaped_return(state, block, expression, ty);
                return Ok(());
            }
            clear_transferred_backings(state, &facts);
            if matches!(ty, ScalarType::Struct(_)) {
                for key in state.backing_values.keys().cloned().collect::<Vec<_>>() {
                    if key.contains('.')
                        && state
                            .backing_values
                            .get(&key)
                            .map(|v| v.is_empty())
                            .unwrap_or(true)
                    {
                        super::drop_backing_entry(state, &key);
                    }
                }
            }
        }
        RuntimeReturnBackingAction::CopyFixed { .. }
        | RuntimeReturnBackingAction::MoveFixed { .. } => {
            if !matches!(ty, ScalarType::RuntimeArray { .. }) {
                return Err(format!(
                    "validated fixed return transport lacks a runtime-array output {output}"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn materialize_validated_return<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    output: usize,
    shared: bool,
    expression: &ScalarExpression,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let (action, _) = return_transport(state, output, shared)?;
    match action {
        RuntimeReturnBackingAction::Transfer => Ok(value),
        RuntimeReturnBackingAction::CopyFixed { .. }
        | RuntimeReturnBackingAction::MoveFixed { .. } => {
            let ScalarExpression::Name { name, .. } = expression else {
                return Err(format!(
                    "validated fixed return transport needs a named source for output {output}"
                ));
            };
            let source_ty = state
                .storage
                .get(name)
                .map(|(_, ty)| ty.clone())
                .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone()))
                .ok_or_else(|| {
                    missing_backing_transport(
                        &format!("fixed return source {name}"),
                        (output as u32, output as u32),
                    )
                })?;
            let ScalarType::Array {
                element, length, ..
            } = source_ty
            else {
                return Err(format!(
                    "validated fixed return transport needs an array source for output {output}"
                ));
            };
            let source = take_basic(value)?.into_pointer_value();
            let destination = materialize_fixed_copy(
                context,
                state,
                source,
                &element,
                length,
                FixedCopyPublication::RequiresHolderAction,
            )?;
            Ok(EmitValue::Basic(destination.into()))
        }
    }
}

fn fallback_escaped_return(
    state: &mut EmitState<'_, '_>,
    block: &ScalarBlock,
    expression: &ScalarExpression,
    ty: &ScalarType,
) {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        let bindings = return_bindings(block);
        let mut visited = BTreeSet::new();
        for name in escaped_runtime_array_owners(expression, &bindings, &mut visited) {
            if state
                .runtime_array_owners
                .get(&name)
                .copied()
                .unwrap_or(false)
            {
                super::drop_backing_entry(state, &name);
            }
        }
        return;
    }
    if let ScalarExpression::Name { name, .. } = expression {
        if state
            .runtime_array_owners
            .get(name)
            .copied()
            .unwrap_or(false)
        {
            super::drop_backing_entry(state, name);
        }
    }
}

pub(super) fn return_bindings(block: &ScalarBlock) -> BTreeMap<String, ScalarExpression> {
    let mut bindings = BTreeMap::new();
    for item in &block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let receivers = binding_receivers(binding);
                if receivers.len() == 1 {
                    bindings.insert(receivers[0].0.clone(), binding.value.clone());
                } else if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions
                {
                    for ((name, _), output) in receivers.iter().zip(&binding.output_values) {
                        bindings.insert(name.clone(), output.value.clone());
                    }
                }
            }
            ScalarBlockItem::Assignment(assignment) => {
                for (target, value) in assignment.targets.iter().zip(&assignment.values) {
                    if let crate::ScalarPlace::Name { name, .. } = &target.place {
                        bindings.insert(name.clone(), value.clone());
                    }
                }
            }
            ScalarBlockItem::Expression(expression) => {
                extend_return_bindings_from_expression(&mut bindings, expression);
            }
            ScalarBlockItem::While(while_expression) => {
                extend_return_bindings_from_expression(&mut bindings, &while_expression.condition);
            }
        }
    }
    bindings
}

fn extend_return_bindings_from_expression(
    bindings: &mut BTreeMap<String, ScalarExpression>,
    expression: &ScalarExpression,
) {
    match expression {
        ScalarExpression::Block(block) => bindings.extend(return_bindings(block)),
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => {
            bindings.extend(return_bindings(then_branch));
            bindings.extend(return_bindings(else_branch));
        }
        ScalarExpression::UnitIf { then_branch, .. } => {
            bindings.extend(return_bindings(then_branch));
        }
        ScalarExpression::FunctionLiteral { .. }
        | ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. }
        | ScalarExpression::RawAddress { .. }
        | ScalarExpression::CheckedAddress { .. }
        | ScalarExpression::Dereference { .. }
        | ScalarExpression::IndexedRead { .. }
        | ScalarExpression::StructLiteral { .. }
        | ScalarExpression::ArrayLiteral { .. }
        | ScalarExpression::Binary { .. }
        | ScalarExpression::Unary { .. }
        | ScalarExpression::Call { .. } => {}
        ScalarExpression::Assignment(assignment) => {
            for (target, value) in assignment.targets.iter().zip(&assignment.values) {
                if let crate::ScalarPlace::Name { name, .. } = &target.place {
                    bindings.insert(name.clone(), value.clone());
                }
            }
        }
    }
}

fn escaped_runtime_array_owners(
    expression: &ScalarExpression,
    bindings: &BTreeMap<String, ScalarExpression>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            if !visited.insert(name.clone()) {
                return BTreeSet::new();
            }
            let owners = match bindings.get(name) {
                Some(value) => escaped_runtime_array_owners(value, bindings, visited),
                None => BTreeSet::new(),
            };
            visited.remove(name);
            owners
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => {
            escaped_runtime_array_owners_in_place(place, bindings, visited)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            fields.iter().fold(BTreeSet::new(), |mut owners, field| {
                owners.extend(escaped_runtime_array_owners(
                    &field.value,
                    bindings,
                    visited,
                ));
                owners
            })
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            elements
                .iter()
                .fold(BTreeSet::new(), |mut owners, element| {
                    owners.extend(escaped_runtime_array_owners(element, bindings, visited));
                    owners
                })
        }
        ScalarExpression::Assignment(assignment) => {
            assignment.values.iter().fold(BTreeSet::new(), |mut owners, value| {
                owners.extend(escaped_runtime_array_owners(value, bindings, visited));
                owners
            })
        }
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => {
            let mut owners = escaped_runtime_array_owners_in_block(then_branch, bindings, visited);
            owners.extend(escaped_runtime_array_owners_in_block(
                else_branch,
                bindings,
                visited,
            ));
            owners
        }
        ScalarExpression::Block(block) => {
            escaped_runtime_array_owners_in_block(block, bindings, visited)
        }
        ScalarExpression::Dereference { place, .. }
        | ScalarExpression::IndexedRead { place, .. } => {
            escaped_runtime_array_owners_in_place(place, bindings, visited)
        }
        ScalarExpression::FunctionLiteral { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. }
        | ScalarExpression::Binary { .. }
        | ScalarExpression::Unary { .. }
        | ScalarExpression::Call { .. }
        | ScalarExpression::UnitIf { .. } => BTreeSet::new(),
    }
}

fn escaped_runtime_array_owners_in_place(
    place: &crate::ScalarPlace,
    bindings: &BTreeMap<String, ScalarExpression>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => {
            let mut owners = BTreeSet::from([name.clone()]);
            if let Some(value) = bindings.get(name) {
                owners.extend(escaped_runtime_array_owners(value, bindings, visited));
            }
            owners
        }
        crate::ScalarPlace::Field { base, .. } | crate::ScalarPlace::Index { base, .. } => {
            escaped_runtime_array_owners_in_place(base, bindings, visited)
        }
        crate::ScalarPlace::Dereference { pointer, .. } => {
            escaped_runtime_array_owners(pointer, bindings, visited)
        }
    }
}

fn escaped_runtime_array_owners_in_block(
    block: &ScalarBlock,
    bindings: &BTreeMap<String, ScalarExpression>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    let mut local_bindings = bindings.clone();
    local_bindings.extend(return_bindings(block));
    block
        .final_output_values
        .iter()
        .fold(BTreeSet::new(), |mut owners, output| {
            owners.extend(escaped_runtime_array_owners(
                &output.value,
                &local_bindings,
                visited,
            ));
            owners
        })
}

pub(super) fn emit_runtime_array_allocation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let (element_size, alignment) = allocation_layout(element, state.structs, state.target_layout)?;
    let prefixed = has_callable_holders(element, state.structs)?;
    let (size, allocation_alignment, bitmap_bytes) = if prefixed {
        let allocation_alignment = alignment.max(state.target_layout.pointer_alignment);
        let header_size = state.target_layout.pointer_size * 2;
        let fixed_overhead = header_size
            .checked_add(allocation_alignment - 1)
            .ok_or_else(|| "callable-array prefix size overflows u64".to_owned())?;
        let limit = if state.target_layout.pointer_size == 4 {
            u32::MAX as u64
        } else {
            u64::MAX
                .checked_sub(allocation_alignment - 1)
                .and_then(|size| size.checked_sub(state.target_layout.pointer_size))
                .ok_or_else(|| "callable-array allocator limit is invalid".to_owned())?
        };
        let maximum_bitmap_bytes = limit
            .checked_sub(fixed_overhead)
            .ok_or_else(|| "callable-array prefix exceeds allocator limit".to_owned())?;
        let bitmap_bytes = state
            .builder
            .build_int_unsigned_div(
                length,
                context.i64_type().const_int(8, false),
                "callable_bitmap_full_bytes",
            )
            .map_err(builder_error)?;
        let remainder = state
            .builder
            .build_int_unsigned_rem(
                length,
                context.i64_type().const_int(8, false),
                "callable_bitmap_remainder",
            )
            .map_err(builder_error)?;
        let extra = state
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                remainder,
                context.i64_type().const_zero(),
                "callable_bitmap_extra",
            )
            .map_err(builder_error)?;
        let extra = state
            .builder
            .build_int_z_extend(extra, context.i64_type(), "callable_bitmap_extra_byte")
            .map_err(builder_error)?;
        let bitmap_bytes = state
            .builder
            .build_int_add(bitmap_bytes, extra, "callable_bitmap_bytes")
            .map_err(builder_error)?;
        let maximum_bitmap = state
            .builder
            .build_int_compare(
                inkwell::IntPredicate::ULE,
                bitmap_bytes,
                context.i64_type().const_int(maximum_bitmap_bytes, false),
                "callable_bitmap_fits",
            )
            .map_err(builder_error)?;
        let prefix = state
            .builder
            .build_int_add(
                bitmap_bytes,
                context.i64_type().const_int(fixed_overhead, false),
                "callable_prefix_limit",
            )
            .map_err(builder_error)?;
        let available = state
            .builder
            .build_int_sub(
                context.i64_type().const_int(limit, false),
                prefix,
                "callable_payload_capacity",
            )
            .map_err(builder_error)?;
        let maximum_length = if element_size == 0 {
            context.i64_type().const_int(u64::MAX, false)
        } else {
            state
                .builder
                .build_int_unsigned_div(
                    available,
                    context.i64_type().const_int(element_size, false),
                    "callable_maximum_length",
                )
                .map_err(builder_error)?
        };
        let length_fits = state
            .builder
            .build_int_compare(
                inkwell::IntPredicate::ULE,
                length,
                maximum_length,
                "callable_length_fits",
            )
            .map_err(builder_error)?;
        let fits = state
            .builder
            .build_and(maximum_bitmap, length_fits, "callable_allocation_fits")
            .map_err(builder_error)?;
        let function = state
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| "runtime allocation has no containing function".to_owned())?;
        let accepted = context.append_basic_block(function, "callable_allocation_accepted");
        let rejected = context.append_basic_block(function, "callable_allocation_rejected");
        state
            .builder
            .build_conditional_branch(fits, accepted, rejected)
            .map_err(builder_error)?;
        state.builder.position_at_end(rejected);
        let panic = state
            .module
            .get_function(CORE_SYSTEM_PANIC_SYMBOL)
            .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
        state
            .builder
            .build_call(panic, &[], "system_panic")
            .map_err(builder_error)?;
        state.builder.build_unreachable().map_err(builder_error)?;
        state.builder.position_at_end(accepted);
        let payload = state
            .builder
            .build_int_mul(
                length,
                context.i64_type().const_int(element_size, false),
                "callable_payload_size",
            )
            .map_err(builder_error)?;
        let size = state
            .builder
            .build_int_add(prefix, payload, "callable_allocation_size")
            .map_err(builder_error)?;
        (size, allocation_alignment, Some(bitmap_bytes))
    } else {
        let size = state
            .builder
            .build_int_mul(
                length,
                context.i64_type().const_int(element_size, false),
                "allocation_size",
            )
            .map_err(builder_error)?;
        (size, alignment, None)
    };
    let allocator = state
        .module
        .get_function(CORE_ALLOC_SYMBOL)
        .ok_or_else(|| "core allocation runtime declaration is missing".to_owned())?;
    let allocation = state
        .builder
        .build_call(
            allocator,
            &[
                size.into(),
                context
                    .i64_type()
                    .const_int(allocation_alignment, false)
                    .into(),
            ],
            "allocation",
        )
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "core allocation runtime returned no pointer".to_owned())?
        .into_pointer_value();
    let failed = state
        .builder
        .build_is_null(allocation, "allocation_failed")
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .ok_or_else(|| "runtime allocation has no containing function".to_owned())?;
    let panic_block = context.append_basic_block(function, "allocation_panic");
    let continue_block = context.append_basic_block(function, "allocation_continue");
    state
        .builder
        .build_conditional_branch(failed, panic_block, continue_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(panic_block);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(panic, &[], "system_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(continue_block);
    let Some(bitmap_bytes) = bitmap_bytes else {
        return Ok(allocation);
    };
    state
        .builder
        .build_memset(allocation, 1, context.i8_type().const_zero(), bitmap_bytes)
        .map_err(builder_error)?;
    let header_size = state.target_layout.pointer_size * 2;
    let before_data = state
        .builder
        .build_int_add(
            bitmap_bytes,
            context.i64_type().const_int(header_size, false),
            "callable_before_data",
        )
        .map_err(builder_error)?;
    let rounded = state
        .builder
        .build_int_add(
            before_data,
            context
                .i64_type()
                .const_int(allocation_alignment - 1, false),
            "callable_round_data",
        )
        .map_err(builder_error)?;
    let aligned = state
        .builder
        .build_and(
            rounded,
            context
                .i64_type()
                .const_int(!(allocation_alignment - 1), false),
            "callable_data_offset",
        )
        .map_err(builder_error)?;
    let data = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            allocation,
            &[aligned],
            "callable_array_data",
        )
    }
    .map_err(builder_error)?;
    let header = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            data,
            &[context
                .i64_type()
                .const_int(header_size.wrapping_neg(), false)],
            "callable_array_header",
        )
    }
    .map_err(builder_error)?;
    state
        .builder
        .build_store(header, allocation)
        .map_err(builder_error)?
        .set_alignment(state.target_layout.pointer_alignment as u32)
        .map_err(|error| error.to_string())?;
    let bitmap_field = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            header,
            &[context
                .i64_type()
                .const_int(state.target_layout.pointer_size, false)],
            "callable_array_bitmap_field",
        )
    }
    .map_err(builder_error)?;
    state
        .builder
        .build_store(bitmap_field, allocation)
        .map_err(builder_error)?
        .set_alignment(state.target_layout.pointer_alignment as u32)
        .map_err(|error| error.to_string())?;
    Ok(data)
}

pub(super) fn binding_receivers(binding: &crate::ScalarBinding) -> Vec<(String, ScalarType)> {
    if binding.receivers.is_empty() {
        vec![(binding.name.clone(), binding.declared_type.clone())]
    } else {
        binding
            .receivers
            .iter()
            .map(|receiver| (receiver.name.clone(), receiver.ty.clone()))
            .collect()
    }
}
