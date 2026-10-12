use super::*;
use inkwell::basic_block::BasicBlock;
use inkwell::values::BasicValue;

pub(super) const GC_TRIAL_SYMBOL: &str = "__wosy_gc_trial";
pub(super) const GC_ENV_HEAD: &str = "__wosy_gc_env_head";
pub(super) const GC_CELL_HEAD: &str = "__wosy_gc_cell_head";
const GC_REGISTER_ENV: &str = "__wosy_gc_register_env";
const GC_REGISTER_CELL: &str = "__wosy_gc_register_cell";
const GC_UNREGISTER_ENV: &str = "__wosy_gc_unregister_env";
const GC_UNREGISTER_CELL: &str = "__wosy_gc_unregister_cell";
const GC_ENV_SUBTRACT: &str = "__wosy_gc_env_subtract";
const GC_CELL_SUBTRACT: &str = "__wosy_gc_cell_subtract";
const GC_ENV_MARK: &str = "__wosy_gc_env_mark";
const GC_CELL_MARK: &str = "__wosy_gc_cell_mark";
const GC_ENV_DETACH: &str = "__wosy_gc_env_detach";
const GC_CELL_DETACH: &str = "__wosy_gc_cell_detach";
const GC_ENV_RELEASE: &str = "__wosy_gc_env_release";
const GC_CELL_RELEASE: &str = "__wosy_gc_cell_release";

const ENV_GC_MARK_OFFSET: i64 = -32;
const ENV_GC_SAVED_OFFSET: i64 = -40;
const ENV_GC_NEXT_OFFSET: i64 = -48;
const CELL_GC_MARK_OFFSET: i64 = -40;
const CELL_GC_SAVED_OFFSET: i64 = -48;
const CELL_GC_NEXT_OFFSET: i64 = -56;
const ENV_DESCRIPTOR_OFFSET: i64 = -8;
const CELL_DESCRIPTOR_OFFSET: i64 = -32;
const ENV_COUNT_OFFSET: i64 = -24;
const CELL_COUNT_OFFSET: i64 = -24;
const ENV_BASE_OFFSET: i64 = -16;
const CELL_BASE_OFFSET: i64 = -16;
const CELL_OWNED_OFFSET: i64 = -8;

pub(super) const ENVIRONMENT_GC_PREFIX: u64 = 48;
pub(super) const CELL_GC_PREFIX: u64 = 56;

fn gc_header_address<'ctx>(
    builder: &Builder<'ctx>,
    context: &'ctx Context,
    node: PointerValue<'ctx>,
    offset: i64,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    unsafe {
        builder.build_in_bounds_gep(
            context.i8_type(),
            node,
            &[context.i64_type().const_int(offset as u64, false)],
            name,
        )
    }
    .map_err(builder_error)
}

pub(super) fn environment_gc_mark_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(
        &state.builder,
        context,
        environment,
        ENV_GC_MARK_OFFSET,
        "environment_gc_mark_address",
    )
}

pub(super) fn environment_gc_saved_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(
        &state.builder,
        context,
        environment,
        ENV_GC_SAVED_OFFSET,
        "environment_gc_saved_address",
    )
}

pub(super) fn environment_gc_next_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(
        &state.builder,
        context,
        environment,
        ENV_GC_NEXT_OFFSET,
        "environment_gc_next_address",
    )
}

pub(super) fn capture_cell_gc_mark_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(
        &state.builder,
        context,
        cell,
        CELL_GC_MARK_OFFSET,
        "capture_cell_gc_mark_address",
    )
}

pub(super) fn capture_cell_gc_saved_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(
        &state.builder,
        context,
        cell,
        CELL_GC_SAVED_OFFSET,
        "capture_cell_gc_saved_address",
    )
}

pub(super) fn capture_cell_gc_next_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(
        &state.builder,
        context,
        cell,
        CELL_GC_NEXT_OFFSET,
        "capture_cell_gc_next_address",
    )
}

pub(super) fn init_environment_gc_header<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<(), String> {
    state
        .builder
        .build_store(
            environment_gc_mark_address(context, state, environment)?,
            context.i64_type().const_zero(),
        )
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_store(
            environment_gc_saved_address(context, state, environment)?,
            context.i64_type().const_zero(),
        )
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_store(
            environment_gc_next_address(context, state, environment)?,
            context.ptr_type(AddressSpace::default()).const_null(),
        )
        .map_err(builder_error)?;
    Ok(())
}

pub(super) fn init_capture_cell_gc_header<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<(), String> {
    state
        .builder
        .build_store(
            capture_cell_gc_mark_address(context, state, cell)?,
            context.i64_type().const_zero(),
        )
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_store(
            capture_cell_gc_saved_address(context, state, cell)?,
            context.i64_type().const_zero(),
        )
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    state
        .builder
        .build_store(
            capture_cell_gc_next_address(context, state, cell)?,
            context.ptr_type(AddressSpace::default()).const_null(),
        )
        .map_err(builder_error)?;
    Ok(())
}

fn emit_gc_register_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    symbol: &str,
    node: PointerValue<'ctx>,
    name: &str,
) -> Result<(), String> {
    if node.is_null() {
        return Ok(());
    }
    let Some(register) = state.module.get_function(symbol) else {
        return Ok(());
    };
    let function = state
        .builder
        .get_insert_block()
        .expect("collector register block")
        .get_parent()
        .expect("collector register function");
    let present = context.append_basic_block(function, name);
    let done = context.append_basic_block(function, "gc.register.done");
    let live = state
        .builder
        .build_is_not_null(node, "gc_register_present")
        .map_err(builder_error)?;
    state
        .builder
        .build_conditional_branch(live, present, done)
        .map_err(builder_error)?;
    state.builder.position_at_end(present);
    state
        .builder
        .build_call(register, &[node.into()], "gc_registered")
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(done)
        .map_err(builder_error)?;
    state.builder.position_at_end(done);
    Ok(())
}

pub(super) fn register_environment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<(), String> {
    emit_gc_register_call(
        context,
        state,
        GC_REGISTER_ENV,
        environment,
        "gc.register.environment",
    )
}

pub(super) fn register_capture_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<(), String> {
    emit_gc_register_call(
        context,
        state,
        GC_REGISTER_CELL,
        cell,
        "gc.register.capture_cell",
    )
}

pub(super) fn unregister_environment<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
) -> Result<(), String> {
    if let Some(unregister) = state.module.get_function(GC_UNREGISTER_ENV) {
        state
            .builder
            .build_call(
                unregister,
                &[environment.into()],
                "gc_unregistered_environment",
            )
            .map_err(builder_error)?;
    }
    Ok(())
}

pub(super) fn unregister_capture_cell<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
) -> Result<(), String> {
    if let Some(unregister) = state.module.get_function(GC_UNREGISTER_CELL) {
        state
            .builder
            .build_call(unregister, &[cell.into()], "gc_unregistered_cell")
            .map_err(builder_error)?;
    }
    Ok(())
}

fn gc_node_address<'ctx>(
    builder: &Builder<'ctx>,
    context: &'ctx Context,
    node: PointerValue<'ctx>,
    offset: i64,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    gc_header_address(builder, context, node, offset, name)
}

pub(super) fn emit_gc_registry<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    for head in [GC_ENV_HEAD, GC_CELL_HEAD] {
        let global = module.add_global(pointer, None, head);
        global.set_linkage(Linkage::Private);
        global.set_initializer(&pointer.const_null());
    }
    for (symbol, next_offset) in [
        (GC_REGISTER_ENV, ENV_GC_NEXT_OFFSET),
        (GC_REGISTER_CELL, CELL_GC_NEXT_OFFSET),
    ] {
        let function = module.add_function(
            symbol,
            context.void_type().fn_type(&[pointer.into()], false),
            Some(Linkage::Internal),
        );
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(function, "gc.register.entry"));
        let node = function
            .get_first_param()
            .expect("collector register node")
            .into_pointer_value();
        let absent = builder
            .build_is_null(node, "gc_register_absent")
            .map_err(builder_error)?;
        let store = context.append_basic_block(function, "gc.register.store");
        let done = context.append_basic_block(function, "gc.register.done");
        builder
            .build_conditional_branch(absent, done, store)
            .map_err(builder_error)?;
        builder.position_at_end(store);
        let head = module
            .get_global(if symbol == GC_REGISTER_ENV {
                GC_ENV_HEAD
            } else {
                GC_CELL_HEAD
            })
            .expect("collector registry head");
        let previous = builder
            .build_load(pointer, head.as_pointer_value(), "gc_register_previous")
            .map_err(builder_error)?
            .into_pointer_value();
        builder
            .build_store(
                gc_node_address(
                    &builder,
                    context,
                    node,
                    next_offset,
                    "gc_register_next_address",
                )?,
                previous,
            )
            .map_err(builder_error)?;
        builder
            .build_store(head.as_pointer_value(), node)
            .map_err(builder_error)?;
        builder
            .build_unconditional_branch(done)
            .map_err(builder_error)?;
        builder.position_at_end(done);
        builder.build_return(None).map_err(builder_error)?;
    }
    let address_type = super::memory::pointer_integer_type(context, target_layout);
    for (symbol, head_name, next_offset) in [
        (GC_UNREGISTER_ENV, GC_ENV_HEAD, ENV_GC_NEXT_OFFSET),
        (GC_UNREGISTER_CELL, GC_CELL_HEAD, CELL_GC_NEXT_OFFSET),
    ] {
        let function = module.add_function(
            symbol,
            context.void_type().fn_type(&[pointer.into()], false),
            Some(Linkage::Internal),
        );
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(function, "gc.unregister.entry"));
        let target = function
            .get_first_param()
            .expect("collector unregister target")
            .into_pointer_value();
        let absent = builder
            .build_is_null(target, "gc_unregister_absent")
            .map_err(builder_error)?;
        let scan_entry = context.append_basic_block(function, "gc.unregister.scan_entry");
        let done = context.append_basic_block(function, "gc.unregister.done");
        builder
            .build_conditional_branch(absent, done, scan_entry)
            .map_err(builder_error)?;
        builder.position_at_end(scan_entry);
        let previous_slot = builder
            .build_alloca(pointer, "gc_unregister_previous")
            .map_err(builder_error)?;
        builder
            .build_store(previous_slot, pointer.const_null())
            .map_err(builder_error)?;
        let head = module
            .get_global(head_name)
            .expect("collector registry head");
        let current_slot = builder
            .build_alloca(pointer, "gc_unregister_current")
            .map_err(builder_error)?;
        let first = builder
            .build_load(pointer, head.as_pointer_value(), "gc_unregister_head")
            .map_err(builder_error)?;
        builder
            .build_store(current_slot, first)
            .map_err(builder_error)?;
        let test = context.append_basic_block(function, "gc.unregister.test");
        let scan = context.append_basic_block(function, "gc.unregister.scan");
        let remove = context.append_basic_block(function, "gc.unregister.remove");
        let head_remove = context.append_basic_block(function, "gc.unregister.head_remove");
        let link_remove = context.append_basic_block(function, "gc.unregister.link_remove");
        let step = context.append_basic_block(function, "gc.unregister.step");
        builder
            .build_unconditional_branch(test)
            .map_err(builder_error)?;
        builder.position_at_end(test);
        let current = builder
            .build_load(pointer, current_slot, "gc_unregister_node")
            .map_err(builder_error)?
            .into_pointer_value();
        let end = builder
            .build_is_null(current, "gc_unregister_end")
            .map_err(builder_error)?;
        builder
            .build_conditional_branch(end, done, scan)
            .map_err(builder_error)?;
        builder.position_at_end(scan);
        let current_id = builder
            .build_ptr_to_int(current, address_type, "gc_unregister_identity")
            .map_err(builder_error)?;
        let target_id = builder
            .build_ptr_to_int(target, address_type, "gc_unregister_wanted")
            .map_err(builder_error)?;
        let found = builder
            .build_int_compare(
                IntPredicate::EQ,
                current_id,
                target_id,
                "gc_unregister_found",
            )
            .map_err(builder_error)?;
        builder
            .build_conditional_branch(found, remove, step)
            .map_err(builder_error)?;
        builder.position_at_end(remove);
        let following = builder
            .build_load(
                pointer,
                gc_node_address(
                    &builder,
                    context,
                    current,
                    next_offset,
                    "gc_unregister_next_address",
                )?,
                "gc_unregister_next",
            )
            .map_err(builder_error)?;
        let previous = builder
            .build_load(pointer, previous_slot, "gc_unregister_previous_node")
            .map_err(builder_error)?
            .into_pointer_value();
        let at_head = builder
            .build_is_null(previous, "gc_unregister_at_head")
            .map_err(builder_error)?;
        builder
            .build_conditional_branch(at_head, head_remove, link_remove)
            .map_err(builder_error)?;
        builder.position_at_end(head_remove);
        builder
            .build_store(head.as_pointer_value(), following)
            .map_err(builder_error)?;
        builder
            .build_unconditional_branch(done)
            .map_err(builder_error)?;
        builder.position_at_end(link_remove);
        builder
            .build_store(
                gc_node_address(
                    &builder,
                    context,
                    previous,
                    next_offset,
                    "gc_unregister_link_address",
                )?,
                following,
            )
            .map_err(builder_error)?;
        builder
            .build_unconditional_branch(done)
            .map_err(builder_error)?;
        builder.position_at_end(step);
        builder
            .build_store(previous_slot, current)
            .map_err(builder_error)?;
        let advance = builder
            .build_load(
                pointer,
                gc_node_address(
                    &builder,
                    context,
                    current,
                    next_offset,
                    "gc_unregister_step_address",
                )?,
                "gc_unregister_following",
            )
            .map_err(builder_error)?;
        builder
            .build_store(current_slot, advance)
            .map_err(builder_error)?;
        builder
            .build_unconditional_branch(test)
            .map_err(builder_error)?;
        builder.position_at_end(done);
        builder.build_return(None).map_err(builder_error)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum GcVisitMode {
    Subtract,
    Mark,
    Detach,
}

fn emit_gc_edge_visitor<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    symbol: &str,
    mark_offset: i64,
    saved_offset: i64,
    count_offset: i64,
    mode: GcVisitMode,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let function = module.add_function(
        symbol,
        context.void_type().fn_type(&[pointer.into()], false),
        Some(Linkage::Internal),
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "gc.visit.entry"));
    let node = function
        .get_first_param()
        .expect("collector visitor node")
        .into_pointer_value();
    let absent = builder
        .build_is_null(node, "gc_visit_absent")
        .map_err(builder_error)?;
    let live = context.append_basic_block(function, "gc.visit.live");
    let done = context.append_basic_block(function, "gc.visit.done");
    builder
        .build_conditional_branch(absent, done, live)
        .map_err(builder_error)?;
    builder.position_at_end(live);
    let mark = builder
        .build_load(
            context.i64_type(),
            gc_node_address(
                &builder,
                context,
                node,
                mark_offset,
                "gc_visit_mark_address",
            )?,
            "gc_visit_mark",
        )
        .map_err(builder_error)?
        .into_int_value();
    mark.as_instruction_value()
        .expect("collector mark load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    match mode {
        GcVisitMode::Subtract => {
            let member = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    mark,
                    context.i64_type().const_int(1, false),
                    "gc_visit_member",
                )
                .map_err(builder_error)?;
            let apply = context.append_basic_block(function, "gc.visit.apply");
            builder
                .build_conditional_branch(member, apply, done)
                .map_err(builder_error)?;
            builder.position_at_end(apply);
            let saved_addr = gc_node_address(
                &builder,
                context,
                node,
                saved_offset,
                "gc_visit_saved_address",
            )?;
            let saved = builder
                .build_load(context.i64_type(), saved_addr, "gc_visit_saved")
                .map_err(builder_error)?
                .into_int_value();
            saved
                .as_instruction_value()
                .expect("collector saved load")
                .set_alignment(8)
                .map_err(|error| error.to_string())?;
            let next = builder
                .build_int_sub(
                    saved,
                    context.i64_type().const_int(1, false),
                    "gc_visit_subtracted",
                )
                .map_err(builder_error)?;
            builder
                .build_store(saved_addr, next)
                .map_err(builder_error)?
                .set_alignment(8)
                .map_err(|error| error.to_string())?;
            builder
                .build_unconditional_branch(done)
                .map_err(builder_error)?;
        }
        GcVisitMode::Mark => {
            let member = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    mark,
                    context.i64_type().const_int(1, false),
                    "gc_visit_member",
                )
                .map_err(builder_error)?;
            let apply = context.append_basic_block(function, "gc.visit.apply");
            builder
                .build_conditional_branch(member, apply, done)
                .map_err(builder_error)?;
            builder.position_at_end(apply);
            builder
                .build_store(
                    gc_node_address(
                        &builder,
                        context,
                        node,
                        mark_offset,
                        "gc_visit_root_address",
                    )?,
                    context.i64_type().const_int(2, false),
                )
                .map_err(builder_error)?
                .set_alignment(8)
                .map_err(|error| error.to_string())?;
            builder
                .build_unconditional_branch(done)
                .map_err(builder_error)?;
        }
        GcVisitMode::Detach => {
            let idle = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    mark,
                    context.i64_type().const_zero(),
                    "gc_visit_idle",
                )
                .map_err(builder_error)?;
            let retiring = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    mark,
                    context.i64_type().const_int(3, false),
                    "gc_visit_retiring",
                )
                .map_err(builder_error)?;
            let disposed = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    mark,
                    context.i64_type().const_int(4, false),
                    "gc_visit_disposed",
                )
                .map_err(builder_error)?;
            let internal = builder
                .build_or(retiring, disposed, "gc_visit_settled")
                .map_err(builder_error)?;
            let skip = builder
                .build_or(idle, internal, "gc_visit_skip")
                .map_err(builder_error)?;
            let attached = context.append_basic_block(function, "gc.visit.attached");
            builder
                .build_conditional_branch(skip, done, attached)
                .map_err(builder_error)?;
            builder.position_at_end(attached);
            let count_addr = gc_node_address(
                &builder,
                context,
                node,
                count_offset,
                "gc_visit_count_address",
            )?;
            let count = builder
                .build_load(context.i64_type(), count_addr, "gc_visit_count")
                .map_err(builder_error)?
                .into_int_value();
            count
                .as_instruction_value()
                .expect("collector count load")
                .set_alignment(8)
                .map_err(|error| error.to_string())?;
            let empty = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    count,
                    context.i64_type().const_zero(),
                    "gc_visit_empty",
                )
                .map_err(builder_error)?;
            let release = context.append_basic_block(function, "gc.visit.release");
            let fail = context.append_basic_block(function, "gc.visit.fail");
            builder
                .build_conditional_branch(empty, fail, release)
                .map_err(builder_error)?;
            builder.position_at_end(fail);
            super::declare_core_runtime(context, module);
            builder
                .build_call(
                    module
                        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
                        .expect("core panic declaration"),
                    &[],
                    "gc_detach_panic",
                )
                .map_err(builder_error)?;
            builder.build_unreachable().map_err(builder_error)?;
            builder.position_at_end(release);
            let next = builder
                .build_int_sub(
                    count,
                    context.i64_type().const_int(1, false),
                    "gc_visit_detached",
                )
                .map_err(builder_error)?;
            builder
                .build_store(count_addr, next)
                .map_err(builder_error)?
                .set_alignment(8)
                .map_err(|error| error.to_string())?;
            builder
                .build_unconditional_branch(done)
                .map_err(builder_error)?;
        }
    }
    builder.position_at_end(done);
    builder.build_return(None).map_err(builder_error)?;
    Ok(())
}

fn emit_gc_release<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_layout: ScalarTargetLayout,
    symbol: &str,
    base_offset: i64,
    owned_offset: Option<i64>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    super::declare_core_runtime(context, module);
    let function = module.add_function(
        symbol,
        context.void_type().fn_type(&[pointer.into()], false),
        Some(Linkage::Internal),
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "gc.release.entry"));
    let node = function
        .get_first_param()
        .expect("collector release node")
        .into_pointer_value();
    let free = module
        .get_function(CORE_FREE_SYMBOL)
        .expect("core free declaration");
    let direct = context.append_basic_block(function, "gc.release.direct");
    if let Some(owned_offset) = owned_offset {
        let owned = builder
            .build_load(
                context.i64_type(),
                gc_node_address(
                    &builder,
                    context,
                    node,
                    owned_offset,
                    "gc_release_owned_address",
                )?,
                "gc_release_owned",
            )
            .map_err(builder_error)?
            .into_int_value();
        let has_backing = builder
            .build_int_compare(
                IntPredicate::NE,
                owned,
                context.i64_type().const_zero(),
                "gc_release_has_backing",
            )
            .map_err(builder_error)?;
        let backing = context.append_basic_block(function, "gc.release.backing");
        builder
            .build_conditional_branch(has_backing, backing, direct)
            .map_err(builder_error)?;
        builder.position_at_end(backing);
        let data = builder
            .build_load(pointer, node, "gc_release_data")
            .map_err(builder_error)?
            .into_pointer_value();
        let header = unsafe {
            builder.build_in_bounds_gep(
                context.i8_type(),
                data,
                &[context
                    .i64_type()
                    .const_int((target_layout.pointer_size * 2).wrapping_neg(), false)],
                "gc_release_header",
            )
        }
        .map_err(builder_error)?;
        let base = builder
            .build_load(pointer, header, "gc_release_backing_base")
            .map_err(builder_error)?
            .into_pointer_value();
        builder
            .build_call(free, &[base.into()], "gc_release_backing_free")
            .map_err(builder_error)?;
        builder
            .build_unconditional_branch(direct)
            .map_err(builder_error)?;
    } else {
        builder
            .build_unconditional_branch(direct)
            .map_err(builder_error)?;
    }
    builder.position_at_end(direct);
    let allocation = builder
        .build_load(
            pointer,
            gc_node_address(
                &builder,
                context,
                node,
                base_offset,
                "gc_release_base_address",
            )?,
            "gc_release_base",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    builder
        .build_call(free, &[allocation.into()], "gc_release_free")
        .map_err(builder_error)?;
    builder.build_return(None).map_err(builder_error)?;
    Ok(())
}

struct GcWalk<'ctx> {
    current: PointerValue<'ctx>,
    slot: PointerValue<'ctx>,
    next_offset: i64,
    test: BasicBlock<'ctx>,
    step: BasicBlock<'ctx>,
    done: BasicBlock<'ctx>,
}

fn gc_walk_begin<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    function: FunctionValue<'ctx>,
    head: GlobalValue<'ctx>,
    next_offset: i64,
    prefix: &str,
) -> Result<GcWalk<'ctx>, String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let entry = function
        .get_first_basic_block()
        .expect("collector walk entry");
    let anchor = builder.get_insert_block().expect("collector walk anchor");
    if let Some(first) = entry.get_first_instruction() {
        builder.position_before(&first);
    } else {
        builder.position_at_end(entry);
    }
    let slot = builder
        .build_alloca(pointer, "gc_walk_slot")
        .map_err(builder_error)?;
    builder.position_at_end(anchor);
    let test = context.append_basic_block(function, &format!("{prefix}.test"));
    let body = context.append_basic_block(function, &format!("{prefix}.body"));
    let step = context.append_basic_block(function, &format!("{prefix}.step"));
    let done = context.append_basic_block(function, &format!("{prefix}.done"));
    let initial = builder
        .build_load(pointer, head.as_pointer_value(), "gc_walk_head")
        .map_err(builder_error)?;
    builder.build_store(slot, initial).map_err(builder_error)?;
    builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    builder.position_at_end(test);
    let node = builder
        .build_load(pointer, slot, "gc_walk_node")
        .map_err(builder_error)?
        .into_pointer_value();
    let end = builder
        .build_is_null(node, "gc_walk_end")
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(end, done, body)
        .map_err(builder_error)?;
    builder.position_at_end(body);
    let current = builder
        .build_load(pointer, slot, "gc_walk_current")
        .map_err(builder_error)?
        .into_pointer_value();
    Ok(GcWalk {
        current,
        slot,
        next_offset,
        test,
        step,
        done,
    })
}

fn gc_walk_end<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    walk: &GcWalk<'ctx>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    builder
        .build_unconditional_branch(walk.step)
        .map_err(builder_error)?;
    builder.position_at_end(walk.step);
    let node = builder
        .build_load(pointer, walk.slot, "gc_walk_advance_node")
        .map_err(builder_error)?
        .into_pointer_value();
    let next = builder
        .build_load(
            pointer,
            gc_node_address(
                builder,
                context,
                node,
                walk.next_offset,
                "gc_walk_next_address",
            )?,
            "gc_walk_next",
        )
        .map_err(builder_error)?;
    builder
        .build_store(walk.slot, next)
        .map_err(builder_error)?;
    builder
        .build_unconditional_branch(walk.test)
        .map_err(builder_error)?;
    builder.position_at_end(walk.done);
    Ok(())
}

fn gc_call_trace<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    node: PointerValue<'ctx>,
    descriptor_offset: i64,
    visitor: FunctionValue<'ctx>,
    name: &str,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let descriptor = builder
        .build_load(
            pointer,
            gc_node_address(
                builder,
                context,
                node,
                descriptor_offset,
                "gc_trace_descriptor_address",
            )?,
            "gc_trace_descriptor",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    let trace = builder
        .build_load(
            pointer,
            unsafe {
                builder.build_in_bounds_gep(
                    pointer,
                    descriptor,
                    &[context.i32_type().const_zero()],
                    "gc_trace_slot",
                )
            }
            .map_err(builder_error)?,
            "gc_trace",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    builder
        .build_indirect_call(
            context
                .void_type()
                .fn_type(&[pointer.into(), pointer.into()], false),
            trace,
            &[
                node.into(),
                visitor.as_global_value().as_pointer_value().into(),
            ],
            name,
        )
        .map_err(builder_error)?;
    Ok(())
}

fn gc_guarded_cell_trace<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    function: FunctionValue<'ctx>,
    cell: PointerValue<'ctx>,
    visitor: FunctionValue<'ctx>,
    prefix: &str,
    join: BasicBlock<'ctx>,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let descriptor = builder
        .build_load(
            pointer,
            gc_node_address(
                builder,
                context,
                cell,
                CELL_DESCRIPTOR_OFFSET,
                "gc_cell_trace_descriptor_address",
            )?,
            "gc_cell_trace_descriptor",
        )
        .map_err(builder_error)?
        .into_pointer_value();
    let missing = builder
        .build_is_null(descriptor, "gc_cell_trace_missing")
        .map_err(builder_error)?;
    let visit = context.append_basic_block(function, &format!("{prefix}.visit"));
    builder
        .build_conditional_branch(missing, join, visit)
        .map_err(builder_error)?;
    builder.position_at_end(visit);
    gc_call_trace(
        context,
        builder,
        cell,
        CELL_DESCRIPTOR_OFFSET,
        visitor,
        "gc_cell_traced",
    )?;
    builder
        .build_unconditional_branch(join)
        .map_err(builder_error)?;
    builder.position_at_end(join);
    Ok(())
}

fn gc_mark_equals<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    node: PointerValue<'ctx>,
    offset: i64,
    value: u64,
    name: &str,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let mark = builder
        .build_load(
            context.i64_type(),
            gc_node_address(builder, context, node, offset, "gc_mark_address")?,
            "gc_mark",
        )
        .map_err(builder_error)?
        .into_int_value();
    mark.as_instruction_value()
        .expect("collector mark load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    builder
        .build_int_compare(
            IntPredicate::EQ,
            mark,
            context.i64_type().const_int(value, false),
            name,
        )
        .map_err(builder_error)
}

fn gc_store_mark<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    node: PointerValue<'ctx>,
    offset: i64,
    value: u64,
) -> Result<(), String> {
    builder
        .build_store(
            gc_node_address(builder, context, node, offset, "gc_mark_store_address")?,
            context.i64_type().const_int(value, false),
        )
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn gc_snapshot_node<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    node: PointerValue<'ctx>,
    count_offset: i64,
    saved_offset: i64,
    mark_offset: i64,
    live_slot: PointerValue<'ctx>,
) -> Result<(), String> {
    let count = builder
        .build_load(
            context.i64_type(),
            gc_node_address(
                builder,
                context,
                node,
                count_offset,
                "gc_snapshot_count_address",
            )?,
            "gc_snapshot_count",
        )
        .map_err(builder_error)?
        .into_int_value();
    count
        .as_instruction_value()
        .expect("collector snapshot load")
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    let saved_addr = gc_node_address(
        builder,
        context,
        node,
        saved_offset,
        "gc_snapshot_saved_address",
    )?;
    builder
        .build_store(saved_addr, count)
        .map_err(builder_error)?
        .set_alignment(8)
        .map_err(|error| error.to_string())?;
    gc_store_mark(context, builder, node, mark_offset, 1)?;
    let live = builder
        .build_load(context.i64_type(), live_slot, "gc_snapshot_live")
        .map_err(builder_error)?
        .into_int_value();
    let next = builder
        .build_int_add(
            live,
            context.i64_type().const_int(1, false),
            "gc_snapshot_grown",
        )
        .map_err(builder_error)?;
    builder
        .build_store(live_slot, next)
        .map_err(builder_error)?;
    Ok(())
}

fn gc_seed_node<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    function: FunctionValue<'ctx>,
    node: PointerValue<'ctx>,
    saved_offset: i64,
    mark_offset: i64,
    prefix: &str,
    join: BasicBlock<'ctx>,
) -> Result<(), String> {
    let saved = builder
        .build_load(
            context.i64_type(),
            gc_node_address(
                builder,
                context,
                node,
                saved_offset,
                "gc_seed_saved_address",
            )?,
            "gc_seed_saved",
        )
        .map_err(builder_error)?
        .into_int_value();
    let rooted = builder
        .build_int_compare(
            IntPredicate::SGT,
            saved,
            context.i64_type().const_zero(),
            "gc_seed_rooted",
        )
        .map_err(builder_error)?;
    let seed = context.append_basic_block(function, &format!("{prefix}.seed"));
    builder
        .build_conditional_branch(rooted, seed, join)
        .map_err(builder_error)?;
    builder.position_at_end(seed);
    gc_store_mark(context, builder, node, mark_offset, 2)?;
    builder
        .build_unconditional_branch(join)
        .map_err(builder_error)?;
    builder.position_at_end(join);
    Ok(())
}

pub(super) fn emit_collector_runtime<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    emit_gc_edge_visitor(
        context,
        module,
        GC_ENV_SUBTRACT,
        ENV_GC_MARK_OFFSET,
        ENV_GC_SAVED_OFFSET,
        ENV_COUNT_OFFSET,
        GcVisitMode::Subtract,
    )?;
    emit_gc_edge_visitor(
        context,
        module,
        GC_CELL_SUBTRACT,
        CELL_GC_MARK_OFFSET,
        CELL_GC_SAVED_OFFSET,
        CELL_COUNT_OFFSET,
        GcVisitMode::Subtract,
    )?;
    emit_gc_edge_visitor(
        context,
        module,
        GC_ENV_MARK,
        ENV_GC_MARK_OFFSET,
        ENV_GC_SAVED_OFFSET,
        ENV_COUNT_OFFSET,
        GcVisitMode::Mark,
    )?;
    emit_gc_edge_visitor(
        context,
        module,
        GC_CELL_MARK,
        CELL_GC_MARK_OFFSET,
        CELL_GC_SAVED_OFFSET,
        CELL_COUNT_OFFSET,
        GcVisitMode::Mark,
    )?;
    emit_gc_edge_visitor(
        context,
        module,
        GC_ENV_DETACH,
        ENV_GC_MARK_OFFSET,
        ENV_GC_SAVED_OFFSET,
        ENV_COUNT_OFFSET,
        GcVisitMode::Detach,
    )?;
    emit_gc_edge_visitor(
        context,
        module,
        GC_CELL_DETACH,
        CELL_GC_MARK_OFFSET,
        CELL_GC_SAVED_OFFSET,
        CELL_COUNT_OFFSET,
        GcVisitMode::Detach,
    )?;
    emit_gc_release(
        context,
        module,
        target_layout,
        GC_ENV_RELEASE,
        ENV_BASE_OFFSET,
        None,
    )?;
    emit_gc_release(
        context,
        module,
        target_layout,
        GC_CELL_RELEASE,
        CELL_BASE_OFFSET,
        Some(CELL_OWNED_OFFSET),
    )?;
    let env_head = module.get_global(GC_ENV_HEAD).expect("collector env head");
    let cell_head = module
        .get_global(GC_CELL_HEAD)
        .expect("collector cell head");
    let visitor = |symbol: &str| module.get_function(symbol).expect("collector visitor");
    let release = |symbol: &str| module.get_function(symbol).expect("collector release");
    let trial = module.add_function(
        GC_TRIAL_SYMBOL,
        context.void_type().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(trial, "gc.trial.entry"));
    let live_slot = builder
        .build_alloca(context.i64_type(), "gc_trial_live")
        .map_err(builder_error)?;
    builder
        .build_store(live_slot, context.i64_type().const_zero())
        .map_err(builder_error)?;
    let round_slot = builder
        .build_alloca(context.i64_type(), "gc_trial_round")
        .map_err(builder_error)?;
    let snapshot = gc_walk_begin(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        "gc.snapshot.env",
    )?;
    gc_snapshot_node(
        context,
        &builder,
        snapshot.current,
        ENV_COUNT_OFFSET,
        ENV_GC_SAVED_OFFSET,
        ENV_GC_MARK_OFFSET,
        live_slot,
    )?;
    gc_walk_end(context, &builder, &snapshot)?;
    let snapshot_cells = gc_walk_begin(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        "gc.snapshot.cell",
    )?;
    gc_snapshot_node(
        context,
        &builder,
        snapshot_cells.current,
        CELL_COUNT_OFFSET,
        CELL_GC_SAVED_OFFSET,
        CELL_GC_MARK_OFFSET,
        live_slot,
    )?;
    gc_walk_end(context, &builder, &snapshot_cells)?;
    let subtract = gc_walk_begin(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        "gc.subtract.env",
    )?;
    gc_call_trace(
        context,
        &builder,
        subtract.current,
        ENV_DESCRIPTOR_OFFSET,
        visitor(GC_CELL_SUBTRACT),
        "gc_env_subtracted",
    )?;
    gc_walk_end(context, &builder, &subtract)?;
    let subtract_cells = gc_walk_begin(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        "gc.subtract.cell",
    )?;
    let subtract_join = context.append_basic_block(trial, "gc.subtract.cell.join");
    gc_guarded_cell_trace(
        context,
        &builder,
        trial,
        subtract_cells.current,
        visitor(GC_ENV_SUBTRACT),
        "gc.subtract.cell",
        subtract_join,
    )?;
    gc_walk_end(context, &builder, &subtract_cells)?;
    let seed = gc_walk_begin(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        "gc.seed.env",
    )?;
    let seed_join = context.append_basic_block(trial, "gc.seed.env.join");
    gc_seed_node(
        context,
        &builder,
        trial,
        seed.current,
        ENV_GC_SAVED_OFFSET,
        ENV_GC_MARK_OFFSET,
        "gc.seed.env",
        seed_join,
    )?;
    gc_walk_end(context, &builder, &seed)?;
    let seed_cells = gc_walk_begin(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        "gc.seed.cell",
    )?;
    let seed_cell_join = context.append_basic_block(trial, "gc.seed.cell.join");
    gc_seed_node(
        context,
        &builder,
        trial,
        seed_cells.current,
        CELL_GC_SAVED_OFFSET,
        CELL_GC_MARK_OFFSET,
        "gc.seed.cell",
        seed_cell_join,
    )?;
    gc_walk_end(context, &builder, &seed_cells)?;
    let rounds = builder
        .build_load(context.i64_type(), live_slot, "gc_trial_rounds")
        .map_err(builder_error)?
        .into_int_value();
    builder
        .build_store(round_slot, context.i64_type().const_zero())
        .map_err(builder_error)?;
    let propagate_test = context.append_basic_block(trial, "gc.propagate.test");
    let propagate_body = context.append_basic_block(trial, "gc.propagate.body");
    let propagate_done = context.append_basic_block(trial, "gc.propagate.done");
    builder
        .build_unconditional_branch(propagate_test)
        .map_err(builder_error)?;
    builder.position_at_end(propagate_test);
    let round = builder
        .build_load(context.i64_type(), round_slot, "gc_propagate_round")
        .map_err(builder_error)?
        .into_int_value();
    let advance = builder
        .build_int_compare(IntPredicate::ULT, round, rounds, "gc_propagate_advance")
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(advance, propagate_body, propagate_done)
        .map_err(builder_error)?;
    builder.position_at_end(propagate_body);
    let propagate_env = gc_walk_begin(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        "gc.propagate.env",
    )?;
    let propagate_env_join = context.append_basic_block(trial, "gc.propagate.env.join");
    let rooted = gc_mark_equals(
        context,
        &builder,
        propagate_env.current,
        ENV_GC_MARK_OFFSET,
        2,
        "gc_propagate_rooted",
    )?;
    let propagate_env_visit = context.append_basic_block(trial, "gc.propagate.env.visit");
    builder
        .build_conditional_branch(rooted, propagate_env_visit, propagate_env_join)
        .map_err(builder_error)?;
    builder.position_at_end(propagate_env_visit);
    gc_call_trace(
        context,
        &builder,
        propagate_env.current,
        ENV_DESCRIPTOR_OFFSET,
        visitor(GC_CELL_MARK),
        "gc_env_propagated",
    )?;
    builder
        .build_unconditional_branch(propagate_env_join)
        .map_err(builder_error)?;
    builder.position_at_end(propagate_env_join);
    gc_walk_end(context, &builder, &propagate_env)?;
    let propagate_cell = gc_walk_begin(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        "gc.propagate.cell",
    )?;
    let propagate_cell_join = context.append_basic_block(trial, "gc.propagate.cell.join");
    let cell_rooted = gc_mark_equals(
        context,
        &builder,
        propagate_cell.current,
        CELL_GC_MARK_OFFSET,
        2,
        "gc_propagate_cell_rooted",
    )?;
    let propagate_cell_check = context.append_basic_block(trial, "gc.propagate.cell.check");
    builder
        .build_conditional_branch(cell_rooted, propagate_cell_check, propagate_cell_join)
        .map_err(builder_error)?;
    builder.position_at_end(propagate_cell_check);
    gc_guarded_cell_trace(
        context,
        &builder,
        trial,
        propagate_cell.current,
        visitor(GC_ENV_MARK),
        "gc.propagate.cell",
        propagate_cell_join,
    )?;
    gc_walk_end(context, &builder, &propagate_cell)?;
    let next_round = builder
        .build_int_add(
            round,
            context.i64_type().const_int(1, false),
            "gc_propagate_next",
        )
        .map_err(builder_error)?;
    builder
        .build_store(round_slot, next_round)
        .map_err(builder_error)?;
    builder
        .build_unconditional_branch(propagate_test)
        .map_err(builder_error)?;
    builder.position_at_end(propagate_done);
    let retire = gc_walk_begin(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        "gc.retire.env",
    )?;
    let retire_join = context.append_basic_block(trial, "gc.retire.env.join");
    let unreachable_env = gc_mark_equals(
        context,
        &builder,
        retire.current,
        ENV_GC_MARK_OFFSET,
        1,
        "gc_retire_unreachable",
    )?;
    let retire_env_apply = context.append_basic_block(trial, "gc.retire.env.apply");
    builder
        .build_conditional_branch(unreachable_env, retire_env_apply, retire_join)
        .map_err(builder_error)?;
    builder.position_at_end(retire_env_apply);
    gc_store_mark(context, &builder, retire.current, ENV_GC_MARK_OFFSET, 3)?;
    builder
        .build_unconditional_branch(retire_join)
        .map_err(builder_error)?;
    builder.position_at_end(retire_join);
    gc_walk_end(context, &builder, &retire)?;
    let retire_cells = gc_walk_begin(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        "gc.retire.cell",
    )?;
    let retire_cell_join = context.append_basic_block(trial, "gc.retire.cell.join");
    let unreachable_cell = gc_mark_equals(
        context,
        &builder,
        retire_cells.current,
        CELL_GC_MARK_OFFSET,
        1,
        "gc_retire_cell_unreachable",
    )?;
    let retire_cell_apply = context.append_basic_block(trial, "gc.retire.cell.apply");
    builder
        .build_conditional_branch(unreachable_cell, retire_cell_apply, retire_cell_join)
        .map_err(builder_error)?;
    builder.position_at_end(retire_cell_apply);
    gc_store_mark(
        context,
        &builder,
        retire_cells.current,
        CELL_GC_MARK_OFFSET,
        3,
    )?;
    builder
        .build_unconditional_branch(retire_cell_join)
        .map_err(builder_error)?;
    builder.position_at_end(retire_cell_join);
    gc_walk_end(context, &builder, &retire_cells)?;
    let detach = gc_walk_begin(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        "gc.detach.env",
    )?;
    let detach_join = context.append_basic_block(trial, "gc.detach.env.join");
    let retiring = gc_mark_equals(
        context,
        &builder,
        detach.current,
        ENV_GC_MARK_OFFSET,
        3,
        "gc_detach_retiring",
    )?;
    let detach_visit = context.append_basic_block(trial, "gc.detach.env.visit");
    builder
        .build_conditional_branch(retiring, detach_visit, detach_join)
        .map_err(builder_error)?;
    builder.position_at_end(detach_visit);
    gc_call_trace(
        context,
        &builder,
        detach.current,
        ENV_DESCRIPTOR_OFFSET,
        visitor(GC_CELL_DETACH),
        "gc_env_detached",
    )?;
    builder
        .build_unconditional_branch(detach_join)
        .map_err(builder_error)?;
    builder.position_at_end(detach_join);
    gc_walk_end(context, &builder, &detach)?;
    let detach_cells = gc_walk_begin(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        "gc.detach.cell",
    )?;
    let detach_cell_join = context.append_basic_block(trial, "gc.detach.cell.join");
    let cell_retiring = gc_mark_equals(
        context,
        &builder,
        detach_cells.current,
        CELL_GC_MARK_OFFSET,
        3,
        "gc_detach_cell_retiring",
    )?;
    let detach_cell_check = context.append_basic_block(trial, "gc.detach.cell.check");
    builder
        .build_conditional_branch(cell_retiring, detach_cell_check, detach_cell_join)
        .map_err(builder_error)?;
    builder.position_at_end(detach_cell_check);
    gc_guarded_cell_trace(
        context,
        &builder,
        trial,
        detach_cells.current,
        visitor(GC_ENV_DETACH),
        "gc.detach.cell",
        detach_cell_join,
    )?;
    gc_walk_end(context, &builder, &detach_cells)?;
    gc_release_walk(
        context,
        &builder,
        trial,
        env_head,
        ENV_GC_NEXT_OFFSET,
        ENV_GC_MARK_OFFSET,
        release(GC_ENV_RELEASE),
        "gc.release.env",
    )?;
    gc_release_walk(
        context,
        &builder,
        trial,
        cell_head,
        CELL_GC_NEXT_OFFSET,
        CELL_GC_MARK_OFFSET,
        release(GC_CELL_RELEASE),
        "gc.release.cell",
    )?;
    builder.build_return(None).map_err(builder_error)?;
    Ok(())
}

fn gc_release_walk<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    function: FunctionValue<'ctx>,
    head: GlobalValue<'ctx>,
    next_offset: i64,
    mark_offset: i64,
    release: FunctionValue<'ctx>,
    prefix: &str,
) -> Result<(), String> {
    let pointer = context.ptr_type(AddressSpace::default());
    let entry = function
        .get_first_basic_block()
        .expect("collector release entry");
    let anchor = builder
        .get_insert_block()
        .expect("collector release anchor");
    if let Some(first) = entry.get_first_instruction() {
        builder.position_before(&first);
    } else {
        builder.position_at_end(entry);
    }
    let previous_slot = builder
        .build_alloca(pointer, "gc_release_previous")
        .map_err(builder_error)?;
    let current_slot = builder
        .build_alloca(pointer, "gc_release_current")
        .map_err(builder_error)?;
    builder.position_at_end(anchor);
    builder
        .build_store(previous_slot, pointer.const_null())
        .map_err(builder_error)?;
    let initial = builder
        .build_load(pointer, head.as_pointer_value(), "gc_release_head")
        .map_err(builder_error)?;
    builder
        .build_store(current_slot, initial)
        .map_err(builder_error)?;
    let test = context.append_basic_block(function, &format!("{prefix}.test"));
    let scan = context.append_basic_block(function, &format!("{prefix}.scan"));
    let retire = context.append_basic_block(function, &format!("{prefix}.retire"));
    let keep = context.append_basic_block(function, &format!("{prefix}.keep"));
    let done = context.append_basic_block(function, &format!("{prefix}.done"));
    builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    builder.position_at_end(test);
    let current = builder
        .build_load(pointer, current_slot, "gc_release_node")
        .map_err(builder_error)?
        .into_pointer_value();
    let end = builder
        .build_is_null(current, "gc_release_end")
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(end, done, scan)
        .map_err(builder_error)?;
    builder.position_at_end(scan);
    let retiring = gc_mark_equals(
        context,
        builder,
        current,
        mark_offset,
        3,
        "gc_release_retiring",
    )?;
    builder
        .build_conditional_branch(retiring, retire, keep)
        .map_err(builder_error)?;
    builder.position_at_end(retire);
    let next = builder
        .build_load(
            pointer,
            gc_node_address(
                builder,
                context,
                current,
                next_offset,
                "gc_release_next_address",
            )?,
            "gc_release_next",
        )
        .map_err(builder_error)?;
    let previous = builder
        .build_load(pointer, previous_slot, "gc_release_previous_node")
        .map_err(builder_error)?
        .into_pointer_value();
    let at_head = builder
        .build_is_null(previous, "gc_release_at_head")
        .map_err(builder_error)?;
    let head_remove = context.append_basic_block(function, &format!("{prefix}.head_remove"));
    let link_remove = context.append_basic_block(function, &format!("{prefix}.link_remove"));
    let disposed = context.append_basic_block(function, &format!("{prefix}.disposed"));
    builder
        .build_conditional_branch(at_head, head_remove, link_remove)
        .map_err(builder_error)?;
    builder.position_at_end(head_remove);
    builder
        .build_store(head.as_pointer_value(), next)
        .map_err(builder_error)?;
    builder
        .build_unconditional_branch(disposed)
        .map_err(builder_error)?;
    builder.position_at_end(link_remove);
    builder
        .build_store(
            gc_node_address(
                builder,
                context,
                previous,
                next_offset,
                "gc_release_link_address",
            )?,
            next,
        )
        .map_err(builder_error)?;
    builder
        .build_unconditional_branch(disposed)
        .map_err(builder_error)?;
    builder.position_at_end(disposed);
    builder
        .build_call(release, &[current.into()], "gc_released")
        .map_err(builder_error)?;
    builder
        .build_store(current_slot, next)
        .map_err(builder_error)?;
    builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    builder.position_at_end(keep);
    builder
        .build_store(previous_slot, current)
        .map_err(builder_error)?;
    let following = builder
        .build_load(
            pointer,
            gc_node_address(
                builder,
                context,
                current,
                next_offset,
                "gc_release_keep_address",
            )?,
            "gc_release_following",
        )
        .map_err(builder_error)?;
    builder
        .build_store(current_slot, following)
        .map_err(builder_error)?;
    builder
        .build_unconditional_branch(test)
        .map_err(builder_error)?;
    builder.position_at_end(done);
    Ok(())
}
