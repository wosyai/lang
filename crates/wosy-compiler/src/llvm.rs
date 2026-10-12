use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;

use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, GlobalValue, PointerValue, StructValue,
    ValueKind,
};
use inkwell::AddressSpace;
use inkwell::{FloatPredicate, IntPredicate};
use serde::{Deserialize, Serialize};

use crate::scalar::{
    CfgPointKind, CoreOperationId, InvalidationCandidate, InvalidationInputs, ReferenceBindingId,
    ReferenceBindingKind, ReferenceOrigin, ReleaseTarget, ResolvedCallTarget, RuntimeBackingKind,
    RuntimeBackingTransfer, RuntimeValueId, RuntimeValuePosition, ScalarAllocationIdentity,
    ScalarAutomaticReturnResult, ScalarAutomaticReturnResultState, ScalarBindingOutputOrigin,
    ScalarFieldReference, ScalarOutputValue, ScalarOverloadSelection,
};
use crate::{
    BinaryOperator, ScalarAssignment, ScalarBlock, ScalarBlockItem, ScalarExpression,
    ScalarFunction, ScalarItem, ScalarModule, ScalarProjectValidation, ScalarStruct,
    ScalarTargetLayout, ScalarType, ScalarValidation, ScalarWhile,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LlvmValueType {
    Void,
    Aggregate(Vec<LlvmValueType>),
    I1,
    I16,
    I32,
    I8,
    I64,
    I128,
    F32,
    F64,
    Char,
    ArtifactId,
    Pointer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmPartition {
    pub module_name: String,
    pub declarations: Vec<LlvmFunction>,
    pub functions: Vec<LlvmFunction>,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmFunction {
    pub name: String,
    pub result: LlvmValueType,
    pub parameters: Vec<(String, LlvmValueType)>,
    pub attributes: Vec<LlvmFunctionAttributes>,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmFunctionAttributes {
    pub group: u32,
    pub wasm_import_module: String,
    pub wasm_import_name: String,
}

#[derive(Clone)]
enum EmitValue<'ctx> {
    Unit,
    Basic(BasicValueEnum<'ctx>),
    Aggregate {
        value: StructValue<'ctx>,
        outputs: Vec<ScalarType>,
    },
}

struct EmitState<'ctx, 'module> {
    builder: &'ctx Builder<'ctx>,
    module: &'module Module<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    literal_signatures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, ScalarType>,
    literal_captures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
    literal_reference_cfgs:
        &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, crate::scalar::ScalarReferenceCfg>,
    covered_literals: std::collections::HashSet<wosy_syntax::SourceSpan>,
    cyclic_literals: Vec<wosy_syntax::SourceSpan>,
    counted_fixed_cells: std::collections::BTreeSet<crate::scalar::ReferenceBindingId>,
    top_level_remainder_cells: Vec<(crate::scalar::ReferenceBindingId, ScalarType)>,
    counted_cell_backings: Vec<wosy_syntax::SourceSpan>,
    resolved_calls: BTreeMap<(u32, u32), ResolvedCallTarget>,
    source: wosy_syntax::SourceIdentity,
    capture_cells: BTreeMap<crate::scalar::ReferenceBindingId, PointerValue<'ctx>>,
    backing: ownership::ValidatedBackingMaps,
    holder_events: holder_events::HolderEvents<'ctx>,
    escaping: BTreeMap<String, Vec<ownership::ValidatedEscape>>,
    backing_values: BTreeMap<String, Vec<RuntimeValueId>>,
    backing_slots: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    imported_cell_slots: BTreeMap<String, PointerValue<'ctx>>,
    backing_captured: BTreeSet<String>,
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    fresh_return_outputs: BTreeSet<usize>,
    branch_return_outputs:
        BTreeMap<(u32, u32), (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>)>,
    joined_return_bindings: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    joined_assignment_owners: BTreeMap<(u32, u32), CheckedAssignmentOwner>,
    return_output_types: Vec<ScalarType>,
    return_body_span: Option<wosy_syntax::ByteSpan>,
    checked_call_owners: BTreeMap<(u32, u32, usize), CheckedCallOwner>,
    checked_owner_flags: BTreeMap<String, inkwell::values::IntValue<'ctx>>,
    call_owner_flags: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    return_owner_flags: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    return_owner_slots: BTreeMap<usize, PointerValue<'ctx>>,
    return_length_slots: BTreeMap<usize, PointerValue<'ctx>>,
    call_length_slots: BTreeMap<usize, PointerValue<'ctx>>,
    runtime_array_owners: BTreeMap<String, bool>,
    runtime_array_allocations: BTreeMap<String, bool>,
    runtime_array_lengths: BTreeMap<String, PointerValue<'ctx>>,
    runtime_array_live: BTreeMap<String, PointerValue<'ctx>>,
    mutable_runtime_allocations: BTreeSet<(u32, u32)>,
    invalidations: BTreeMap<
        (u32, u32),
        (
            CoreOperationId,
            Vec<InvalidationCandidate>,
            InvalidationInputs,
        ),
    >,
    automatic_return_result_owners: BTreeMap<String, ScalarAutomaticReturnResult>,
    globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    all_globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    structs: &'module [ScalarStruct],
    target_layout: ScalarTargetLayout,
    next_literal: usize,
    next_block: usize,
}

#[derive(Clone, Copy)]
enum CheckedCallOwner {
    Fresh,
    Borrowed,
    Mixed,
}

#[derive(Clone, Copy)]
enum CheckedAssignmentOwner {
    Local,
    External,
    Borrowed,
}

fn mutable_runtime_allocations(function: &ScalarFunction) -> BTreeSet<(u32, u32)> {
    let mut result = BTreeSet::new();
    for point in &function.reference_cfg.points {
        match &point.kind {
            CfgPointKind::Assign { target, .. } if target.projections.is_empty() => {
                result.insert((target.declaration_span.start, target.declaration_span.end));
            }
            CfgPointKind::Release {
                target: ReleaseTarget::Checked { candidates, .. },
            }
            | CfgPointKind::Invalidate { candidates, .. } => {
                for candidate in candidates {
                    result.insert((
                        candidate.place.declaration_span.start,
                        candidate.place.declaration_span.end,
                    ));
                }
            }
            _ => {}
        }
    }
    result
}

pub(super) struct CaptureProducer {
    event: CfgPointKind,
    parent: crate::scalar::ScalarReferenceCfg,
    point: crate::scalar::CfgPointId,
    descriptor: std::cell::OnceCell<holder_events::CaptureDescriptor>,
    pub(super) counted_cells: std::cell::RefCell<BTreeSet<crate::scalar::ReferenceBindingId>>,
}

pub(super) fn cyclic_callable_backedge(
    producer: &CaptureProducer,
    binding: &crate::scalar::ReferenceBindingId,
) -> bool {
    let CfgPointKind::Capture { environment, .. } = &producer.event else {
        return false;
    };
    producer.parent.holder_edges.iter().any(|edge| {
        edge.actions.iter().any(|action| {
            matches!(action,
                crate::scalar::CallableHolderAction::Move { environment: incoming, to: crate::scalar::CallableHolderPlace::Binding(target), .. }
                | crate::scalar::CallableHolderAction::Retain { environment: incoming, to: crate::scalar::CallableHolderPlace::Binding(target), .. }
                if incoming == environment && target.source == binding.source
                    && target.function_span == binding.function_span
                    && target.declaration_span == binding.declaration_span)
        })
    })
}

fn capture_cycle_candidate_present(
    captures: &std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
) -> bool {
    captures.iter().any(|(_, producer)| {
        let CfgPointKind::Capture { captures, .. } = &producer.event else {
            return false;
        };
        captures
            .iter()
            .any(|capture| cyclic_callable_backedge(producer, &capture.binding))
            || producer_has_selected_array_install(producer)
    })
}

fn cycle_counted_literals(
    captures: &std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
) -> Vec<wosy_syntax::SourceSpan> {
    captures
        .iter()
        .filter(|(_, producer)| {
            let CfgPointKind::Capture { captures, .. } = &producer.event else {
                return false;
            };
            captures
                .iter()
                .any(|capture| cyclic_callable_backedge(producer, &capture.binding))
                || producer_has_selected_array_install(producer)
        })
        .map(|(literal, _)| literal.clone())
        .collect()
}

pub(super) fn fixed_cycle_cell_covered(
    captures: &std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
    literal: &wosy_syntax::SourceSpan,
    binding: &crate::scalar::ReferenceBindingId,
) -> bool {
    captures.get(literal).is_some_and(|producer| {
        producer_has_selected_array_install(producer)
            && match &producer.event {
                CfgPointKind::Capture { captures, .. } => captures.iter().any(|input| {
                    input.binding.source == binding.source
                        && input.binding.function_span == binding.function_span
                        && input.binding.declaration_span == binding.declaration_span
                }),
                _ => false,
            }
    })
}

fn counted_cell_backing_origins(
    captures: &std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
) -> Vec<wosy_syntax::SourceSpan> {
    let mut origins = Vec::new();
    for producer in captures.values() {
        if !producer_has_selected_array_install(producer) {
            continue;
        }
        let CfgPointKind::Capture { literal, .. } = &producer.event else {
            continue;
        };
        let maps = ownership::validated_backing_maps(&producer.parent);
        let Some(inputs) = maps.captures.get(&(literal.start, literal.end)) else {
            continue;
        };
        for input in inputs {
            let installed = input
                .backings
                .iter()
                .flat_map(|fact| &fact.callable_family)
                .flat_map(|family| &family.elements)
                .count()
                + input
                    .projected
                    .iter()
                    .flat_map(|projected| &projected.backing.callable_family)
                    .flat_map(|family| &family.elements)
                    .count();
            if installed > 0 {
                continue;
            }
            for fact in &input.backings {
                if !matches!(
                    fact.kind,
                    crate::scalar::RuntimeBackingKind::OwnedAllocation
                ) {
                    continue;
                }
                if !fact.callable_family.iter().any(|family| {
                    matches!(family.element, crate::scalar::ScalarType::Callable { .. })
                }) {
                    continue;
                }
                if !origins.contains(&fact.origin) {
                    origins.push(fact.origin.clone());
                }
            }
        }
    }
    origins
}

pub(super) fn producer_has_selected_array_install(producer: &CaptureProducer) -> bool {
    let CfgPointKind::Capture {
        environment,
        captures,
        ..
    } = &producer.event
    else {
        return false;
    };
    let mut actions = producer
        .parent
        .holder_edges
        .iter()
        .flat_map(|edge| &edge.actions)
        .chain(
            producer
                .parent
                .holder_exits
                .iter()
                .flat_map(|exit| &exit.actions),
        );
    actions.any(|action| {
        matches!(action,
            crate::scalar::CallableHolderAction::RetainSelected { environment: installed, to: holder, .. }
            | crate::scalar::CallableHolderAction::MoveSelected { environment: installed, to: holder, .. }
            if installed.literal == environment.literal
                && captures.iter().any(|input| input.binding.source == holder.owner.source
                    && input.binding.function_span == holder.owner.function_span
                    && input.binding.declaration_span == holder.owner.declaration_span))
    })
}

fn cyclic_cell_install<'ctx>(
    state: &EmitState<'ctx, '_>,
    environment: &crate::scalar::CallableEnvironmentId,
    to: &crate::scalar::CallableHolderPlace,
) -> Option<PointerValue<'ctx>> {
    let crate::scalar::CallableHolderPlace::Binding(target) = to else {
        return None;
    };
    if !state.cyclic_literals.contains(&environment.literal) {
        return None;
    }
    let cell = *state.capture_cells.get(target)?;
    if state
        .globals
        .values()
        .chain(state.all_globals.values())
        .any(|(global, _)| global.as_pointer_value() == cell)
    {
        return None;
    }
    state
        .literal_captures
        .get(&environment.literal)
        .filter(|producer| cyclic_callable_backedge(producer, target))
        .map(|_| cell)
}

fn collect_literal_captures<'a>(
    modules: impl Iterator<
        Item = (
            &'a wosy_syntax::SourceIdentity,
            &'a [ScalarItem],
            &'a crate::scalar::ScalarReferenceCfg,
        ),
    >,
) -> std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer> {
    let mut events = std::collections::HashMap::new();
    for (source, items, cfg) in modules {
        for point in &cfg.points {
            if let CfgPointKind::Capture { literal, .. } = &point.kind {
                events.insert(
                    wosy_syntax::SourceSpan::new(source.clone(), *literal),
                    CaptureProducer {
                        event: point.kind.clone(),
                        parent: cfg.clone(),
                        point: point.id,
                        descriptor: std::cell::OnceCell::new(),
                        counted_cells: std::cell::RefCell::new(BTreeSet::new()),
                    },
                );
            }
        }
        for item in items {
            if let ScalarItem::Function(function) = item {
                collect_function_literal_captures(source, function, &mut events);
            }
        }
    }
    events
}

fn collect_function_literal_captures(
    source: &wosy_syntax::SourceIdentity,
    function: &ScalarFunction,
    events: &mut std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
) {
    for point in &function.reference_cfg.points {
        if let CfgPointKind::Capture { literal, .. } = &point.kind {
            events.insert(
                wosy_syntax::SourceSpan::new(source.clone(), *literal),
                CaptureProducer {
                    event: point.kind.clone(),
                    parent: function.reference_cfg.clone(),
                    point: point.id,
                    descriptor: std::cell::OnceCell::new(),
                    counted_cells: std::cell::RefCell::new(BTreeSet::new()),
                },
            );
        }
    }
    for arm in &function.overload_arms {
        collect_function_literal_captures(source, arm, events);
    }
}

fn collect_literal_reference_cfgs<'a>(
    cfgs: impl Iterator<Item = &'a crate::scalar::ScalarReferenceCfg>,
) -> std::collections::HashMap<wosy_syntax::SourceSpan, crate::scalar::ScalarReferenceCfg> {
    fn insert(
        cfg: &crate::scalar::ScalarReferenceCfg,
        result: &mut std::collections::HashMap<
            wosy_syntax::SourceSpan,
            crate::scalar::ScalarReferenceCfg,
        >,
    ) {
        for literal in &cfg.literal_cfgs {
            result.insert(literal.literal.clone(), (*literal.reference_cfg).clone());
            insert(&literal.reference_cfg, result);
        }
    }
    let mut result = std::collections::HashMap::new();
    for cfg in cfgs {
        insert(cfg, &mut result);
    }
    result
}

fn resolved_calls(function: &ScalarFunction) -> BTreeMap<(u32, u32), ResolvedCallTarget> {
    resolved_cfg_calls(&function.reference_cfg)
}

fn resolved_cfg_calls(
    cfg: &crate::scalar::ScalarReferenceCfg,
) -> BTreeMap<(u32, u32), ResolvedCallTarget> {
    cfg.points
        .iter()
        .filter_map(|point| match &point.kind {
            CfgPointKind::Call { target, .. } => Some((
                (point.source_span.range.start, point.source_span.range.end),
                target.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn resolved_invalidations(
    function: &ScalarFunction,
) -> Result<
    BTreeMap<
        (u32, u32),
        (
            CoreOperationId,
            Vec<InvalidationCandidate>,
            InvalidationInputs,
        ),
    >,
    String,
> {
    let mut events = BTreeMap::new();
    for point in &function.reference_cfg.points {
        let (operation, candidates, inputs) = match &point.kind {
            CfgPointKind::Invalidate {
                operation,
                candidates,
                inputs,
                ..
            } => (*operation, candidates, inputs.clone()),
            CfgPointKind::Release {
                target: ReleaseTarget::Checked { candidates, .. },
            } => (CoreOperationId::Free, candidates, InvalidationInputs::Free),
            _ => continue,
        };
        if !function.reference_cfg.points.iter().any(|call| {
            call.source_span == point.source_span && matches!(&call.kind,
                CfgPointKind::Call { target: ResolvedCallTarget::Core(target), .. } if *target == operation)
        }) {
            return Err("core invalidation lacks a resolved call target".to_owned());
        }
        if candidates.is_empty() {
            return Err("core invalidation lacks a resolved allocation place".to_owned());
        }
        let range = point.source_span.range;
        events.insert(
            (range.start, range.end),
            (operation, candidates.clone(), inputs),
        );
    }
    Ok(events)
}

#[derive(Clone, Copy)]
struct ProjectCallScope<'a> {
    module: &'a ScalarModule,
    modules: &'a [&'a ScalarModule],
}

struct LlvmExternIdentity {
    internal_name: String,
    import_module: String,
    import_name: String,
    signature: ScalarType,
}

const CORE_ALLOC_SYMBOL: &str = "__wosy_core_alloc";
const CORE_FREE_SYMBOL: &str = "__wosy_core_free";
const CORE_SYSTEM_PANIC_SYMBOL: &str = "__wosy_core_system_panic";

fn declare_core_runtime<'ctx>(context: &'ctx Context, module: &Module<'ctx>) {
    let pointer = context.ptr_type(AddressSpace::default());
    let u64_type = context.i64_type();
    module.add_function(
        CORE_ALLOC_SYMBOL,
        pointer.fn_type(&[u64_type.into(), u64_type.into()], false),
        None,
    );
    module.add_function(
        CORE_FREE_SYMBOL,
        context.void_type().fn_type(&[pointer.into()], false),
        None,
    );
    module.add_function(
        CORE_SYSTEM_PANIC_SYMBOL,
        context.void_type().fn_type(&[], false),
        None,
    );
}

pub fn emit_scalar_llvm(validation: &ScalarValidation) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".into());
    }
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&validation.program.source.path));
    let builder = context.create_builder();
    let target_layout = validation.program.target_layout;
    let literal_captures = collect_literal_captures(std::iter::once((
        &validation.program.source,
        validation.program.items.as_slice(),
        &validation.program.top_level_reference_cfg,
    )));
    ownership::emit_graph_mutation_primitive(
        &context,
        &module,
        target_layout,
        capture_cycle_candidate_present(&literal_captures),
    )?;
    let literal_signatures = validation
        .program
        .resolved_literals
        .borrow()
        .iter()
        .map(|entry| (entry.literal.clone(), entry.signature.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let literal_reference_cfgs = collect_literal_reference_cfgs(
        std::iter::once(&validation.program.top_level_reference_cfg).chain(
            validation
                .program
                .items
                .iter()
                .filter_map(|item| match item {
                    ScalarItem::Function(function) => Some(&function.reference_cfg),
                    _ => None,
                }),
        ),
    );
    let mut globals = BTreeMap::new();
    for item in &validation.program.items {
        if let ScalarItem::Binding(binding) = item {
            for (name, ty) in binding_receivers(binding).iter() {
                if *ty == ScalarType::Unit {
                    continue;
                }
                let llvm_ty =
                    storage_type(&context, ty, &validation.program.structs, target_layout)?;
                let global = module.add_global(llvm_ty, None, name);
                global.set_linkage(Linkage::Internal);
                global.set_initializer(&llvm_ty.const_zero());
                globals.insert(name.clone(), (global, ty.clone()));
            }
        }
    }
    let mut signatures = validation
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => {
                Some((binding.name.clone(), binding.declared_type.clone()))
            }
            ScalarItem::Function(function) => {
                Some((function.name.clone(), function.signature.clone()))
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Executable(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut externs = Vec::new();
    let mut functions = BTreeMap::new();
    let mut call_targets = BTreeMap::new();
    for function in validation
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function)
                if function.generic_parameters.is_empty() && function.overload_arms.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        })
    {
        let value = module.add_function(
            &function.name,
            definition_function_type(&context, function, target_layout)?,
            None,
        );
        name_definition_parameters(value, function);
        functions.insert(function.name.clone(), value);
        call_targets.insert(function.name.clone(), function.name.clone());
    }
    let specializations = generic_specializations(
        &validation.program.items,
        |name| {
            validation.program.items.iter().find_map(|item| match item {
                ScalarItem::Function(function) if function.name == name => Some(function),
                _ => None,
            })
        },
        |name| name.to_owned(),
    )?;
    for (lookup, function, name) in &specializations {
        let value = module.add_function(
            name,
            definition_function_type(&context, function, target_layout)?,
            None,
        );
        name_definition_parameters(value, function);
        call_targets.insert(lookup.clone(), name.clone());
        signatures.insert(name.clone(), function.signature.clone());
        functions.insert(name.clone(), value);
    }
    let overloads = selected_overload_specializations(
        &validation.program.items,
        |name| name.to_owned(),
        selected_overload_selections(&validation.program.items)
            .into_iter()
            .filter_map(|(receiver, name, selection)| {
                receiver.is_none().then_some((name, selection))
            }),
    );
    for (lookup, function, name) in &overloads {
        let value = module.add_function(
            name,
            definition_function_type(&context, function, target_layout)?,
            None,
        );
        name_definition_parameters(value, function);
        call_targets.insert(lookup.clone(), name.clone());
        signatures.insert(name.clone(), function.signature.clone());
        functions.insert(name.clone(), value);
    }
    for item in &validation.program.items {
        if let ScalarItem::Extern(extern_decl) = item {
            for function in &extern_decl.functions {
                let key = format!("{}.{}", extern_decl.binding, function.name);
                let crate::scalar::ScalarExternModule::Valid(import_module) =
                    &extern_decl.actual_module
                else {
                    continue;
                };
                let identity = llvm_extern_identity(
                    &validation.program.source,
                    &extern_decl.binding,
                    import_module,
                    &function.name,
                    &function.signature,
                );
                let value = module.add_function(
                    &identity.internal_name,
                    function_type(&context, &function.signature, target_layout)?.0,
                    None,
                );
                call_targets.insert(key, identity.internal_name.clone());
                signatures.insert(identity.internal_name.clone(), function.signature.clone());
                functions.insert(identity.internal_name.clone(), value);
                externs.push(identity);
            }
        }
    }
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    let mut escaping: BTreeMap<String, Vec<ownership::ValidatedEscape>> = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for function in validation
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) => Some(function),
            _ => None,
        })
    {
        if let Some(target) = call_targets.get(&function.name) {
            escaping.insert(
                target.clone(),
                validated_backing_maps(&function.reference_cfg).escaping,
            );
            dynamic_owner_outputs.insert(target.clone(), dynamic_return_outputs(function));
        }
    }
    for (_, function, name) in specializations.iter().chain(overloads.iter()) {
        escaping.insert(
            name.clone(),
            validated_backing_maps(&function.reference_cfg).escaping,
        );
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for function in validation
        .program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function)
                if function.generic_parameters.is_empty() && function.overload_arms.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        })
    {
        emit_function(
            &context,
            &builder,
            &literal_signatures,
            &literal_captures,
            &literal_reference_cfgs,
            &validation.program.source,
            &functions,
            &call_targets,
            &signatures,
            &dynamic_owner_outputs,
            &escaping,
            &globals,
            function,
            &function.name,
            &validation.program.items,
            &module,
            &validation.program.structs,
            target_layout,
        )?;
    }
    for (_, function, name) in &specializations {
        emit_function(
            &context,
            &builder,
            &literal_signatures,
            &literal_captures,
            &literal_reference_cfgs,
            &validation.program.source,
            &functions,
            &call_targets,
            &signatures,
            &dynamic_owner_outputs,
            &escaping,
            &globals,
            function,
            name,
            &validation.program.items,
            &module,
            &validation.program.structs,
            target_layout,
        )?;
    }
    for (_, function, name) in &overloads {
        emit_function(
            &context,
            &builder,
            &literal_signatures,
            &literal_captures,
            &literal_reference_cfgs,
            &validation.program.source,
            &functions,
            &call_targets,
            &signatures,
            &dynamic_owner_outputs,
            &escaping,
            &globals,
            function,
            name,
            &validation.program.items,
            &module,
            &validation.program.structs,
            target_layout,
        )?;
    }
    emit_main(
        &context,
        &builder,
        &literal_signatures,
        &literal_captures,
        &literal_reference_cfgs,
        &validation.program.source,
        &functions,
        &call_targets,
        &signatures,
        &dynamic_owner_outputs,
        &escaping,
        &globals,
        &validation.program.items,
        &validation.program.top_level_reference_cfg,
        &validation.program.terminal_owners,
        &module,
        &validation.program.structs,
        target_layout,
    )?;
    finish_partition(
        ManuallyDrop::into_inner(module),
        validation.program.source.path.clone(),
        &context,
        &functions,
        &signatures,
        &externs,
        target_layout,
    )
}

pub fn emit_scalar_project_llvm(
    validation: &ScalarProjectValidation,
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar project".into());
    }
    let modules = validation
        .project
        .initialization_order
        .iter()
        .map(|source| {
            validation
                .project
                .modules
                .iter()
                .find(|module| module.source == *source)
                .ok_or_else(|| format!("missing module {}", source.path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let all_structs = modules
        .iter()
        .flat_map(|module| module.structs.iter().cloned())
        .collect::<Vec<_>>();
    let module_name = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .source
        .path
        .clone();
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&module_name));
    let builder = context.create_builder();
    let target_layout = validation
        .project
        .modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .target_layout;
    let literal_captures = collect_literal_captures(modules.iter().map(|module| {
        (
            &module.source,
            module.items.as_slice(),
            &module.top_level_reference_cfg,
        )
    }));
    ownership::emit_graph_mutation_primitive(
        &context,
        &module,
        target_layout,
        capture_cycle_candidate_present(&literal_captures),
    )?;
    let literal_signatures = modules
        .iter()
        .flat_map(|module| module.resolved_literals.borrow().clone().into_iter())
        .map(|entry| (entry.literal, entry.signature))
        .collect::<std::collections::HashMap<_, _>>();
    let literal_reference_cfgs =
        collect_literal_reference_cfgs(modules.iter().flat_map(|module| {
            std::iter::once(&module.top_level_reference_cfg).chain(module.items.iter().filter_map(
                |item| match item {
                    ScalarItem::Function(function) => Some(&function.reference_cfg),
                    _ => None,
                },
            ))
        }));
    let mut globals = BTreeMap::new();
    let mut functions = BTreeMap::new();
    let mut signatures = BTreeMap::new();
    let mut externs = Vec::new();
    let mut call_targets = BTreeMap::new();
    let mut definitions = Vec::new();
    for source_module in &modules {
        for item in &source_module.items {
            if let ScalarItem::Binding(binding) = item {
                for (name, ty) in binding_receivers(binding).iter() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let name = project_global_name(&source_module.source, name);
                    let llvm_ty = storage_type(&context, ty, &all_structs, target_layout)?;
                    let global = module.add_global(llvm_ty, None, &name);
                    global.set_linkage(Linkage::Internal);
                    global.set_initializer(&llvm_ty.const_zero());
                    globals.insert(name, (global, ty.clone()));
                }
            }
            if let ScalarItem::Function(function) = item {
                if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
                    continue;
                }
                let name = project_function_name(&source_module.source, &function.name);
                let value = module.add_function(
                    &name,
                    definition_function_type(&context, function, target_layout)?,
                    None,
                );
                name_definition_parameters(value, function);
                functions.insert(name.clone(), value);
                call_targets.insert(name.clone(), name.clone());
                signatures.insert(name.clone(), function.signature.clone());
                definitions.push((source_module, function, name));
            }
            if let ScalarItem::Extern(extern_decl) = item {
                let module_name = match &extern_decl.actual_module {
                    crate::scalar::ScalarExternModule::Valid(name) => name,
                    crate::scalar::ScalarExternModule::Invalid { .. } => continue,
                };
                for function in &extern_decl.functions {
                    let identity = llvm_extern_identity(
                        &source_module.source,
                        &extern_decl.binding,
                        module_name,
                        &function.name,
                        &function.signature,
                    );
                    let value = module.add_function(
                        &identity.internal_name,
                        function_type(&context, &function.signature, target_layout)?.0,
                        None,
                    );
                    let key = project_extern_lookup_key(
                        &source_module.source,
                        &extern_decl.binding,
                        module_name,
                        &function.name,
                    );
                    call_targets.insert(key, identity.internal_name.clone());
                    signatures.insert(identity.internal_name.clone(), function.signature.clone());
                    functions.insert(identity.internal_name.clone(), value);
                    externs.push(identity);
                }
            }
        }
    }
    let mut project_specializations = Vec::new();
    for source_module in &modules {
        let mut calls = Vec::new();
        for calling_module in &modules {
            for (receiver, name, arguments) in generic_calls_in_items(&calling_module.items) {
                let called_module = match receiver {
                    Some(binding)
                        if calling_module
                            .namespace_bindings
                            .iter()
                            .any(|namespace| namespace.binding == binding) =>
                    {
                        project_namespace_target(calling_module, &modules, &binding)?
                    }
                    Some(_) => continue,
                    None => calling_module,
                };
                if called_module.source == source_module.source {
                    calls.push((name, arguments));
                }
            }
        }
        let specializations = generic_specializations_from_calls(
            calls,
            |name| {
                source_module.items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == name => Some(function),
                    _ => None,
                })
            },
            |name| project_function_name(&source_module.source, name),
        )?;
        for (lookup, function, name) in specializations {
            let value = module.add_function(
                &name,
                definition_function_type(&context, &function, target_layout)?,
                None,
            );
            name_definition_parameters(value, &function);
            call_targets.insert(lookup, name.clone());
            signatures.insert(name.clone(), function.signature.clone());
            functions.insert(name.clone(), value);
            project_specializations.push((source_module, function, name));
        }
    }
    let mut project_overloads = Vec::new();
    for source_module in &modules {
        let selections = modules.iter().flat_map(|calling_module| {
            selected_overload_selections(&calling_module.items)
                .into_iter()
                .filter_map(|(receiver, name, selection)| match receiver {
                    None if calling_module.source == source_module.source => {
                        Some((name, selection))
                    }
                    Some(binding)
                        if project_namespace_target(calling_module, &modules, &binding)
                            .is_ok_and(|target| target.source == source_module.source) =>
                    {
                        Some((name, selection))
                    }
                    _ => None,
                })
        });
        for (lookup, function, name) in selected_overload_specializations(
            &source_module.items,
            |name| project_function_name(&source_module.source, name),
            selections,
        ) {
            let value = module.add_function(
                &name,
                definition_function_type(&context, &function, target_layout)?,
                None,
            );
            name_definition_parameters(value, &function);
            call_targets.insert(lookup, name.clone());
            signatures.insert(name.clone(), function.signature.clone());
            functions.insert(name.clone(), value);
            project_overloads.push((source_module, function, name));
        }
    }
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    let mut escaping: BTreeMap<String, Vec<ownership::ValidatedEscape>> = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for (_, function, name) in &definitions {
        escaping.insert(
            name.clone(),
            validated_backing_maps(&function.reference_cfg).escaping,
        );
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for (_, function, name) in &project_specializations {
        escaping.insert(
            name.clone(),
            validated_backing_maps(&function.reference_cfg).escaping,
        );
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for (_, function, name) in &project_overloads {
        escaping.insert(
            name.clone(),
            validated_backing_maps(&function.reference_cfg).escaping,
        );
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for (source_module, function, name) in definitions {
        emit_project_function(
            &context,
            &builder,
            &literal_signatures,
            &literal_captures,
            &literal_reference_cfgs,
            &functions,
            &call_targets,
            &signatures,
            &dynamic_owner_outputs,
            &escaping,
            function,
            source_module,
            &modules,
            &globals,
            &name,
            &module,
            &all_structs,
            target_layout,
        )?;
    }
    for (source_module, function, name) in &project_specializations {
        emit_project_function(
            &context,
            &builder,
            &literal_signatures,
            &literal_captures,
            &literal_reference_cfgs,
            &functions,
            &call_targets,
            &signatures,
            &dynamic_owner_outputs,
            &escaping,
            function,
            source_module,
            &modules,
            &globals,
            name,
            &module,
            &all_structs,
            target_layout,
        )?;
    }
    for (source_module, function, name) in &project_overloads {
        emit_project_function(
            &context,
            &builder,
            &literal_signatures,
            &literal_captures,
            &literal_reference_cfgs,
            &functions,
            &call_targets,
            &signatures,
            &dynamic_owner_outputs,
            &escaping,
            function,
            source_module,
            &modules,
            &globals,
            name,
            &module,
            &all_structs,
            target_layout,
        )?;
    }
    emit_project_main(
        &context,
        &builder,
        &literal_signatures,
        &literal_captures,
        &literal_reference_cfgs,
        &functions,
        &call_targets,
        &signatures,
        &dynamic_owner_outputs,
        &escaping,
        &modules,
        &validation.project.terminal_owners,
        &globals,
        &module,
        &all_structs,
        target_layout,
    )?;
    finish_partition(
        ManuallyDrop::into_inner(module),
        module_name,
        &context,
        &functions,
        &signatures,
        &externs,
        target_layout,
    )
}

pub fn emit_scalar_llvm_text(validation: &ScalarValidation) -> Result<String, String> {
    Ok(emit_scalar_llvm(validation)?.to_text())
}

impl LlvmPartition {
    pub fn to_text(&self) -> String {
        let mut text = self.text.clone();
        if !self.declarations.is_empty() {
            let module_directives = text.lines().take(2).collect::<Vec<_>>().join("\n");
            let body = text
                .lines()
                .skip(2)
                .filter(|line| {
                    !self.declarations.iter().any(|declaration| {
                        line.starts_with("declare ")
                            && line.contains(&format!("@{}(", declaration.name))
                    })
                })
                .collect::<Vec<_>>()
                .join("\n");
            let mut serialized = String::new();
            serialized.push_str(&module_directives);
            serialized.push_str("\n\n");
            for declaration in &self.declarations {
                let line = text
                    .lines()
                    .find(|line| {
                        line.starts_with("declare ")
                            && line.contains(&format!("@{}(", declaration.name))
                    })
                    .expect("LLVM extern declaration is present in module text");
                serialized.push_str(line);
                write!(serialized, " #{}", declaration.attributes[0].group,)
                    .expect("writing to a String cannot fail");
                serialized.push('\n');
            }
            serialized.push('\n');
            for attributes in declaration_attributes(&self.declarations) {
                writeln!(
                    serialized,
                    "attributes #{} = {{ \"wasm-import-module\"=\"{}\" \"wasm-import-name\"=\"{}\" }}",
                    attributes.group,
                    attributes.wasm_import_module,
                    attributes.wasm_import_name
                )
                .expect("writing to a String cannot fail");
            }
            serialized.push_str(&body);
            text = serialized;
        } else {
            let has_trailing_newline = text.ends_with('\n');
            let mut lines = text.lines().collect::<Vec<_>>();
            if let Some(index) = lines
                .iter()
                .position(|line| line.starts_with("source_filename"))
            {
                if lines.get(index + 1) == Some(&"") && lines.get(index + 2) == Some(&"") {
                    lines.remove(index + 2);
                }
            }
            text = lines.join("\n");
            if has_trailing_newline {
                text.push('\n');
            }
        }
        text
    }
}

fn declaration_attributes(declarations: &[LlvmFunction]) -> Vec<&LlvmFunctionAttributes> {
    let mut attributes = declarations
        .iter()
        .flat_map(|declaration| declaration.attributes.iter())
        .collect::<Vec<_>>();
    attributes.sort_by_key(|attribute| attribute.group);
    attributes
}

fn finish_partition<'ctx>(
    module: inkwell::module::Module<'ctx>,
    module_name: String,
    context: &'ctx Context,
    functions: &BTreeMap<String, FunctionValue<'ctx>>,
    signatures: &BTreeMap<String, ScalarType>,
    externs: &[LlvmExternIdentity],
    target_layout: ScalarTargetLayout,
) -> Result<LlvmPartition, String> {
    module.verify().map_err(|error| error.to_string())?;
    let text = module.print_to_string().to_string();
    let metadata = functions
        .iter()
        .filter_map(|(name, function)| {
            let signature = if name == "main" {
                ScalarType::Callable {
                    outputs: crate::ScalarOutputSequence {
                        outputs: vec![crate::ScalarOutput {
                            ty: ScalarType::I32,
                            span: wosy_syntax::ByteSpan::new(0, 0),
                        }],
                        span: wosy_syntax::ByteSpan::new(0, 0),
                    },
                    parameters: Vec::new(),
                }
            } else {
                signatures.get(name)?.clone()
            };
            let (_, result) = function_type(context, &signature, target_layout).ok()?;
            let ScalarType::Callable {
                outputs: _,
                parameters,
            } = signature
            else {
                return None;
            };
            let parameters = parameters
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    let parameter = function.get_nth_param(index as u32)?;
                    let name = parameter.get_name();
                    Some((name.to_string_lossy().to_string(), value_type(ty).ok()?))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(LlvmFunction {
                name: name.clone(),
                result,
                parameters,
                attributes: Vec::new(),
                body: String::new(),
            })
        })
        .collect();
    let declarations = externs
        .iter()
        .enumerate()
        .filter_map(|(group, extern_identity)| {
            let function = functions.get(&extern_identity.internal_name)?;
            let ScalarType::Callable {
                outputs: _,
                parameters,
            } = &extern_identity.signature
            else {
                return None;
            };
            let (_, result) =
                function_type(context, &extern_identity.signature, target_layout).ok()?;
            let parameters = parameters
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    let value = function.get_nth_param(index as u32)?;
                    Some((
                        value.get_name().to_string_lossy().to_string(),
                        value_type(ty).ok()?,
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(LlvmFunction {
                name: extern_identity.internal_name.clone(),
                result,
                parameters,
                attributes: vec![LlvmFunctionAttributes {
                    group: group as u32,
                    wasm_import_module: extern_identity.import_module.clone(),
                    wasm_import_name: extern_identity.import_name.clone(),
                }],
                body: String::new(),
            })
        })
        .collect();
    Ok(LlvmPartition {
        module_name,
        declarations,
        functions: metadata,
        text,
    })
}

fn function_type<'ctx>(
    context: &'ctx Context,
    signature: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<(FunctionType<'ctx>, LlvmValueType), String> {
    let ScalarType::Callable {
        outputs,
        parameters,
    } = signature
    else {
        return Err("function has no callable signature".into());
    };
    let parameters = parameters
        .iter()
        .map(|ty| basic_type(context, ty, target_layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    match outputs.outputs.as_slice() {
        [] => Ok((
            context.void_type().fn_type(&parameters, false),
            LlvmValueType::Void,
        )),
        [output] if output.ty == ScalarType::Unit => Ok((
            context.void_type().fn_type(&parameters, false),
            LlvmValueType::Void,
        )),
        [output] => Ok((
            basic_type(context, &output.ty, target_layout)?.fn_type(&parameters, false),
            value_type(&output.ty)?,
        )),
        _ => Ok((
            aggregate_type(context, outputs, target_layout)?.fn_type(&parameters, false),
            LlvmValueType::Aggregate(
                outputs
                    .outputs
                    .iter()
                    .map(|output| {
                        if output.ty == ScalarType::Unit {
                            Ok(LlvmValueType::I8)
                        } else {
                            value_type(&output.ty)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        )),
    }
}

fn definition_function_type<'ctx>(
    context: &'ctx Context,
    function: &ScalarFunction,
    target_layout: ScalarTargetLayout,
) -> Result<FunctionType<'ctx>, String> {
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &function.signature
    else {
        return Err("function has no callable signature".to_owned());
    };
    let mut arguments = parameters
        .iter()
        .flat_map(|ty| {
            let mut types = vec![basic_type(context, ty, target_layout).map(Into::into)];
            if matches!(ty, ScalarType::RuntimeArray { .. }) {
                types.push(Ok(context.i64_type().into()));
            }
            types
        })
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    for _ in dynamic_return_outputs(function) {
        arguments.push(context.ptr_type(AddressSpace::default()).into());
    }
    for output in &outputs.outputs {
        if matches!(output.ty, ScalarType::RuntimeArray { .. }) {
            arguments.push(context.ptr_type(AddressSpace::default()).into());
        }
    }
    match outputs.outputs.as_slice() {
        [] => Ok(context.void_type().fn_type(&arguments, false)),
        [output] if output.ty == ScalarType::Unit => {
            Ok(context.void_type().fn_type(&arguments, false))
        }
        [output] => Ok(basic_type(context, &output.ty, target_layout)?.fn_type(&arguments, false)),
        _ => Ok(aggregate_type(context, outputs, target_layout)?.fn_type(&arguments, false)),
    }
}

fn name_definition_parameters(value: FunctionValue<'_>, function: &ScalarFunction) {
    let ScalarType::Callable { parameters, .. } = &function.signature else {
        return;
    };
    let mut position = 0;
    for (name, ty) in function.parameters.iter().zip(parameters) {
        value
            .get_nth_param(position)
            .expect("defined parameter")
            .set_name(name);
        position += 1;
        if matches!(ty, ScalarType::RuntimeArray { .. }) {
            value
                .get_nth_param(position)
                .expect("defined array length")
                .set_name(&format!("{name}_length"));
            position += 1;
        }
    }
}

mod closure;
mod memory;
use memory::{
    aggregate_type, allocation_layout, basic_type, build_aggregate, builder_error,
    emit_array_literal, emit_place_value, emit_place_value_in_project, emit_struct_literal,
    emit_typed_expression, emit_utf8_literal, entry_alloca, evaluate_callable_selection,
    extract_output, float_constant, integer_constant, integer_scalar_type, integer_type,
    integer_width, place_pointer, pointer_integer_type, record_selected_element,
    resolve_callable_selection, storage_type, store_value, store_value_locked, struct_member_place,
    structure, take_basic, value_type,
};
fn emit_binding_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
) -> Result<EmitValue<'ctx>, String> {
    reject_untransported_callable_name(state, binding)?;
    if binding.output_sequence.outputs.len() == 1 {
        return emit_typed_expression(context, state, &binding.value, &binding.declared_type);
    }
    match binding.output_origin {
        ScalarBindingOutputOrigin::SingleExpression => {
            emit_expression(context, state, &binding.value)
        }
        ScalarBindingOutputOrigin::IndependentExpressions => {
            let values = binding
                .output_values
                .iter()
                .map(|output| emit_typed_expression(context, state, &output.value, &output.ty))
                .collect::<Result<Vec<_>, _>>()?;
            build_aggregate(context, state, &binding.output_sequence, values)
        }
    }
}

fn store_binding_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    if binding.is_allocation {
        let receiver = binding
            .receivers
            .first()
            .ok_or_else(|| "runtime allocation has no receiver".to_owned())?;
        let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
            return Err("allocation binding requires a runtime array type".to_owned());
        };
        let length = receiver
            .allocation_length
            .as_ref()
            .ok_or_else(|| "runtime allocation has no length".to_owned())?;
        let length = take_basic(emit_typed_expression(
            context,
            state,
            length,
            &ScalarType::U64,
        )?)?
        .into_int_value();
        let slot = emit_runtime_array_allocation(context, state, element, length)?;
        let length_slot = entry_alloca(
            state,
            context.i64_type().into(),
            &format!("{}_length", receiver.name),
        )?;
        state
            .builder
            .build_store(length_slot, length)
            .map_err(builder_error)?;
        state
            .runtime_array_lengths
            .insert(receiver.name.clone(), length_slot);
        if !state
            .mutable_runtime_allocations
            .contains(&(receiver.name_span.start, receiver.name_span.end))
        {
            state
                .storage
                .insert(receiver.name.clone(), (slot, receiver.ty.clone()));
            if matches!(element.as_ref(), ScalarType::Callable { .. }) {
                state.holder_events.register_binding(binding, 0, slot)?;
            }
            state
                .runtime_array_owners
                .insert(receiver.name.clone(), true);
            state
                .runtime_array_allocations
                .insert(receiver.name.clone(), true);
            record_backing_point_values(
                state,
                &receiver.name,
                backing_transport_span(receiver.name_span),
            );
            return Ok(());
        }
        let backing = entry_alloca(
            state,
            context.ptr_type(AddressSpace::default()).into(),
            &format!("{}_backing", receiver.name),
        )?;
        state
            .builder
            .build_store(backing, slot)
            .map_err(builder_error)?;
        let live = entry_alloca(
            state,
            context.bool_type().into(),
            &format!("{}_live", receiver.name),
        )?;
        state
            .builder
            .build_store(live, context.bool_type().const_int(1, false))
            .map_err(builder_error)?;
        state.runtime_array_live.insert(receiver.name.clone(), live);
        state
            .storage
            .insert(receiver.name.clone(), (backing, receiver.ty.clone()));
        if matches!(element.as_ref(), ScalarType::Callable { .. }) {
            state.holder_events.register_binding(binding, 0, backing)?;
        }
        state
            .runtime_array_owners
            .insert(receiver.name.clone(), true);
        state
            .runtime_array_allocations
            .insert(receiver.name.clone(), false);
        record_backing_point_values(
            state,
            &receiver.name,
            backing_transport_span(receiver.name_span),
        );
        return Ok(());
    }
    let receivers = binding_receivers(binding);
    let mut staged = Vec::new();
    for (position, (_, ty)) in receivers.iter().enumerate() {
        let value = extract_output(state, value.clone(), position)?;
        let (source, span, _) = receiver_source(binding, position);
        staged.push(apply_runtime_value(
            context, state, span, ty, source, value,
        )?);
    }
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    for (position, (name, ty)) in receivers.iter().enumerate() {
        let receiver_span = binding
            .receivers
            .get(position)
            .map_or(binding.name_span, |receiver| receiver.name_span);
        let copy_required = matches!(ty, ScalarType::Struct(_))
            && state
                .backing
                .copies
                .contains_key(&(receiver_span.start, receiver_span.end));
        state
            .holder_events
            .binding(binding, position, copy_required)?;
        let value = staged[position].clone();
        if *ty == ScalarType::Unit {
            state.values.insert(name.clone(), EmitValue::Unit);
            continue;
        }
        let slot = entry_alloca(
            state,
            storage_type(context, ty, state.structs, state.target_layout)?,
            name,
        )?;
        let (source, span, output) = receiver_source(binding, position);
        store_value_locked(context, state, slot, ty, value)?;
        state.storage.insert(name.clone(), (slot, ty.clone()));
        if matches!(ty, ScalarType::Callable { .. } | ScalarType::Struct(_))
            || matches!(ty, ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. }
                if matches!(element.as_ref(), ScalarType::Callable { .. }))
        {
            state
                .holder_events
                .register_binding(binding, position, slot)?;
        }
        bind_runtime_length(context, state, name, ty, source, output)?;
        apply_runtime_ownership(state, name, span, ty, source)?;
        apply_struct_backings(context, state, name, slot, ty, source, Some(span), true)?;
        transition_automatic_return_result_binding_initialization(
            state,
            name,
            ty,
            &binding.value,
            position,
        )?;
    }
    emit_binding_fixed_copy_actions_locked(context, state, binding, &receivers)?;
    for edge in state.holder_events.binding_action_edges(binding)? {
        emit_connected_holder_actions_locked(context, state, edge, None)?;
    }
    ownership::end_graph_mutation(&state.builder, state.module)?;
    Ok(())
}

fn emit_binding_fixed_copy_actions_locked<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    receivers: &[(String, ScalarType)],
) -> Result<(), String> {
    for (position, (_, ty)) in receivers.iter().enumerate() {
        if !matches!(ty, ScalarType::RuntimeArray { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
        {
            continue;
        }
        let (_, span, _) = receiver_source(binding, position);
        let Some((RuntimeBackingTransfer::CopyFixed { source }, _, _)) =
            state.backing.transfers.get(&span)
        else {
            continue;
        };
        let target = state.holder_events.binding_id(binding, position)?;
        let receiver_span = binding.receivers[position].name_span;
        for edge in state
            .holder_events
            .fixed_copy_action_edges(&target, source, receiver_span)?
        {
            if state
                .holder_events
                .connected_actions(edge)?
                .iter()
                .any(|action| {
                    matches!(
                        action,
                        crate::scalar::CallableHolderAction::DropArrayElements { .. }
                    )
                })
            {
                return Err(format!("fixed callable binding at {receiver_span:?} has an unexpected old-element Drop"));
            }
            emit_connected_holder_actions_locked(context, state, edge, Some(false))?;
        }
    }
    Ok(())
}

fn store_global_binding_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    let receivers = binding_receivers(binding);
    let mut staged = Vec::new();
    for (position, (name, ty)) in receivers.iter().enumerate() {
        if *ty == ScalarType::Unit {
            staged.push(None);
            continue;
        }
        let (global, _) = state
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown LLVM global {name} at {:?}", state.source))?;
        let (source, span, _) = receiver_source(binding, position);
        let output = extract_output(state, value.clone(), position)?;
        let prepared = apply_runtime_value(context, state, span, ty, source, output)?;
        staged.push(Some((global, prepared)));
    }
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    for (position, (name, ty)) in receivers.iter().enumerate() {
        let copy_required = matches!(ty, ScalarType::Struct(_))
            && state.backing.copies.contains_key(&(
                binding
                    .receivers
                    .get(position)
                    .map_or(binding.name_span, |receiver| receiver.name_span)
                    .start,
                binding
                    .receivers
                    .get(position)
                    .map_or(binding.name_span, |receiver| receiver.name_span)
                    .end,
            ));
        state
            .holder_events
            .binding(binding, position, copy_required)?;
        if *ty == ScalarType::Unit {
            continue;
        }
        let (global, prepared) = staged[position].as_ref().expect("non-unit global output");
        let (source, span, output) = receiver_source(binding, position);
        let slot = global.as_pointer_value();
        store_value_locked(context, state, slot, ty, prepared.clone())?;
        if matches!(ty, ScalarType::Callable { .. } | ScalarType::Struct(_))
            || matches!(ty, ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. }
                if matches!(element.as_ref(), ScalarType::Callable { .. }))
        {
            state
                .holder_events
                .register_binding(binding, position, slot)?;
        }
        if matches!(ty, ScalarType::RuntimeArray { .. }) {
            state.storage.insert(name.clone(), (slot, ty.clone()));
        }
        bind_runtime_length(context, state, name, ty, source, output)?;
        apply_runtime_ownership(state, name, span, ty, source)?;
        apply_struct_backings(context, state, name, slot, ty, source, Some(span), true)?;
    }
    emit_binding_fixed_copy_actions_locked(context, state, binding, &receivers)?;
    for edge in state.holder_events.binding_action_edges(binding)? {
        emit_connected_holder_actions_locked(context, state, edge, None)?;
    }
    ownership::end_graph_mutation(&state.builder, state.module)
}

fn receiver_source(
    binding: &crate::ScalarBinding,
    position: usize,
) -> (&ScalarExpression, (u32, u32), usize) {
    match binding.output_origin {
        ScalarBindingOutputOrigin::SingleExpression => (
            &binding.value,
            backing_transport_span(binding.name_span),
            position,
        ),
        ScalarBindingOutputOrigin::IndependentExpressions => {
            if let Some(output) = binding.output_values.get(position) {
                (
                    &output.value,
                    backing_transport_span(output.span),
                    output.position,
                )
            } else {
                (
                    &binding.value,
                    backing_transport_span(binding.name_span),
                    position,
                )
            }
        }
    }
}

fn record_backing_point_values(state: &mut EmitState<'_, '_>, key: &str, span: (u32, u32)) {
    if let Some((_, facts, _)) = state.backing.transfers.get(&span) {
        let values = ownership::owned_backing_value_facts(facts);
        if !values.is_empty() {
            state.backing_values.insert(key.into(), values);
        }
    }
}

fn transition_automatic_return_result_binding_initialization(
    state: &mut EmitState<'_, '_>,
    name: &str,
    ty: &ScalarType,
    value: &ScalarExpression,
    output: usize,
) -> Result<(), String> {
    let ScalarType::CheckedReference { inner, .. } = ty else {
        return Ok(());
    };
    let (source, owner_flag) = match value {
        ScalarExpression::Call { span, .. } => match state
            .checked_call_owners
            .get(&(span.start, span.end, output))
            .ok_or_else(|| format!("missing validated checked-call owner at {span:?}"))?
        {
            CheckedCallOwner::Fresh => (true, state.call_owner_flags.get(&output).copied()),
            CheckedCallOwner::Borrowed => (false, None),
            CheckedCallOwner::Mixed => (
                true,
                Some(
                    state
                        .call_owner_flags
                        .get(&output)
                        .copied()
                        .ok_or_else(|| format!("missing mixed-call owner flag at {span:?}"))?,
                ),
            ),
        },
        ScalarExpression::Name { name: source, .. } => (
            state.automatic_return_result_owners.contains_key(source),
            state.checked_owner_flags.get(source).copied(),
        ),
        _ => (false, None),
    };
    if source {
        if let Some(owner_flag) = owner_flag {
            state
                .checked_owner_flags
                .insert(name.to_owned(), owner_flag);
        }
        state.automatic_return_result_owners.insert(
            name.to_owned(),
            ScalarAutomaticReturnResult {
                aggregate_type: inner.as_ref().clone(),
                allocation_identity: ScalarAllocationIdentity {
                    binding: name.to_owned(),
                },
                state: ScalarAutomaticReturnResultState::Live,
            },
        );
        if let ScalarExpression::Name { name: source, .. } = value {
            state.automatic_return_result_owners.remove(source);
            state.checked_owner_flags.remove(source);
        }
    }
    Ok(())
}

fn release_automatic_return_result_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outer_owners: &BTreeMap<String, ScalarAutomaticReturnResult>,
) -> Result<(), String> {
    let owners = state
        .automatic_return_result_owners
        .keys()
        .filter(|name| !outer_owners.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    for name in owners {
        let (slot, ty) = state
            .storage
            .get(&name)
            .cloned()
            .ok_or_else(|| format!("missing automatic return-result storage for {name}"))?;
        let address = state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                slot,
                "return_result_owner",
            )
            .map_err(builder_error)?
            .into_int_value();
        let block = state
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing return-result release block".to_owned())?;
        let function = block
            .get_parent()
            .ok_or_else(|| "missing return-result release function".to_owned())?;
        let free = context.append_basic_block(function, "return_result_release.live");
        let next = context.append_basic_block(function, "return_result_release.next");
        let mut is_live = state
            .builder
            .build_int_compare(
                IntPredicate::NE,
                address,
                address.get_type().const_zero(),
                "return_result_release_live",
            )
            .map_err(builder_error)?;
        if let Some(flag) = state.checked_owner_flags.get(&name) {
            is_live = state
                .builder
                .build_and(is_live, *flag, "return_result_owned")
                .map_err(builder_error)?;
        }
        state
            .builder
            .build_conditional_branch(is_live, free, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(free);
        let pointer = state
            .builder
            .build_int_to_ptr(
                address,
                context.ptr_type(AddressSpace::default()),
                "return_result_release",
            )
            .map_err(builder_error)?;
        declare_core_runtime(context, state.module);
        let release = state
            .module
            .get_function(CORE_FREE_SYMBOL)
            .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
        state
            .builder
            .build_call(release, &[pointer.into()], "return_result_release")
            .map_err(builder_error)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
        state.automatic_return_result_owners.remove(&name);
        state.checked_owner_flags.remove(&name);
    }
    Ok(())
}

fn backing_transport_span(span: wosy_syntax::ByteSpan) -> (u32, u32) {
    (span.start, span.end)
}

fn place_key(state: &EmitState<'_, '_>, place: &crate::ScalarPlace) -> Result<String, String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => Ok(name.clone()),
        crate::ScalarPlace::Field { base, field, .. } => {
            let base = place_key(state, base)?;
            let ScalarFieldReference::Resolved(field) = field else {
                return Err("unresolved LLVM struct field".to_owned());
            };
            let structure = structure(state.structs, field.structure.clone())?;
            let declared = structure
                .fields
                .get(field.index)
                .filter(|candidate| candidate.id == *field)
                .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?;
            Ok(format!("{base}.{}", declared.name))
        }
        crate::ScalarPlace::Index { base, index, .. } => {
            let base = place_key(state, base)?;
            let (start, end) = expression_span_of(index);
            Ok(format!("{base}[{start}:{end}]"))
        }
        crate::ScalarPlace::Dereference { .. } => {
            Err("dereferenced place has no backing owner key".to_owned())
        }
    }
}

fn expression_span_of(expression: &ScalarExpression) -> (u32, u32) {
    let span = match expression {
        ScalarExpression::Assignment(assignment) => assignment.span,
        ScalarExpression::FunctionLiteral { span, .. }
        | ScalarExpression::Name { span, .. }
        | ScalarExpression::Member { span, .. }
        | ScalarExpression::Dereference { span, .. }
        | ScalarExpression::IndexedRead { span, .. }
        | ScalarExpression::Integer { span, .. }
        | ScalarExpression::InvalidInteger { span, .. }
        | ScalarExpression::Float { span, .. }
        | ScalarExpression::InvalidFloat { span, .. }
        | ScalarExpression::Boolean { span, .. }
        | ScalarExpression::Char { span, .. }
        | ScalarExpression::Utf8 { span, .. }
        | ScalarExpression::RawAddress { span, .. }
        | ScalarExpression::CheckedAddress { span, .. }
        | ScalarExpression::Unary { span, .. }
        | ScalarExpression::Binary { span, .. }
        | ScalarExpression::Call { span, .. }
        | ScalarExpression::If { span, .. }
        | ScalarExpression::UnitIf { span, .. } => *span,
        ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::ArrayLiteral { span, .. } => *span,
        ScalarExpression::Block(block) => block.span,
    };
    (span.start, span.end)
}

fn struct_field_address<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    root: PointerValue<'ctx>,
    root_ty: &ScalarType,
    path: &[String],
) -> Result<PointerValue<'ctx>, String> {
    let mut pointer = root;
    let mut ty = root_ty.clone();
    for name in path {
        let ScalarType::Struct(id) = &ty else {
            return Err(format!("backing field path crosses a non-struct value"));
        };
        let structure = state
            .structs
            .iter()
            .find(|structure| structure.id == *id)
            .ok_or_else(|| format!("unknown LLVM struct {}", id.index))?;
        let field = structure
            .fields
            .iter()
            .find(|field| field.name == *name)
            .ok_or_else(|| format!("unknown LLVM struct field {name}"))?;
        let offset = field
            .offset
            .ok_or_else(|| format!("LLVM struct field {name} has no valid offset"))?;
        pointer = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                pointer,
                &[context.i8_type().const_int(offset, false)],
                name.as_str(),
            )
        }
        .map_err(|error| error.to_string())?;
        ty = field.ty.clone();
    }
    Ok(pointer)
}

fn field_path_names(path: &[String]) -> String {
    path.join(".")
}

fn apply_struct_backings<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    root: &str,
    slot: PointerValue<'ctx>,
    ty: &ScalarType,
    source: &ScalarExpression,
    copy_span: Option<(u32, u32)>,
    locked: bool,
) -> Result<(), String> {
    if let Some(span) = copy_span {
        if state.backing.copies.contains_key(&span) {
            if let ScalarExpression::Name { name, .. } = source {
                retain_shared_fields(context, state, slot, ty, name, root)?;
            }
        }
    }
    let ScalarType::Struct(id) = ty else {
        return Ok(());
    };
    let ScalarExpression::StructLiteral { fields, .. } = source else {
        return Ok(());
    };
    apply_struct_literal_backings(
        context,
        state,
        root,
        slot,
        id,
        fields,
        &mut Vec::new(),
        locked,
    )
}

fn apply_struct_literal_backings<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    root: &str,
    slot: PointerValue<'ctx>,
    struct_id: &crate::ScalarStructId,
    fields: &[crate::ScalarStructLiteralField],
    path: &mut Vec<String>,
    locked: bool,
) -> Result<(), String> {
    let structure = state
        .structs
        .iter()
        .find(|structure| structure.id == *struct_id)
        .ok_or_else(|| format!("unknown LLVM struct {}", struct_id.index))?;
    let declared = structure.fields.clone();
    for field in fields {
        let Some(declared) = declared
            .iter()
            .find(|candidate| candidate.name == field.name)
        else {
            continue;
        };
        path.push(field.name.clone());
        let key = format!("{root}.{}", field_path_names(path));
        if matches!(declared.ty, ScalarType::RuntimeArray { .. }) {
            let address = struct_field_address(
                context,
                state,
                slot,
                &ScalarType::Struct(struct_id.clone()),
                path,
            )?;
            apply_field_backing(
                context,
                state,
                &key,
                address,
                &field.value,
                field.span,
                locked,
            )?;
        } else if matches!(declared.ty, ScalarType::Struct(_)) {
            if let ScalarExpression::StructLiteral { fields: nested, .. } = &field.value {
                let address = struct_field_address(
                    context,
                    state,
                    slot,
                    &ScalarType::Struct(struct_id.clone()),
                    path,
                )?;
                let ScalarType::Struct(nested_id) = &declared.ty else {
                    path.pop();
                    continue;
                };
                apply_struct_literal_backings(
                    context, state, root, address, nested_id, nested, path, locked,
                )?;
            }
        }
        path.pop();
    }
    Ok(())
}

fn apply_field_backing<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &str,
    address: PointerValue<'ctx>,
    value: &ScalarExpression,
    span: wosy_syntax::ByteSpan,
    locked: bool,
) -> Result<(), String> {
    if locked {
        release_backing_entry_locked(context, state, key)?;
    } else {
        release_backing_entry(context, state, key)?;
    }
    let Some((transfer, facts)) = state
        .backing
        .places
        .get(&backing_transport_span(span))
        .cloned()
    else {
        if let ScalarExpression::Name { name, .. } = value {
            if state
                .runtime_array_owners
                .get(name)
                .copied()
                .unwrap_or(false)
            {
                return Err(ownership::missing_backing_transport(
                    &format!("runtime-array field `{key}`"),
                    backing_transport_span(span),
                ));
            }
        }
        return Ok(());
    };
    match transfer {
        RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed => {
            return Err(ownership::missing_backing_transport(
                &format!("runtime-array field `{key}`"),
                backing_transport_span(span),
            ));
        }
        RuntimeBackingTransfer::CopyFixed { .. } | RuntimeBackingTransfer::Move { .. } => {
            let ScalarExpression::Name { name, .. } = value else {
                return Err(format!(
                    "validated backing transport disagrees with field shape at {}..{}",
                    span.start, span.end
                ));
            };
            let owned = ownership::owned_backing_value_facts(&facts);
            let source_type = storage_backing_type(state, name);
            match source_type.clone() {
                Some(ScalarType::Array {
                    element, length, ..
                }) => {
                    let current = state
                        .builder
                        .build_load(
                            context.ptr_type(AddressSpace::default()),
                            address,
                            "field_backing",
                        )
                        .map_err(builder_error)?
                        .into_pointer_value();
                    let destination = ownership::materialize_fixed_copy(
                        context,
                        state,
                        current,
                        &element,
                        length,
                        ownership::FixedCopyPublication::RequiresHolderAction,
                    )?;
                    state
                        .builder
                        .build_store(address, destination)
                        .map_err(builder_error)?;
                }
                Some(ScalarType::RuntimeArray { .. }) => {
                    transfer_backing_ownership(state, name);
                }
                _ => {
                    return Err(format!(
                        "validated backing source `{name}` has an invalid type at {}..{}",
                        span.start, span.end
                    ));
                }
            }
            register_backing_entry(state, key.into(), owned);
            state.backing_slots.insert(
                key.into(),
                (
                    address,
                    match source_type {
                        Some(ScalarType::Array { element, .. })
                        | Some(ScalarType::RuntimeArray { element, .. }) => {
                            ScalarType::RuntimeArray { element, span }
                        }
                        _ => unreachable!("validated runtime-array source"),
                    },
                ),
            );
        }
    }
    Ok(())
}

fn retain_shared_fields<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    slot: PointerValue<'ctx>,
    ty: &ScalarType,
    source: &str,
    root: &str,
) -> Result<(), String> {
    let prefix = format!("{source}.");
    let shared = state
        .backing_values
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(key, values)| (key.clone(), values.clone()))
        .collect::<Vec<_>>();
    for (key, values) in shared {
        let suffix = key[prefix.len()..].to_owned();
        let path = suffix.split('.').map(str::to_owned).collect::<Vec<_>>();
        let Ok(address) = struct_field_address(context, state, slot, ty, &path) else {
            continue;
        };
        let mut field_type = ty.clone();
        for part in &path {
            let ScalarType::Struct(id) = &field_type else {
                return Err(format!("invalid runtime-array field path {key}"));
            };
            field_type = structure(state.structs, id.clone())?
                .fields
                .iter()
                .find(|field| field.name == *part)
                .ok_or_else(|| format!("unknown runtime-array field {part}"))?
                .ty
                .clone();
        }
        let ScalarType::RuntimeArray { .. } = field_type else {
            return Err(format!("invalid runtime-array field type {key}"));
        };
        let target = format!("{root}.{suffix}");
        state.runtime_array_owners.insert(target.clone(), false);
        state
            .runtime_array_allocations
            .insert(target.clone(), false);
        state.backing_values.insert(target.clone(), values);
        state.backing_slots.insert(target, (address, field_type));
    }
    Ok(())
}

fn param_backing_values(
    state: &EmitState<'_, '_>,
    function_span: wosy_syntax::ByteSpan,
    body_span: wosy_syntax::ByteSpan,
    param_span: wosy_syntax::ByteSpan,
) -> Vec<RuntimeValueId> {
    if state.backing.input_paths.is_empty() {
        return vec![RuntimeValueId {
            source: state.source.clone(),
            function_span,
            creation_span: param_span,
            position: RuntimeValuePosition::Parameter {
                declaration_span: param_span,
            },
        }];
    }
    let _ = body_span;
    state
        .backing
        .input_paths
        .iter()
        .flatten()
        .filter(|fact| fact.owner.declaration_span == param_span)
        .map(|fact| fact.value.clone())
        .collect()
}

fn drop_matching_backing_entries(state: &mut EmitState<'_, '_>, values: &[RuntimeValueId]) {
    if values.is_empty() {
        return;
    }
    for key in state
        .backing_values
        .iter()
        .filter(|(_, entry)| entry.iter().any(|value| values.contains(value)))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>()
    {
        drop_backing_entry(state, &key);
    }
}

fn backing_entry_root(key: &str) -> &str {
    key.split('.').next().unwrap_or(key)
}

fn storage_backing_type(state: &EmitState<'_, '_>, name: &str) -> Option<ScalarType> {
    state
        .storage
        .get(name)
        .map(|(_, ty)| ty.clone())
        .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone()))
}

fn register_backing_entry(state: &mut EmitState<'_, '_>, key: String, values: Vec<RuntimeValueId>) {
    state.runtime_array_owners.insert(key.clone(), true);
    state.runtime_array_allocations.insert(key.clone(), false);
    state.backing_values.insert(key, values);
}

fn runtime_length_for_source<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    source: &ScalarExpression,
    output: usize,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let slot = match source {
        ScalarExpression::Name { name, .. } => {
            if let Some(ScalarType::Array { length, .. }) = storage_backing_type(state, name) {
                return Ok(context.i64_type().const_int(length, false));
            }
            state.runtime_array_lengths.get(name)
        }
        ScalarExpression::Call { .. } => state.call_length_slots.get(&output),
        ScalarExpression::Assignment(assignment) => {
            return state
                .holder_events
                .assignment_runtime_length(assignment, output);
        }
        _ => return Err("runtime-array length requires validated source transport".to_owned()),
    }
    .ok_or_else(|| "runtime-array value lacks evaluated length".to_owned())?;
    Ok(state
        .builder
        .build_load(context.i64_type(), *slot, "array_length")
        .map_err(builder_error)?
        .into_int_value())
}

fn record_return_length<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    ty: &ScalarType,
    source: &ScalarExpression,
    output: usize,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(());
    }
    let length = runtime_length_for_source(context, state, source, output)?;
    let slot = *state
        .return_length_slots
        .get(&output)
        .ok_or_else(|| format!("missing runtime-array return length slot {output}"))?;
    state
        .builder
        .build_store(slot, length)
        .map_err(builder_error)?;
    Ok(())
}

fn call_argument_lengths<'ctx>(
    context: &'ctx Context,
    state: &EmitState<'ctx, '_>,
    target: &str,
    arguments: &[ScalarExpression],
    parameters: &[ScalarType],
) -> Result<Vec<Option<inkwell::values::IntValue<'ctx>>>, String> {
    if !state.dynamic_owner_outputs.contains_key(target)
        && parameters
            .iter()
            .any(|ty| matches!(ty, ScalarType::RuntimeArray { .. }))
    {
        return Err(format!(
            "foreign runtime-array call {target} requires a declared length contract"
        ));
    }
    arguments
        .iter()
        .zip(parameters)
        .map(|(argument, parameter)| {
            if matches!(parameter, ScalarType::RuntimeArray { .. }) {
                runtime_length_for_source(context, state, argument, 0).map(Some)
            } else {
                Ok(None)
            }
        })
        .collect()
}

fn bind_runtime_length<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    key: &str,
    ty: &ScalarType,
    source: &ScalarExpression,
    output: usize,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(());
    }
    let length = runtime_length_for_source(context, state, source, output)?;
    let slot = entry_alloca(state, context.i64_type().into(), &format!("{key}_length"))?;
    state
        .builder
        .build_store(slot, length)
        .map_err(builder_error)?;
    state.runtime_array_lengths.insert(key.to_owned(), slot);
    Ok(())
}

fn drop_backing_entry(state: &mut EmitState<'_, '_>, key: &str) {
    if key == "bytes" {
        eprintln!(
            "[D3DBG] drop_backing_entry key={key}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
    }
    state.runtime_array_owners.insert(key.into(), false);
    state.runtime_array_allocations.remove(key);
    state.runtime_array_lengths.remove(key);
    state.runtime_array_live.remove(key);
    state.backing_values.remove(key);
    state.backing_slots.remove(key);
    state.imported_cell_slots.remove(key);
    state.backing_captured.remove(key);
}

fn transfer_backing_ownership(state: &mut EmitState<'_, '_>, key: &str) {
    state.runtime_array_owners.insert(key.into(), false);
    state.backing_values.remove(key);
    state.backing_slots.remove(key);
    state.imported_cell_slots.remove(key);
    state.backing_captured.remove(key);
}

fn release_backing_entry<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &str,
) -> Result<(), String> {
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    release_backing_entry_locked(context, state, key)?;
    ownership::end_graph_mutation(&state.builder, state.module)
}

fn release_backing_entry_locked<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &str,
) -> Result<(), String> {
    if !state
        .runtime_array_owners
        .get(key)
        .copied()
        .unwrap_or(false)
    {
        drop_backing_entry(state, key);
        return Ok(());
    }
    let values = state.backing_values.get(key).cloned().unwrap_or_default();
    let slot = state.backing_slots.get(key).map(|(slot, _)| *slot);
    let share = state
        .backing_values
        .iter()
        .find(|(other, other_values)| {
            (*other).as_str() != key && other_values.iter().any(|value| values.contains(value)) && {
                let self_captured = state.backing_captured.contains(key);
                let other_captured = state.backing_captured.contains(*other);
                if self_captured || other_captured {
                    match (slot, state.backing_slots.get(*other).map(|(slot, _)| *slot)) {
                        (Some(current), Some(candidate)) => current == candidate,
                        _ => false,
                    }
                } else {
                    true
                }
            }
        })
        .map(|(other, _)| other.clone());
    if let Some(share) = share {
        state.runtime_array_owners.insert(share, true);
        drop_backing_entry(state, key);
        return Ok(());
    }
    ownership::release_runtime_array_owner(context, state, key)?;
    drop_backing_entry(state, key);
    Ok(())
}

fn apply_runtime_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    span: (u32, u32),
    ty: &ScalarType,
    source: &ScalarExpression,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(value);
    }
    let Some((transfer, _, _)) = state.backing.transfers.get(&span).cloned() else {
        return match source {
            ScalarExpression::Name { name, .. } => {
                if state
                    .runtime_array_owners
                    .get(name)
                    .copied()
                    .unwrap_or(false)
                {
                    Err(ownership::missing_backing_transport(
                        &format!("runtime-array binding `{name}`"),
                        span,
                    ))
                } else {
                    Ok(value)
                }
            }
            ScalarExpression::Call { span, .. } => {
                let call = backing_transport_span(*span);
                if state
                    .backing
                    .call_facts
                    .keys()
                    .any(|(start, end, _)| (*start, *end) == (call.0, call.1))
                {
                    Err(ownership::missing_backing_transport(
                        "runtime-array call result",
                        call,
                    ))
                } else {
                    Ok(value)
                }
            }
            _ => Err(ownership::missing_backing_transport(
                "runtime-array value",
                span,
            )),
        };
    };
    match transfer {
        RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed => Err(
            ownership::missing_backing_transport("runtime-array allocation", span),
        ),
        RuntimeBackingTransfer::CopyFixed { .. } | RuntimeBackingTransfer::Move { .. } => {
            let call_output = matches!(
                transfer,
                RuntimeBackingTransfer::Move { ref source }
                    if matches!(
                        source.kind,
                        ReferenceBindingKind::CallOutput { .. }
                    )
            );
            let ScalarExpression::Name { name, .. } = source else {
                if call_output {
                    return Ok(value);
                }
                return Err(format!(
                    "validated backing transport disagrees with initializer shape at {}..{}",
                    span.0, span.1
                ));
            };
            let source_ty = storage_backing_type(state, name).ok_or_else(|| {
                ownership::missing_backing_transport(&format!("backing source `{name}`"), span)
            })?;
            match source_ty {
                ScalarType::Array {
                    element, length, ..
                } => {
                    let source_pointer = take_basic(value)?.into_pointer_value();
                    let destination = ownership::materialize_fixed_copy(
                        context,
                        state,
                        source_pointer,
                        &element,
                        length,
                        ownership::FixedCopyPublication::ConnectedHolder,
                    )?;
                    Ok(EmitValue::Basic(destination.into()))
                }
                ScalarType::RuntimeArray { .. } => Ok(value),
                _ => Err(format!(
                    "validated backing source `{name}` has an invalid type at {}..{}",
                    span.0, span.1
                )),
            }
        }
    }
}

fn apply_runtime_ownership(
    state: &mut EmitState<'_, '_>,
    key: &str,
    span: (u32, u32),
    ty: &ScalarType,
    source: &ScalarExpression,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(());
    }
    let Some((transfer, facts, _)) = state.backing.transfers.get(&span).cloned() else {
        state.runtime_array_owners.insert(key.into(), false);
        state.backing_values.remove(key);
        return Ok(());
    };
    match transfer {
        RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed => {
            return Err(ownership::missing_backing_transport(
                "runtime-array allocation",
                span,
            ));
        }
        RuntimeBackingTransfer::CopyFixed { .. } => {
            register_backing_entry(
                state,
                key.into(),
                ownership::owned_backing_value_facts(&facts),
            );
        }
        RuntimeBackingTransfer::Move { .. } => {
            if let ScalarExpression::Name { name, .. } = source {
                if name != key {
                    transfer_backing_ownership(state, name);
                }
            }
            register_backing_entry(
                state,
                key.into(),
                ownership::owned_backing_value_facts(&facts),
            );
        }
    }
    Ok(())
}

mod assignment_destination;
mod assignment_snapshot;
use assignment_destination::{
    freeze_assignment_destination, FrozenAssignmentDestination, FrozenAssignmentSelection,
};
use assignment_snapshot::{finish_assignment_outputs, snapshot_assignment_outputs};
mod collector;
mod descriptors;
mod holder_events;
mod imports;
mod ownership;
use ownership::{
    binding_receivers, checked_call_owners, dynamic_return_outputs, emit_runtime_array_allocation,
    fresh_return_outputs, initialize_joined_return_bindings, joined_assignment_owners,
    joined_return_binding_types, materialize_validated_return, release_captured_backings,
    release_scope_runtime_array_owners, transfer_runtime_array_return, validated_backing_maps,
    ValidatedBackingMaps,
};
fn emit_project_binding_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    reject_untransported_callable_name(state, binding)?;
    if binding.output_sequence.outputs.len() == 1 {
        return emit_project_typed_expression(
            context,
            state,
            &binding.value,
            &binding.declared_type,
            module,
            modules,
        );
    }
    match binding.output_origin {
        ScalarBindingOutputOrigin::SingleExpression => {
            emit_project_expression(context, state, &binding.value, module, modules)
        }
        ScalarBindingOutputOrigin::IndependentExpressions => {
            let values = binding
                .output_values
                .iter()
                .map(|output| {
                    emit_project_typed_expression(
                        context,
                        state,
                        &output.value,
                        &output.ty,
                        module,
                        modules,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            build_aggregate(context, state, &binding.output_sequence, values)
        }
    }
}

fn reject_untransported_callable_name(
    state: &EmitState<'_, '_>,
    binding: &crate::ScalarBinding,
) -> Result<(), String> {
    if let (ScalarType::Callable { .. }, ScalarExpression::Name { name, span }) =
        (&binding.declared_type, &binding.value)
    {
        if !state.storage.contains_key(name)
            && !state.globals.contains_key(name)
            && !state.values.contains_key(name)
        {
            return Err(format!(
                "unsupported source-qualified indirect callable value `{name}` at {:?}:{}..{} lacks validated callable-holder transport",
                binding.occurrence.source_span.source, span.start, span.end
            ));
        }
    }
    Ok(())
}

fn counted_capture_literals(
    items: &[&ScalarItem],
) -> Result<std::collections::HashSet<wosy_syntax::SourceSpan>, String> {
    let mut literals = std::collections::HashSet::new();
    for item in items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        let events = holder_events::HolderEvents::new(&function.reference_cfg)?;
        for coverage in events.covered_cells.values() {
            for environment in &coverage.environments {
                literals.insert(environment.literal.clone());
            }
        }
    }
    Ok(literals)
}

fn emit_function<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    literal_signatures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, ScalarType>,
    literal_captures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
    literal_reference_cfgs: &'ctx std::collections::HashMap<
        wosy_syntax::SourceSpan,
        crate::scalar::ScalarReferenceCfg,
    >,
    source: &wosy_syntax::SourceIdentity,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    escaping: &'ctx BTreeMap<String, Vec<ownership::ValidatedEscape>>,
    globals: &'ctx BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    function: &ScalarFunction,
    name: &str,
    items: &[ScalarItem],
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let (fresh_return_outputs, branch_return_outputs) = fresh_return_outputs(function)?;
    let joined_bindings = joined_return_binding_types(function, &branch_return_outputs)?;
    let joined_assignment_owners = joined_assignment_owners(function, &joined_bindings)?;
    let mut state = EmitState {
        builder,
        module,
        functions,
        call_targets,
        signatures,
        dynamic_owner_outputs,
        literal_signatures,
        literal_captures,
        literal_reference_cfgs,
        covered_literals: counted_capture_literals(&items.iter().collect::<Vec<_>>())?,
        cyclic_literals: cycle_counted_literals(literal_captures),
        counted_fixed_cells: std::collections::BTreeSet::new(),
        top_level_remainder_cells: Vec::new(),
        counted_cell_backings: counted_cell_backing_origins(literal_captures),
        source: source.clone(),
        resolved_calls: resolved_calls(function),
        capture_cells: BTreeMap::new(),
        backing: validated_backing_maps(&function.reference_cfg),
        holder_events: holder_events::HolderEvents::new(&function.reference_cfg)?,
        escaping: escaping.clone(),
        backing_values: BTreeMap::new(),
        backing_slots: BTreeMap::new(),
        imported_cell_slots: BTreeMap::new(),
        backing_captured: BTreeSet::new(),
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        fresh_return_outputs,
        branch_return_outputs,
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners,
        return_output_types: match &function.signature {
            ScalarType::Callable { outputs, .. } => outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect(),
            _ => return Err("function has no callable signature".to_owned()),
        },
        return_body_span: Some(function.body.span),
        checked_call_owners: checked_call_owners(function)?,
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
        mutable_runtime_allocations: mutable_runtime_allocations(function),
        invalidations: resolved_invalidations(function)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_joined_return_bindings(context, &mut state, joined_bindings)?;
    insert_unit_values(&mut state.values, items);
    register_global_holder_storage(
        &mut state.holder_events,
        source,
        items,
        globals,
        &[],
        globals,
    )?;
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        let explicit_count = parameters.len()
            + parameters
                .iter()
                .filter(|ty| matches!(ty, ScalarType::RuntimeArray { .. }))
                .count();
        for (position, output) in dynamic_return_outputs(function).into_iter().enumerate() {
            let slot = value
                .get_nth_param((explicit_count + position) as u32)
                .ok_or_else(|| format!("missing mixed-return owner slot {output}"))?
                .into_pointer_value();
            state.return_owner_slots.insert(output, slot);
        }
        let mut length_index = explicit_count + dynamic_return_outputs(function).len();
        for (index, output) in state.return_output_types.iter().enumerate() {
            if matches!(output, ScalarType::RuntimeArray { .. }) {
                let slot = value
                    .get_nth_param(length_index as u32)
                    .ok_or_else(|| format!("missing runtime-array return length slot {index}"))?
                    .into_pointer_value();
                state.return_length_slots.insert(index, slot);
                length_index += 1;
            }
        }
        let mut argument_index = 0;
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(argument_index)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            argument_index += 1;
            let slot = builder
                .build_alloca(
                    storage_type(context, parameter, structs, target_layout)?,
                    &function.parameters[index],
                )
                .map_err(builder_error)?;
            store_value(
                context,
                &mut state,
                slot,
                parameter,
                EmitValue::Basic(argument),
            )?;
            state.storage.insert(
                function.parameters[index].clone(),
                (slot, parameter.clone()),
            );
            if matches!(parameter, ScalarType::RuntimeArray { .. }) {
                let length = value
                    .get_nth_param(argument_index)
                    .ok_or_else(|| "missing runtime-array parameter length".to_owned())?
                    .into_int_value();
                argument_index += 1;
                let length_slot = entry_alloca(
                    &mut state,
                    context.i64_type().into(),
                    &format!("{}_length", function.parameters[index]),
                )?;
                builder
                    .build_store(length_slot, length)
                    .map_err(builder_error)?;
                state
                    .runtime_array_lengths
                    .insert(function.parameters[index].clone(), length_slot);
                state
                    .runtime_array_owners
                    .insert(function.parameters[index].clone(), true);
                state
                    .runtime_array_allocations
                    .insert(function.parameters[index].clone(), false);
                let values = param_backing_values(
                    &state,
                    function.span,
                    function.body.span,
                    function.parameter_spans[index],
                );
                if !values.is_empty() {
                    state
                        .backing_values
                        .insert(function.parameters[index].clone(), values);
                }
            }
        }
    }
    let outputs = match &function.signature {
        ScalarType::Callable { outputs, .. } => outputs,
        _ => return Err("function has no callable signature".into()),
    };
    let mut result = match function.body.items.as_slice() {
        [ScalarBlockItem::Expression(expression)] => {
            if outputs.outputs.len() == 1 {
                emit_typed_expression(context, &mut state, expression, &outputs.outputs[0].ty)?
            } else {
                let value = emit_expression(context, &mut state, expression)?;
                if outputs
                    .outputs
                    .iter()
                    .all(|output| output.ty == outputs.outputs[0].ty)
                {
                    build_aggregate(
                        context,
                        &mut state,
                        outputs,
                        vec![value; outputs.outputs.len()],
                    )?
                } else {
                    value
                }
            }
        }
        _ => emit_block(context, &mut state, &function.body)?,
    };
    if let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() {
        result = apply_expression_body_return(
            context,
            &mut state,
            &function.body,
            outputs,
            expression,
            result,
        )?;
        emit_expression_body_return_handoff(context, &mut state, &function.body, &result)?;
    }
    if let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() {
        for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
            record_forwarded_return_owner(context, &mut state, expression, output)?;
        }
        validate_return_holder_events(&state, &function.body)?;
        let exit = state.holder_events.block_exit(&function.body.occurrence)?;
        state.holder_events.validated(exit)?;
    }
    let prepared = return_memory::prepare_return(context, &mut state, result, outputs)?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new(), &BTreeSet::new())?;
    release_captured_backings(context, &mut state)?;
    emit_return(&state, prepared)
}

fn emit_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    literal_signatures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, ScalarType>,
    literal_captures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
    literal_reference_cfgs: &'ctx std::collections::HashMap<
        wosy_syntax::SourceSpan,
        crate::scalar::ScalarReferenceCfg,
    >,
    source: &wosy_syntax::SourceIdentity,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    escaping: &'ctx BTreeMap<String, Vec<ownership::ValidatedEscape>>,
    globals: &'ctx BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    items: &[ScalarItem],
    top_level_cfg: &crate::scalar::ScalarReferenceCfg,
    terminal_owners: &[crate::scalar::CallableProgramTerminalOwner],
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        builder,
        module,
        functions,
        call_targets,
        signatures,
        dynamic_owner_outputs,
        literal_signatures,
        literal_captures,
        literal_reference_cfgs,
        covered_literals: counted_capture_literals(&items.iter().collect::<Vec<_>>())?,
        cyclic_literals: cycle_counted_literals(literal_captures),
        counted_fixed_cells: std::collections::BTreeSet::new(),
        top_level_remainder_cells: Vec::new(),
        counted_cell_backings: counted_cell_backing_origins(literal_captures),
        source: source.clone(),
        resolved_calls: resolved_cfg_calls(top_level_cfg),
        capture_cells: BTreeMap::new(),
        backing: validated_backing_maps(top_level_cfg),
        holder_events: holder_events::HolderEvents::new(top_level_cfg)?,
        escaping: escaping.clone(),
        backing_values: BTreeMap::new(),
        backing_slots: BTreeMap::new(),
        imported_cell_slots: BTreeMap::new(),
        backing_captured: BTreeSet::new(),
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        return_body_span: None,
        checked_call_owners: BTreeMap::new(),
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
        mutable_runtime_allocations: BTreeSet::new(),
        invalidations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, items);
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                let value = emit_binding_value(context, &mut state, binding)?;
                if binding.is_allocation {
                    store_binding_outputs(context, &mut state, binding, value)?;
                    continue;
                }
                store_global_binding_outputs(context, &mut state, binding, value)?;
            }
            ScalarItem::Executable(item) => {
                emit_block_item(context, &mut state, item)?;
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Function(_) => {}
        }
    }
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new(), &BTreeSet::new())?;
    release_captured_backings(context, &mut state)?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    emit_program_terminal_owners(context, &mut state, terminal_owners, false)?;
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn emit_project_function<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    literal_signatures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, ScalarType>,
    literal_captures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
    literal_reference_cfgs: &'ctx std::collections::HashMap<
        wosy_syntax::SourceSpan,
        crate::scalar::ScalarReferenceCfg,
    >,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    escaping: &'ctx BTreeMap<String, Vec<ownership::ValidatedEscape>>,
    function: &ScalarFunction,
    source_module: &ScalarModule,
    modules: &[&'module ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    name: &str,
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    eprintln!("[D3DBG] enter fn={name}");
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let (fresh_return_outputs, branch_return_outputs) = fresh_return_outputs(function)?;
    let joined_bindings = joined_return_binding_types(function, &branch_return_outputs)?;
    let joined_assignment_owners = joined_assignment_owners(function, &joined_bindings)?;
    let mut state = EmitState {
        builder,
        module,
        functions,
        call_targets,
        signatures,
        dynamic_owner_outputs,
        literal_signatures,
        literal_captures,
        literal_reference_cfgs,
        covered_literals: counted_capture_literals(
            &modules
                .iter()
                .flat_map(|module| &module.items)
                .collect::<Vec<_>>(),
        )?,
        cyclic_literals: cycle_counted_literals(literal_captures),
        counted_fixed_cells: std::collections::BTreeSet::new(),
        top_level_remainder_cells: Vec::new(),
        counted_cell_backings: counted_cell_backing_origins(literal_captures),
        source: source_module.source.clone(),
        resolved_calls: resolved_calls(function),
        capture_cells: BTreeMap::new(),
        backing: validated_backing_maps(&function.reference_cfg),
        holder_events: holder_events::HolderEvents::new(&function.reference_cfg)?,
        escaping: escaping.clone(),
        backing_values: BTreeMap::new(),
        backing_slots: BTreeMap::new(),
        imported_cell_slots: BTreeMap::new(),
        backing_captured: BTreeSet::new(),
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        fresh_return_outputs,
        branch_return_outputs,
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners,
        return_output_types: match &function.signature {
            ScalarType::Callable { outputs, .. } => outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect(),
            _ => return Err("function has no callable signature".to_owned()),
        },
        return_body_span: Some(function.body.span),
        checked_call_owners: checked_call_owners(function)?,
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
        mutable_runtime_allocations: mutable_runtime_allocations(function),
        invalidations: resolved_invalidations(function)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: module_globals(source_module, globals),
        all_globals: globals.clone(),
        structs,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_joined_return_bindings(context, &mut state, joined_bindings)?;
    insert_unit_values(&mut state.values, &source_module.items);
    register_global_holder_storage(
        &mut state.holder_events,
        &source_module.source,
        &source_module.items,
        &state.globals,
        modules,
        globals,
    )?;
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        let explicit_count = parameters.len()
            + parameters
                .iter()
                .filter(|ty| matches!(ty, ScalarType::RuntimeArray { .. }))
                .count();
        for (position, output) in dynamic_return_outputs(function).into_iter().enumerate() {
            let slot = value
                .get_nth_param((explicit_count + position) as u32)
                .ok_or_else(|| format!("missing mixed-return owner slot {output}"))?
                .into_pointer_value();
            state.return_owner_slots.insert(output, slot);
        }
        let mut length_index = explicit_count + dynamic_return_outputs(function).len();
        for (index, output) in state.return_output_types.iter().enumerate() {
            if matches!(output, ScalarType::RuntimeArray { .. }) {
                let slot = value
                    .get_nth_param(length_index as u32)
                    .ok_or_else(|| format!("missing runtime-array return length slot {index}"))?
                    .into_pointer_value();
                state.return_length_slots.insert(index, slot);
                length_index += 1;
            }
        }
        let mut argument_index = 0;
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(argument_index)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            argument_index += 1;
            let slot = builder
                .build_alloca(
                    storage_type(context, parameter, structs, target_layout)?,
                    &function.parameters[index],
                )
                .map_err(builder_error)?;
            store_value(
                context,
                &mut state,
                slot,
                parameter,
                EmitValue::Basic(argument),
            )?;
            state.storage.insert(
                function.parameters[index].clone(),
                (slot, parameter.clone()),
            );
            if matches!(parameter, ScalarType::RuntimeArray { .. }) {
                let length = value
                    .get_nth_param(argument_index)
                    .ok_or_else(|| "missing runtime-array parameter length".to_owned())?
                    .into_int_value();
                argument_index += 1;
                let length_slot = entry_alloca(
                    &mut state,
                    context.i64_type().into(),
                    &format!("{}_length", function.parameters[index]),
                )?;
                builder
                    .build_store(length_slot, length)
                    .map_err(builder_error)?;
                state
                    .runtime_array_lengths
                    .insert(function.parameters[index].clone(), length_slot);
                state
                    .runtime_array_owners
                    .insert(function.parameters[index].clone(), true);
                state
                    .runtime_array_allocations
                    .insert(function.parameters[index].clone(), false);
                let values = param_backing_values(
                    &state,
                    function.span,
                    function.body.span,
                    function.parameter_spans[index],
                );
                if !values.is_empty() {
                    state
                        .backing_values
                        .insert(function.parameters[index].clone(), values);
                }
            }
        }
    }
    let outputs = match &function.signature {
        ScalarType::Callable { outputs, .. } => outputs,
        _ => return Err("function has no callable signature".into()),
    };
    let mut result = match function.body.items.as_slice() {
        [ScalarBlockItem::Expression(expression)] => {
            if outputs.outputs.len() == 1 {
                emit_project_typed_expression(
                    context,
                    &mut state,
                    expression,
                    &outputs.outputs[0].ty,
                    source_module,
                    modules,
                )?
            } else {
                let value = emit_project_expression(
                    context,
                    &mut state,
                    expression,
                    source_module,
                    modules,
                )?;
                if outputs
                    .outputs
                    .iter()
                    .all(|output| output.ty == outputs.outputs[0].ty)
                {
                    build_aggregate(
                        context,
                        &mut state,
                        outputs,
                        vec![value; outputs.outputs.len()],
                    )?
                } else {
                    value
                }
            }
        }
        _ => emit_project_block(context, &mut state, &function.body, source_module, modules)?,
    };
    if let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() {
        result = apply_expression_body_return(
            context,
            &mut state,
            &function.body,
            outputs,
            expression,
            result,
        )?;
        emit_expression_body_return_handoff(context, &mut state, &function.body, &result)?;
    }
    if let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() {
        for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
            record_forwarded_return_owner(context, &mut state, expression, output)?;
        }
        validate_return_holder_events(&state, &function.body)?;
        let exit = state.holder_events.block_exit(&function.body.occurrence)?;
        state.holder_events.validated(exit)?;
    }
    let prepared = return_memory::prepare_return(context, &mut state, result, outputs)?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new(), &BTreeSet::new())?;
    release_captured_backings(context, &mut state)?;
    emit_return(&state, prepared)
}

fn emit_project_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    literal_signatures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, ScalarType>,
    literal_captures: &'ctx std::collections::HashMap<wosy_syntax::SourceSpan, CaptureProducer>,
    literal_reference_cfgs: &'ctx std::collections::HashMap<
        wosy_syntax::SourceSpan,
        crate::scalar::ScalarReferenceCfg,
    >,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    escaping: &'ctx BTreeMap<String, Vec<ownership::ValidatedEscape>>,
    modules: &[&ScalarModule],
    terminal_owners: &[crate::scalar::CallableProgramTerminalOwner],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        builder,
        module,
        functions,
        call_targets,
        signatures,
        dynamic_owner_outputs,
        literal_signatures,
        literal_captures,
        literal_reference_cfgs,
        covered_literals: counted_capture_literals(
            &modules
                .iter()
                .flat_map(|module| &module.items)
                .collect::<Vec<_>>(),
        )?,
        cyclic_literals: cycle_counted_literals(literal_captures),
        counted_fixed_cells: std::collections::BTreeSet::new(),
        top_level_remainder_cells: Vec::new(),
        counted_cell_backings: counted_cell_backing_origins(literal_captures),
        source: modules
            .first()
            .ok_or_else(|| "project has no reachable modules".to_owned())?
            .source
            .clone(),
        resolved_calls: BTreeMap::new(),
        capture_cells: BTreeMap::new(),
        backing: ValidatedBackingMaps::default(),
        holder_events: holder_events::HolderEvents::new(
            &modules
                .first()
                .ok_or_else(|| "project has no reachable modules".to_owned())?
                .top_level_reference_cfg,
        )?,
        escaping: escaping.clone(),
        backing_values: BTreeMap::new(),
        backing_slots: BTreeMap::new(),
        imported_cell_slots: BTreeMap::new(),
        backing_captured: BTreeSet::new(),
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        return_body_span: None,
        checked_call_owners: BTreeMap::new(),
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
        mutable_runtime_allocations: BTreeSet::new(),
        invalidations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        globals: BTreeMap::new(),
        all_globals: globals.clone(),
        structs,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    let root = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?;
    initialize_project_module(context, &mut state, root, modules, globals, &mut Vec::new())?;
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new(), &BTreeSet::new())?;
    release_captured_backings(context, &mut state)?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    emit_program_terminal_owners(context, &mut state, terminal_owners, true)?;
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn emit_program_terminal_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    owners: &[crate::scalar::CallableProgramTerminalOwner],
    project: bool,
) -> Result<(), String> {
    if owners.is_empty() {
        return Ok(());
    }
    let mut released = Vec::new();
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    for owner in owners {
        if released.contains(&(owner.source.clone(), owner.holder.clone())) {
            continue;
        }
        if owner.global.source != owner.source || owner.environment.literal.source != owner.source {
            return Err(format!(
                "program terminal owner has disconnected source {:?}",
                owner.global
            ));
        }
        if owner.module_relation.environment != owner.environment
            || !owner.module_relation.holders.contains(&owner.holder)
            || !matches!(&owner.action,
                crate::scalar::CallableHolderAction::Drop { environment, holder, span }
                if environment == &owner.environment && holder == &owner.holder && span.source == owner.source)
        {
            return Err(format!(
                "program terminal owner {:?} has disconnected terminal edge",
                owner.global
            ));
        }
        let projections = match &owner.holder {
            crate::scalar::CallableHolderPlace::Binding(binding) if binding == &owner.global => {
                &[][..]
            }
            crate::scalar::CallableHolderPlace::Projection(place)
                if place.binding == owner.global =>
            {
                &place.projections[..]
            }
            _ => {
                return Err(format!(
                    "program terminal owner has disconnected holder {:?}",
                    owner.holder
                ))
            }
        };
        let global_name = if project {
            project_global_name(&owner.source, &owner.global_name)
        } else {
            owner.global_name.clone()
        };
        let (global, ty) = state.all_globals.get(&global_name).ok_or_else(|| {
            format!(
                "program terminal owner {:?} lacks source-qualified LLVM global",
                owner.global
            )
        })?;
        if !crate::scalar::scalar_type_equal(ty, &owner.ty) {
            return Err(format!(
                "program terminal owner {:?} has different LLVM global type",
                owner.global
            ));
        }
        let mut address = global.as_pointer_value();
        let mut current = ty.clone();
        for projection in projections {
            match projection {
                crate::scalar::ReferencePlaceProjection::Field(
                    crate::scalar::ScalarFieldReference::Resolved(field),
                ) => {
                    if !matches!(&current, ScalarType::Struct(id) if id == &field.structure) {
                        return Err(format!(
                            "program terminal owner {:?} has unrelated field",
                            owner.global
                        ));
                    }
                    let declared = memory::structure(state.structs, field.structure.clone())?
                        .fields
                        .get(field.index)
                        .filter(|candidate| candidate.id == *field)
                        .ok_or_else(|| {
                            format!(
                                "program terminal owner {:?} has unknown field",
                                owner.global
                            )
                        })?;
                    let offset = declared.offset.ok_or_else(|| {
                        format!(
                            "program terminal owner {:?} lacks field layout",
                            owner.global
                        )
                    })?;
                    address = unsafe {
                        state.builder.build_in_bounds_gep(
                            context.i8_type(),
                            address,
                            &[context.i64_type().const_int(offset, false)],
                            "terminal_holder_field",
                        )
                    }
                    .map_err(builder_error)?;
                    current = declared.ty.clone();
                }
                crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer {
                    value,
                    ..
                }) => {
                    let index = u64::try_from(value).map_err(|_| {
                        format!(
                            "program terminal owner {:?} has invalid index",
                            owner.global
                        )
                    })?;
                    let ScalarType::Array {
                        element, length, ..
                    } = &current
                    else {
                        return Err(format!(
                            "program terminal owner {:?} requires a fixed-array projection",
                            owner.global
                        ));
                    };
                    if index >= *length {
                        return Err(format!(
                            "program terminal owner {:?} has out-of-bounds index",
                            owner.global
                        ));
                    }
                    address = unsafe {
                        state.builder.build_in_bounds_gep(
                            storage_type(context, &current, state.structs, state.target_layout)?,
                            address,
                            &[
                                context.i32_type().const_zero(),
                                context.i64_type().const_int(index, false),
                            ],
                            "terminal_holder_element",
                        )
                    }
                    .map_err(builder_error)?;
                    current = *element.clone();
                }
                _ => {
                    return Err(format!(
                        "program terminal owner {:?} has unsupported projection",
                        owner.global
                    ))
                }
            }
        }
        if !matches!(current, ScalarType::Callable { .. }) {
            return Err(format!(
                "program terminal owner {:?} does not select a callable",
                owner.global
            ));
        }
        let callable = state
            .builder
            .build_load(
                storage_type(context, &current, state.structs, state.target_layout)?,
                address,
                "terminal_holder",
            )
            .map_err(builder_error)?
            .into_struct_value();
        let environment = state
            .builder
            .build_extract_value(callable, 1, "terminal_holder_environment")
            .map_err(builder_error)?
            .into_pointer_value();
        ownership::drop_environment(context, state, environment)?;
        released.push((owner.source.clone(), owner.holder.clone()));
    }
    let remainder = std::mem::take(&mut state.top_level_remainder_cells);
    for (binding, element) in &remainder {
        let cell = state
            .capture_cells
            .get(binding)
            .ok_or_else(|| format!("terminal remainder cell {binding:?} lacks emitted storage"))?;
        ownership::drop_capture_cell(
            context,
            state,
            *cell,
            ownership::CaptureCellResidence::Promoted,
            element,
        )?;
    }
    ownership::end_graph_mutation(&state.builder, state.module)
}

fn initialize_project_module<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    module: &'module ScalarModule,
    modules: &[&'module ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    initialized: &mut Vec<wosy_syntax::SourceIdentity>,
) -> Result<(), String> {
    if initialized.contains(&module.source) {
        return Ok(());
    }
    initialized.push(module.source.clone());

    let previous_source = std::mem::replace(&mut state.source, module.source.clone());
    let previous_calls = std::mem::replace(
        &mut state.resolved_calls,
        resolved_cfg_calls(&module.top_level_reference_cfg),
    );
    let previous_backing = std::mem::replace(
        &mut state.backing,
        validated_backing_maps(&module.top_level_reference_cfg),
    );
    let previous_holder_events = std::mem::replace(
        &mut state.holder_events,
        holder_events::HolderEvents::new(&module.top_level_reference_cfg)?,
    );

    let values = state.values.clone();
    let storage = state.storage.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
    let backing_values = state.backing_values.clone();
    let backing_slots = state.backing_slots.clone();
    let imported_cell_slots = state.imported_cell_slots.clone();
    let backing_captured = state.backing_captured.clone();
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
    let checked_owner_flags = state.checked_owner_flags.clone();
    let current_module_globals = module_globals(module, globals);
    let current_globals = std::mem::replace(&mut state.globals, current_module_globals);
    state.values.clear();
    state.storage.clear();
    state.runtime_array_owners.clear();
    state.runtime_array_allocations.clear();
    state.runtime_array_lengths.clear();
    state.runtime_array_live.clear();
    state.backing_values.clear();
    state.backing_slots.clear();
    state.imported_cell_slots.clear();
    state.backing_captured.clear();
    state.checked_owner_flags.clear();
    state.automatic_return_result_owners.clear();
    insert_unit_values(&mut state.values, &module.items);

    for item in &module.items {
        match item {
            ScalarItem::Namespace(namespace) => {
                let binding = module
                    .namespace_bindings
                    .iter()
                    .find(|binding| binding.binding == namespace.binding)
                    .ok_or_else(|| format!("unknown namespace binding {}", namespace.binding))?;
                let target = modules
                    .iter()
                    .find(|candidate| candidate.source == binding.target)
                    .ok_or_else(|| format!("missing namespace target {}", binding.target.path))?;
                initialize_project_module(context, state, target, modules, globals, initialized)?;
                state.globals = module_globals(module, globals);
            }
            ScalarItem::Binding(binding) => {
                let value = emit_project_binding_value(context, state, binding, module, modules)?;
                if binding.is_allocation {
                    let receiver = binding
                        .receivers
                        .first()
                        .ok_or_else(|| "runtime allocation has no receiver".to_owned())?;
                    let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
                        return Err("allocation binding requires a runtime array type".to_owned());
                    };
                    let length = receiver
                        .allocation_length
                        .as_ref()
                        .ok_or_else(|| "runtime allocation has no length".to_owned())?;
                    let length = take_basic(emit_project_typed_expression(
                        context,
                        state,
                        length,
                        &ScalarType::U64,
                        module,
                        modules,
                    )?)?
                    .into_int_value();
                    let pointer = emit_runtime_array_allocation(context, state, element, length)?;
                    let (global, _) = state
                        .all_globals
                        .get(&project_global_name(&module.source, &receiver.name))
                        .cloned()
                        .ok_or_else(|| format!("unknown LLVM global {}", receiver.name))?;
                    let length_slot = entry_alloca(
                        state,
                        context.i64_type().into(),
                        &format!("{}_length", receiver.name),
                    )?;
                    state
                        .builder
                        .build_store(length_slot, length)
                        .map_err(builder_error)?;
                    ownership::begin_graph_mutation(&state.builder, state.module)?;
                    state
                        .builder
                        .build_store(global.as_pointer_value(), pointer)
                        .map_err(builder_error)?;
                    ownership::end_graph_mutation(&state.builder, state.module)?;
                    state.storage.insert(
                        receiver.name.clone(),
                        (global.as_pointer_value(), receiver.ty.clone()),
                    );
                    state
                        .runtime_array_lengths
                        .insert(receiver.name.clone(), length_slot);
                    state
                        .runtime_array_owners
                        .insert(receiver.name.clone(), true);
                    state
                        .runtime_array_allocations
                        .insert(receiver.name.clone(), false);
                    record_backing_point_values(
                        state,
                        &receiver.name,
                        backing_transport_span(receiver.name_span),
                    );
                    continue;
                }
                store_global_binding_outputs(context, state, binding, value)?;
            }
            ScalarItem::Executable(item) => {
                emit_project_block_item(context, state, item, module, modules)?;
            }
            ScalarItem::Extern(_) | ScalarItem::Function(_) => {}
        }
    }

    state.globals = current_globals;
    state.values = values;
    let mut retained_storage = storage;
    retained_storage.extend(
        std::mem::take(&mut state.storage)
            .into_iter()
            .map(|(name, storage)| (project_runtime_array_name(&module.source, &name), storage)),
    );
    state.storage = retained_storage;
    let mut retained_owners = runtime_array_owners;
    retained_owners.extend(
        std::mem::take(&mut state.runtime_array_owners)
            .into_iter()
            .map(|(name, owner)| (project_runtime_array_name(&module.source, &name), owner)),
    );
    state.runtime_array_owners = retained_owners;
    let mut retained_allocations = runtime_array_allocations;
    retained_allocations.extend(
        std::mem::take(&mut state.runtime_array_allocations)
            .into_iter()
            .map(|(name, allocation)| {
                (
                    project_runtime_array_name(&module.source, &name),
                    allocation,
                )
            }),
    );
    state.runtime_array_allocations = retained_allocations;
    let mut retained_lengths = runtime_array_lengths;
    retained_lengths.extend(
        std::mem::take(&mut state.runtime_array_lengths)
            .into_iter()
            .map(|(name, slot)| (project_runtime_array_name(&module.source, &name), slot)),
    );
    state.runtime_array_lengths = retained_lengths;
    let mut retained_live = runtime_array_live;
    retained_live.extend(
        std::mem::take(&mut state.runtime_array_live)
            .into_iter()
            .map(|(name, slot)| (project_runtime_array_name(&module.source, &name), slot)),
    );
    state.runtime_array_live = retained_live;
    let mut retained_values = backing_values;
    retained_values.extend(
        std::mem::take(&mut state.backing_values)
            .into_iter()
            .map(|(name, values)| (project_runtime_array_name(&module.source, &name), values)),
    );
    state.backing_values = retained_values;
    let mut retained_slots = backing_slots;
    retained_slots.extend(
        std::mem::take(&mut state.backing_slots)
            .into_iter()
            .map(|(name, slot)| (project_runtime_array_name(&module.source, &name), slot)),
    );
    state.backing_slots = retained_slots;
    let mut retained_imported_cells = imported_cell_slots;
    retained_imported_cells.extend(
        std::mem::take(&mut state.imported_cell_slots)
            .into_iter()
            .map(|(name, slot)| (project_runtime_array_name(&module.source, &name), slot)),
    );
    state.imported_cell_slots = retained_imported_cells;
    let mut retained_captured = backing_captured;
    retained_captured.extend(
        std::mem::take(&mut state.backing_captured)
            .into_iter()
            .map(|name| project_runtime_array_name(&module.source, &name)),
    );
    state.backing_captured = retained_captured;
    let mut retained_return_results = automatic_return_result_owners;
    retained_return_results.extend(std::mem::take(&mut state.automatic_return_result_owners));
    state.automatic_return_result_owners = retained_return_results;
    state.checked_owner_flags = checked_owner_flags;
    state.source = previous_source;
    state.resolved_calls = previous_calls;
    state.backing = previous_backing;
    state.holder_events = previous_holder_events;
    Ok(())
}

fn merge_scope_map<V: Clone>(
    outer: BTreeMap<String, V>,
    current: &BTreeMap<String, V>,
) -> BTreeMap<String, V> {
    let mut merged = outer
        .into_iter()
        .map(|(name, value)| {
            let updated = current.get(&name).cloned().unwrap_or(value);
            (name, updated)
        })
        .collect::<BTreeMap<_, _>>();
    for (name, value) in current {
        if !merged.contains_key(name) {
            merged.insert(name.clone(), value.clone());
        }
    }
    merged
}

fn outer_scope_names(state: &EmitState<'_, '_>) -> BTreeSet<String> {
    state
        .storage
        .keys()
        .chain(state.globals.keys())
        .cloned()
        .collect()
}

fn validate_return_holder_events(
    state: &EmitState<'_, '_>,
    block: &ScalarBlock,
) -> Result<(), String> {
    for output in &block.final_output_values {
        let point =
            state
                .holder_events
                .return_output(&state.source, output.span, output.position)?;
        state.holder_events.validated(point)?;
    }
    Ok(())
}

fn selected_callable_array<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    holder: &crate::scalar::CallableArraySelectedHolderFact,
) -> Result<
    (
        PointerValue<'ctx>,
        inkwell::values::IntValue<'ctx>,
        PointerValue<'ctx>,
    ),
    String,
> {
    let data = local_callable_array_data(context, state, &holder.place.binding)?;
    let selected = state
        .holder_events
        .selected
        .iter()
        .filter(|(_, slots)| slots.index_point == holder.index_point)
        .collect::<Vec<_>>();
    let [(point, _)] = selected.as_slice() else {
        return Err(format!(
            "selected callable backing at {:?} lacks unique cached selection",
            holder.occurrence
        ));
    };
    let (index, address, present) =
        memory::selected_element(context, state, **point, holder.index_point)?;
    emit_checked_dereference_guard(context, state, present)?;
    Ok((data, index, address))
}

fn local_callable_array_data<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    binding: &crate::scalar::ReferenceBindingId,
) -> Result<PointerValue<'ctx>, String> {
    if let Some(cell) = state.capture_cells.get(binding) {
        if !counted_callable_capture_binding(state, binding)
            && !state.counted_fixed_cells.contains(binding)
        {
            return Err(format!(
                "unsupported captured callable backing {binding:?} lacks source-qualified counted descriptor for connected cell trace/dispose and cycle collection"
            ));
        }
        let captures = state.storage.iter().filter(|(_, (address, ty))| {
            address == cell
                && (matches!(ty, ScalarType::RuntimeArray { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
                    || matches!(ty, ScalarType::Array { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. })))
        }).collect::<Vec<_>>();
        let [(_, _)] = captures.as_slice() else {
            return Err(format!(
                "captured callable backing {binding:?} lacks unique typed cell storage"
            ));
        };
        return Ok(state
            .builder
            .build_load(
                context.ptr_type(AddressSpace::default()),
                *cell,
                "selected_capture_array_data",
            )
            .map_err(builder_error)?
            .into_pointer_value());
    }
    let slot = state
        .holder_events
        .bindings
        .get(binding)
        .ok_or_else(|| format!("local callable backing {binding:?} lacks bound storage"))?;
    let names = state
        .storage
        .iter()
        .filter(|(_, (address, ty))| {
            address == slot && matches!(ty, ScalarType::RuntimeArray { .. })
        })
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    let [name] = names.as_slice() else {
        return Err(format!(
            "local callable backing {binding:?} lacks unique runtime array"
        ));
    };
    let data = if state.runtime_array_allocations.get(*name) == Some(&true) {
        *slot
    } else {
        state
            .builder
            .build_load(
                context.ptr_type(AddressSpace::default()),
                *slot,
                "selected_array_data",
            )
            .map_err(builder_error)?
            .into_pointer_value()
    };
    Ok(data)
}

fn counted_callable_capture_binding(
    state: &EmitState<'_, '_>,
    binding: &crate::scalar::ReferenceBindingId,
) -> bool {
    state.literal_captures.iter().any(|(literal, producer)| {
        state.covered_literals.contains(literal)
            && producer.descriptor.get().is_some_and(|descriptor| {
                descriptor.cells.iter().any(|cell| cell.binding == *binding)
            })
    })
}

fn counted_callable_capture_backing(
    state: &EmitState<'_, '_>,
    backing: &crate::scalar::RuntimeBackingFact,
) -> bool {
    matches!(&backing.callable_family.as_slice(), [family] if matches!(family.element, ScalarType::Callable { .. }))
        && state.capture_cells.contains_key(&backing.owner)
        && (counted_callable_capture_binding(state, &backing.owner)
            || state.counted_fixed_cells.contains(&backing.owner))
        && (state.literal_captures.iter().any(|(literal, producer)| {
            state.covered_literals.contains(literal)
                && producer.descriptor.get().is_some_and(|descriptor| {
                    descriptor.cells.iter().any(|cell| {
                        cell.binding == backing.owner && cell.backings.contains(backing)
                    })
                })
        }) || state.counted_fixed_cells.contains(&backing.owner))
}

fn emit_connected_holder_actions<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    edge: holder_events::HolderEmissionEdge,
    before_assignment: Option<bool>,
) -> Result<(), String> {
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    emit_connected_holder_actions_locked(context, state, edge, before_assignment)?;
    ownership::end_graph_mutation(&state.builder, state.module)
}

fn emit_connected_holder_actions_locked<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    edge: holder_events::HolderEmissionEdge,
    before_assignment: Option<bool>,
) -> Result<(), String> {
    let actions = state.holder_events.connected_actions(edge)?;
    let mut selected_drop_emitted = false;
    let mut selected_install_emitted = false;
    let mut selected_read_retained = false;
    let mut dropped_holders = Vec::new();
    let mut retained_holders = Vec::new();
    let mut retained_fixed_copies = Vec::new();
    for action in &actions {
        if before_assignment == Some(true)
            && !matches!(
                action,
                crate::scalar::CallableHolderAction::Drop { .. }
                    | crate::scalar::CallableHolderAction::DropSelected { .. }
                    | crate::scalar::CallableHolderAction::SkipUninitializedSelected { .. }
                    | crate::scalar::CallableHolderAction::ReplaceCaptureCellValue { .. }
            )
        {
            continue;
        }
        if before_assignment == Some(false)
            && (matches!(
                action,
                crate::scalar::CallableHolderAction::Drop { .. }
                    | crate::scalar::CallableHolderAction::DropSelected { .. }
                    | crate::scalar::CallableHolderAction::SkipUninitializedSelected { .. }
                    | crate::scalar::CallableHolderAction::ReplaceCaptureCellValue { .. }
            ) || (matches!(
                action,
                crate::scalar::CallableHolderAction::DropArrayElements { .. }
            ) && matches!(
                state.holder_events.cfg_point(edge.from())?.kind,
                CfgPointKind::Backing {
                    transfer: RuntimeBackingTransfer::CopyFixed { .. },
                    ..
                }
            )))
        {
            continue;
        }
        match action {
            crate::scalar::CallableHolderAction::DropSelected { holder, old, .. } => {
                state.holder_events.action(edge, action)?;
                let crate::scalar::CallableCellCaseFact::Initialized { .. } = old else {
                    return Err(format!(
                        "selected drop at {:?} has no initialized cell",
                        holder.occurrence
                    ));
                };
                if !selected_drop_emitted {
                    let (data, index, address) = selected_callable_array(context, state, holder)?;
                    let installed =
                        ownership::callable_array_element_installed(context, state, data, index)?;
                    let function = state
                        .builder
                        .get_insert_block()
                        .expect("selected drop block")
                        .get_parent()
                        .expect("selected drop function");
                    let drop = context.append_basic_block(function, "selected.drop.installed");
                    let next = context.append_basic_block(function, "selected.drop.next");
                    state
                        .builder
                        .build_conditional_branch(installed, drop, next)
                        .map_err(builder_error)?;
                    state.builder.position_at_end(drop);
                    let callable = state
                        .builder
                        .build_load(
                            context.struct_type(
                                &[
                                    context.ptr_type(AddressSpace::default()).into(),
                                    context.ptr_type(AddressSpace::default()).into(),
                                ],
                                false,
                            ),
                            address,
                            "selected_old_callable",
                        )
                        .map_err(builder_error)?
                        .into_struct_value();
                    let environment = state
                        .builder
                        .build_extract_value(callable, 1, "selected_old_environment")
                        .map_err(builder_error)?
                        .into_pointer_value();
                    let literals = actions
                        .iter()
                        .filter_map(|candidate| match candidate {
                            crate::scalar::CallableHolderAction::DropSelected {
                                holder: other,
                                old:
                                    crate::scalar::CallableCellCaseFact::Initialized {
                                        environment_origin,
                                        ..
                                    },
                                ..
                            } if other.value == holder.value
                                && other.index_point == holder.index_point =>
                            {
                                Some(environment_origin.literal.clone())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    for literal in &literals {
                        if !state.literal_captures.contains_key(literal) {
                            return Err(format!(
                                "source-qualified selected callable lacks producer at {literal:?}"
                            ));
                        }
                    }
                    ownership::drop_environment(context, state, environment)?;
                    state
                        .builder
                        .build_unconditional_branch(next)
                        .map_err(builder_error)?;
                    state.builder.position_at_end(next);
                    selected_drop_emitted = true;
                }
            }
            crate::scalar::CallableHolderAction::RetainCheckedSelected { holder, to, .. } => {
                state.holder_events.action(edge, action)?;
                let crate::scalar::CallableHolderPlace::Temporary(read) = to else {
                    return Err(format!(
                        "indexed read at {:?} lacks a source-qualified temporary",
                        holder.occurrence
                    ));
                };
                if *read != holder.index_point || *read != edge.from() {
                    return Err(format!(
                        "indexed read at {:?} has disconnected cached storage",
                        holder.occurrence
                    ));
                }
                if !selected_read_retained {
                    let (callable, address, present) =
                        memory::callable_read(context, state, *read)?;
                    emit_checked_dereference_guard(context, state, present)?;
                    let data = local_callable_array_data(context, state, &holder.place.binding)?;
                    let pointer_type = pointer_integer_type(context, state.target_layout);
                    let current = state
                        .builder
                        .build_ptr_to_int(address, pointer_type, "read_element_pointer")
                        .map_err(builder_error)?;
                    let origin = state
                        .builder
                        .build_ptr_to_int(data, pointer_type, "read_array_pointer")
                        .map_err(builder_error)?;
                    let offset = state
                        .builder
                        .build_int_sub(current, origin, "read_element_offset")
                        .map_err(builder_error)?;
                    let wide = state
                        .builder
                        .build_int_z_extend_or_bit_cast(
                            offset,
                            context.i64_type(),
                            "read_element_wide_offset",
                        )
                        .map_err(builder_error)?;
                    let index = state
                        .builder
                        .build_int_unsigned_div(
                            wide,
                            context
                                .i64_type()
                                .const_int(state.target_layout.pointer_size * 2, false),
                            "read_element_index",
                        )
                        .map_err(builder_error)?;
                    let installed =
                        ownership::callable_array_element_installed(context, state, data, index)?;
                    let function = state
                        .builder
                        .get_insert_block()
                        .expect("read retain block")
                        .get_parent()
                        .expect("read retain function");
                    let retain = context.append_basic_block(function, "indexed_read.retain");
                    let done = context.append_basic_block(function, "indexed_read.done");
                    state
                        .builder
                        .build_conditional_branch(installed, retain, done)
                        .map_err(builder_error)?;
                    state.builder.position_at_end(retain);
                    let environment = state
                        .builder
                        .build_extract_value(callable, 1, "read_holder_environment")
                        .map_err(builder_error)?
                        .into_pointer_value();
                    ownership::retain_environment(context, state, environment)?;
                    state
                        .builder
                        .build_unconditional_branch(done)
                        .map_err(builder_error)?;
                    state.builder.position_at_end(done);
                    selected_read_retained = true;
                }
            }
            crate::scalar::CallableHolderAction::SkipUninitializedSelected { .. } => {
                state.holder_events.action(edge, action)?;
            }
            crate::scalar::CallableHolderAction::MoveSelected { to, .. }
            | crate::scalar::CallableHolderAction::RetainSelected { to, .. } => {
                state.holder_events.action(edge, action)?;
                if let crate::scalar::CallableHolderAction::RetainSelected { .. } = action {
                    let (_, _, address) = selected_callable_array(context, state, to)?;
                    let callable = state
                        .builder
                        .build_load(
                            context.struct_type(
                                &[
                                    context.ptr_type(AddressSpace::default()).into(),
                                    context.ptr_type(AddressSpace::default()).into(),
                                ],
                                false,
                            ),
                            address,
                            "selected_incoming_callable",
                        )
                        .map_err(builder_error)?
                        .into_struct_value();
                    let environment = state
                        .builder
                        .build_extract_value(callable, 1, "selected_incoming_environment")
                        .map_err(builder_error)?
                        .into_pointer_value();
                    ownership::retain_environment(context, state, environment)?;
                }
                if !selected_install_emitted {
                    let (data, index, _) = selected_callable_array(context, state, to)?;
                    ownership::install_callable_array_element(context, state, data, index)?;
                    selected_install_emitted = true;
                }
            }
            crate::scalar::CallableHolderAction::DropArrayElements { backing, .. } => {
                state.holder_events.action(edge, action)?;
                if counted_callable_capture_backing(state, backing)
                    && !state.holder_events.bindings.contains_key(&backing.owner)
                {
                    continue;
                }
                if counted_callable_capture_backing(state, backing) {
                    continue;
                }
                let names = state
                    .storage
                    .iter()
                    .filter(|(name, (slot, ty))| {
                        !state.backing_captured.contains(*name)
                            && matches!(ty, ScalarType::RuntimeArray { .. })
                            && state.holder_events.bindings.get(&backing.owner) == Some(slot)
                    })
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>();
                let [name] = names.as_slice() else {
                    return Err(format!("callable array cleanup at {:?} lacks unique local backing for {:?}: {names:?}", backing.origin, backing.value));
                };
                release_backing_entry_locked(context, state, name)?;
            }
            crate::scalar::CallableHolderAction::DropCapturedValueCellOwner {
                binding,
                cell,
                ty,
                ..
            } => {
                state.holder_events.action(edge, action)?;
                if cell.source != binding.source || cell.range != binding.declaration_span {
                    return Err(format!(
                        "captured value cell at {cell:?} lacks its source-qualified binding"
                    ));
                }
                let address = *state.capture_cells.get(binding).ok_or_else(|| {
                    format!("captured value cell at {cell:?} lacks emitted storage")
                })?;
                ownership::drop_capture_cell(
                    context,
                    state,
                    address,
                    ownership::CaptureCellResidence::Promoted,
                    ty,
                )?;
            }
            crate::scalar::CallableHolderAction::Retain {
                environment,
                from,
                to,
                ..
            } => {
                if !state.holder_events.counted_environment(environment)
                    && !state.cyclic_literals.contains(&environment.literal)
                {
                    continue;
                }
                state
                    .holder_events
                    .counted_action(edge, action, environment)?;
                if let crate::scalar::CallableHolderPlace::Projection(place) = to {
                    if actions.iter().any(|selected| {
                        matches!(selected,
                        crate::scalar::CallableHolderAction::RetainSelected {
                            environment: installed, from: source, to: holder, ..
                        } if installed == environment && source == from && holder.place == *place)
                    }) {
                        continue;
                    }
                }
                if retained_holders.contains(to) {
                    continue;
                }
                retained_holders.push(to.clone());
                let memory::ConcreteOwnerAction::Holder(holder) =
                    memory::concrete_owner_action(context, state, edge, action)
                        .map_err(|error| format!("{action:?} at {edge:?}: {error}"))?
                else {
                    return Err(format!(
                        "connected retain lacks callable storage at {edge:?}"
                    ));
                };
                ownership::retain_environment(context, state, holder.environment)?;
            }
            crate::scalar::CallableHolderAction::Drop {
                environment,
                holder: source,
                ..
            } => {
                if !state.holder_events.counted_environment(environment)
                    && !state.cyclic_literals.contains(&environment.literal)
                {
                    continue;
                }
                if let crate::scalar::CallableHolderPlace::Projection(place) = source {
                    if state.holder_events.covered_cells.get(&place.binding).is_some_and(|coverage| {
                        coverage.environments.iter().all(|candidate| state.covered_literals.contains(&candidate.literal))
                            && matches!(&coverage.backing.callable_family.as_slice(), [family] if matches!(family.element, ScalarType::Callable { .. }))
                    }) && state.capture_cells.contains_key(&place.binding)
                        && state.holder_events.scope_exit_edge(edge)
                    {
                        state.holder_events.action(edge, action)?;
                        continue;
                    }
                    if state.counted_fixed_cells.contains(&place.binding)
                        && state.capture_cells.contains_key(&place.binding)
                        && state.holder_events.scope_exit_edge(edge)
                    {
                        state.holder_events.action(edge, action)?;
                        continue;
                    }
                    if place.projections.iter().any(|projection| {
                        matches!(
                            projection,
                            crate::scalar::ReferencePlaceProjection::Index(_)
                        )
                    }) && (actions.iter().any(|candidate| {
                        matches!(candidate,
                            crate::scalar::CallableHolderAction::DropArrayElements { backing, .. }
                            if backing.owner == place.binding)
                    }) || state.storage.iter().any(|(name, (slot, ty))| {
                        state.holder_events.bindings.get(&place.binding) == Some(slot)
                            && matches!(ty, ScalarType::RuntimeArray { .. })
                            && state.runtime_array_owners.get(name) == Some(&false)
                    })) {
                        state.holder_events.action(edge, action)?;
                        continue;
                    }
                }
                if matches!(source, crate::scalar::CallableHolderPlace::Temporary(_))
                    && state.holder_events.moved_to_selected(environment, source)
                {
                    state.holder_events.action(edge, action)?;
                    continue;
                }
                state.holder_events.action(edge, action)?;
                state
                    .holder_events
                    .counted_action(edge, action, environment)?;
                if dropped_holders.contains(source) {
                    continue;
                }
                dropped_holders.push(source.clone());
                let memory::ConcreteOwnerAction::Holder(holder) =
                    memory::concrete_owner_action(context, state, edge, action)
                        .map_err(|error| format!("{action:?} at {edge:?}: {error}"))?
                else {
                    return Err(format!("connected drop lacks callable storage at {edge:?}"));
                };
                ownership::drop_environment(context, state, holder.environment)?;
            }
            crate::scalar::CallableHolderAction::DropCaptureCellOwner {
                binding, backing, ..
            } => {
                if !state.holder_events.covered_cells.contains_key(binding)
                    && !state.counted_fixed_cells.contains(binding)
                    && !state
                        .top_level_remainder_cells
                        .iter()
                        .any(|(known, _)| known == binding)
                {
                    continue;
                }
                let element = if let Some(coverage) = state.holder_events.covered_cells.get(binding)
                {
                    if !coverage.owner_edges.contains(&edge) {
                        return Err(format!(
                            "capture cell {binding:?} has a disconnected release owner at {edge:?}"
                        ));
                    }
                    match coverage.backing.callable_family.as_slice() {
                        [] => ScalarType::U8,
                        [family] => family.element.clone(),
                        _ => {
                            return Err(format!(
                                "capture cell {binding:?} has conflicting element types"
                            ))
                        }
                    }
                } else {
                    match backing.callable_family.as_slice() {
                        [] => ScalarType::U8,
                        [family] => {
                            if !matches!(family.element, ScalarType::Callable { .. }) {
                                return Err(format!(
                                    "fixed capture cell {binding:?} has non-callable element type"
                                ));
                            }
                            family.element.clone()
                        }
                        _ => {
                            return Err(format!(
                                "fixed capture cell {binding:?} lacks one callable element family"
                            ));
                        }
                    }
                };
                let memory::ConcreteOwnerAction::CaptureCell(cell) =
                    memory::concrete_owner_action(context, state, edge, action)?
                else {
                    return Err(format!(
                        "connected cell exit lacks emitted cell at {edge:?}"
                    ));
                };
                ownership::drop_capture_cell(
                    context,
                    state,
                    cell,
                    ownership::CaptureCellResidence::Promoted,
                    &element,
                )?;
                if let Some(position) = state
                    .top_level_remainder_cells
                    .iter()
                    .position(|(known, _)| known == binding)
                {
                    state.top_level_remainder_cells.remove(position);
                }
            }
            crate::scalar::CallableHolderAction::Move { environment, .. }
            | crate::scalar::CallableHolderAction::ReturnTransfer { environment, .. } => {
                if state.holder_events.counted_environment(environment)
                    || state.cyclic_literals.contains(&environment.literal)
                {
                    state.holder_events.action(edge, action)?;
                    state
                        .holder_events
                        .counted_action(edge, action, environment)?;
                    for coverage in state.holder_events.covered_cells.values() {
                        if coverage.environments.contains(environment)
                            && !coverage.actions.contains(&(edge, action.clone()))
                        {
                            return Err(format!(
                                "capture cell {:?} lacks connected transfer {action:?} at {edge:?}",
                                coverage.backing.value
                            ));
                        }
                    }
                    let to = match action {
                        crate::scalar::CallableHolderAction::Move { to, .. }
                        | crate::scalar::CallableHolderAction::ReturnTransfer { to, .. } => to,
                        _ => unreachable!(),
                    };
                    let _holder = memory::concrete_callable_holder(context, state, edge, to)?;
                    if let Some(cell) = cyclic_cell_install(state, environment, to) {
                        let installed = state
                            .builder
                            .build_load(
                                context.struct_type(
                                    &[
                                        context.ptr_type(AddressSpace::default()).into(),
                                        context.ptr_type(AddressSpace::default()).into(),
                                    ],
                                    false,
                                ),
                                cell,
                                "cyclic_install_holder",
                            )
                            .map_err(builder_error)?
                            .into_struct_value();
                        let environment = state
                            .builder
                            .build_extract_value(installed, 1, "cyclic_install_environment")
                            .map_err(builder_error)?
                            .into_pointer_value();
                        ownership::retain_environment(context, state, environment)?;
                    }
                }
            }
            crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                binding,
                previous,
                ..
            } if state.holder_events.covered_cells.contains_key(binding) => {
                let memory::ConcreteOwnerAction::CaptureCell(cell) =
                    memory::concrete_owner_action(context, state, edge, action)?
                else {
                    return Err(format!(
                        "connected cell replacement lacks emitted cell at {edge:?}"
                    ));
                };
                let element = match previous.callable_family.as_slice() {
                    [] => ScalarType::U8,
                    [family] => family.element.clone(),
                    _ => {
                        return Err(format!(
                            "capture cell {binding:?} has conflicting element types"
                        ))
                    }
                };
                ownership::replace_capture_cell_backing(context, state, cell, &element)?;
            }
            crate::scalar::CallableHolderAction::SelectElement { .. } => {}
            crate::scalar::CallableHolderAction::RetainArrayElements {
                source, backing, ..
            } => {
                state.holder_events.action(edge, action)?;
                let CfgPointKind::Backing {
                    target,
                    transfer: RuntimeBackingTransfer::CopyFixed { source: copied },
                    ..
                } = &state.holder_events.cfg_point(edge.from())?.kind
                else {
                    return Err(format!(
                        "fixed-copy retain lacks source-qualified Backing at {edge:?}"
                    ));
                };
                if &source.owner != copied
                    || &backing.owner != target
                    || source.callable_family.len() != backing.callable_family.len()
                {
                    return Err(format!(
                        "fixed-copy retain has unrelated source or destination at {edge:?}"
                    ));
                }
                let [family] = backing.callable_family.as_slice() else {
                    return Err(format!(
                        "fixed-copy retain lacks one callable family at {edge:?}"
                    ));
                };
                if !matches!(family.element, ScalarType::Callable { .. }) {
                    return Err(format!(
                        "fixed-copy retain has non-callable element at {edge:?}"
                    ));
                }
                let [source_family] = source.callable_family.as_slice() else {
                    return Err(format!(
                        "fixed-copy retain lacks one source callable family at {edge:?}"
                    ));
                };
                if source_family.element != family.element {
                    return Err(format!(
                        "fixed-copy retain changes callable element type at {edge:?}"
                    ));
                }
                let crate::scalar::RuntimeArrayLengthFact::Fixed { value: length, .. } =
                    backing.length
                else {
                    return Err(format!("fixed-copy retain lacks fixed length at {edge:?}"));
                };
                if retained_fixed_copies
                    .iter()
                    .any(|(owner, _, _)| owner == &backing.owner)
                {
                    if !retained_fixed_copies.contains(&(
                        backing.owner.clone(),
                        source.value.clone(),
                        length,
                    )) {
                        return Err(format!(
                            "fixed-copy retain has ambiguous backing alternatives at {edge:?}"
                        ));
                    }
                    continue;
                }
                retained_fixed_copies.push((backing.owner.clone(), source.value.clone(), length));
                for index in 0..length {
                    let candidates = source_family.elements.iter().filter(|element| matches!(
                        &element.index,
                        crate::scalar::CallableArrayIndexFact::Constant(value) if *value == index.into()
                    )).collect::<Vec<_>>();
                    let [element] = candidates.as_slice() else {
                        return Err(format!("fixed-copy retain lacks one source-qualified initialized element {index} at {edge:?}"));
                    };
                    if !state
                        .literal_captures
                        .contains_key(&element.environment.literal)
                    {
                        return Err(format!(
                            "fixed-copy retain lacks producer for element {index} at {:?}",
                            element.environment.literal
                        ));
                    }
                }
                let slot = *state.holder_events.bindings.get(target).ok_or_else(|| {
                    format!("fixed-copy retain lacks published destination {target:?} at {edge:?}")
                })?;
                let data = state
                    .builder
                    .build_load(
                        context.ptr_type(AddressSpace::default()),
                        slot,
                        "fixed_copy_destination",
                    )
                    .map_err(builder_error)?
                    .into_pointer_value();
                ownership::retain_fixed_copy_elements(context, state, data, length)?;
            }
            crate::scalar::CallableHolderAction::ReplaceCaptureCellValue { .. } => {
                return Err(format!(
                    "unsupported connected holder action {action:?} at {:?}",
                    state.holder_events.source_span(edge)?
                ));
            }
        }
    }
    Ok(())
}

fn counted_assignment_cell<'ctx>(
    state: &EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    output: usize,
    target: &crate::scalar::ScalarAssignmentTarget,
) -> Result<Option<crate::scalar::ReferenceBindingId>, String> {
    let crate::ScalarPlace::Name { .. } = &target.place else {
        return Ok(None);
    };
    let binding = state.holder_events.assignment_binding(assignment, output)?;
    Ok((state.capture_cells.contains_key(&binding)
        && state.holder_events.covered_cells.contains_key(&binding))
    .then_some(binding))
}

fn prepare_counted_replacement<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    output: usize,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
) -> Result<Option<PointerValue<'ctx>>, String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(None);
    }
    let Some(binding) = counted_assignment_cell(state, assignment, output, target)? else {
        return Ok(None);
    };
    let edges = state
        .holder_events
        .replacement_edges(assignment, &binding)?;
    if edges.len() != 1 {
        return Err(format!(
            "captured cell {binding:?} requires one replacement edge"
        ));
    }
    for edge in edges {
        emit_connected_holder_actions_locked(context, state, edge, Some(true))?;
    }
    Ok(Some(state.capture_cells[&binding]))
}

fn finish_counted_replacement<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    target: &crate::scalar::ScalarAssignmentTarget,
    expression: &ScalarExpression,
) -> Result<(), String> {
    let ScalarExpression::Name { name: source, .. } = expression else {
        return Err(format!(
            "captured cell replacement at {:?} lacks source backing",
            target.target_span
        ));
    };
    let crate::ScalarPlace::Name {
        name: target_name, ..
    } = &target.place
    else {
        return Err(format!(
            "captured cell replacement at {:?} lacks named target",
            target.target_span
        ));
    };
    let source_length = *state.runtime_array_lengths.get(source).ok_or_else(|| {
        format!("captured cell incoming backing `{source}` lacks evaluated length")
    })?;
    let target_length = ownership::capture_cell_length_address(context, state, cell)?;
    let length = state
        .builder
        .build_load(
            context.i64_type(),
            source_length,
            "capture_cell_incoming_length",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(target_length, length)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(
            ownership::capture_cell_owned_address(context, state, cell)?,
            context.i64_type().const_int(1, false),
        )
        .map_err(builder_error)?;
    transfer_backing_ownership(state, source);
    state
        .runtime_array_owners
        .insert(target_name.clone(), false);
    state.backing_values.remove(target_name);
    Ok(())
}

fn emit_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let values = state.values.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
    let backing_values = state.backing_values.clone();
    let backing_slots = state.backing_slots.clone();
    let imported_cell_slots = state.imported_cell_slots.clone();
    let backing_captured = state.backing_captured.clone();
    let outer_names = outer_scope_names(state);
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
    let checked_owner_flags = state.checked_owner_flags.clone();
    let mut result = EmitValue::Unit;
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value = emit_binding_value(context, state, binding)?;
                store_binding_outputs(context, state, binding, value)?;
                EmitValue::Unit
            }
            ScalarBlockItem::Expression(expression) => emit_expression(context, state, expression)?,
            ScalarBlockItem::Assignment(assignment) => emit_assignment(context, state, assignment)?,
            ScalarBlockItem::While(expression) => emit_while(context, state, expression)?,
        };
    }
    if !block.final_output_values.is_empty() {
        result = emit_final_outputs(context, state, block)?;
    }
    if state.return_body_span == Some(block.span) {
        validate_return_holder_events(state, block)?;
        ownership::begin_graph_mutation(&state.builder, state.module)?;
        record_final_return_holders(context, state, block, &result)?;
        for output in &block.final_output_values {
            let point =
                state
                    .holder_events
                    .return_output(&state.source, output.span, output.position)?;
            let edges = point
                .successors
                .iter()
                .map(|to| holder_events::HolderEmissionEdge::Outgoing {
                    from: point.id,
                    to: *to,
                })
                .collect::<Vec<_>>();
            for edge in edges {
                emit_connected_holder_actions_locked(context, state, edge, None)?;
            }
        }
        ownership::end_graph_mutation(&state.builder, state.module)?;
    }
    if state.return_body_span == Some(block.span) {
        result = materialize_return_branch(context, state, block, result)?;
        let outputs = crate::ScalarOutputSequence {
            outputs: block
                .final_output_values
                .iter()
                .map(|output| crate::ScalarOutput {
                    ty: output.ty.clone(),
                    span: output.span,
                })
                .collect(),
            span: block.span,
        };
        result = materialize_fresh_return_outputs(context, state, result, &outputs)?;
        state.fresh_return_outputs.clear();
    } else {
        result = materialize_return_branch(context, state, block, result)?;
    }
    let exit = state.holder_events.block_exit(&block.occurrence)?;
    state.holder_events.validated(exit)?;
    let exit_edges = state.holder_events.scope_action_edges(exit)?;
    for edge in exit_edges {
        emit_connected_holder_actions(context, state, edge, None)?;
    }
    release_scope_runtime_array_owners(context, state, &runtime_array_owners, &outer_names)?;
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    state.storage = storage;
    state.values = values;
    state.runtime_array_owners = merge_scope_map(runtime_array_owners, &state.runtime_array_owners);
    state.runtime_array_allocations =
        merge_scope_map(runtime_array_allocations, &state.runtime_array_allocations);
    state.runtime_array_lengths =
        merge_scope_map(runtime_array_lengths, &state.runtime_array_lengths);
    state.runtime_array_live = merge_scope_map(runtime_array_live, &state.runtime_array_live);
    state.backing_values = merge_scope_map(backing_values, &state.backing_values);
    state.backing_slots = merge_scope_map(backing_slots, &state.backing_slots);
    state.imported_cell_slots = merge_scope_map(imported_cell_slots, &state.imported_cell_slots);
    state.backing_captured = backing_captured
        .into_iter()
        .chain(state.backing_captured.iter().cloned())
        .collect();
    state.automatic_return_result_owners = automatic_return_result_owners;
    state.checked_owner_flags = checked_owner_flags;
    Ok(result)
}

fn emit_project_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let values = state.values.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
    let backing_values = state.backing_values.clone();
    let backing_slots = state.backing_slots.clone();
    let imported_cell_slots = state.imported_cell_slots.clone();
    let backing_captured = state.backing_captured.clone();
    let outer_names = outer_scope_names(state);
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
    let checked_owner_flags = state.checked_owner_flags.clone();
    let mut result = EmitValue::Unit;
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value = emit_project_binding_value(context, state, binding, module, modules)?;
                store_binding_outputs(context, state, binding, value)?;
                EmitValue::Unit
            }
            ScalarBlockItem::Expression(expression) => {
                emit_project_expression(context, state, expression, module, modules)?
            }
            ScalarBlockItem::Assignment(assignment) => {
                emit_project_assignment(context, state, assignment, module, modules)?
            }
            ScalarBlockItem::While(expression) => {
                emit_project_while(context, state, expression, module, modules)?
            }
        };
    }
    if !block.final_output_values.is_empty() {
        result = emit_project_final_outputs(context, state, block, module, modules)?;
    }
    if state.return_body_span == Some(block.span) {
        validate_return_holder_events(state, block)?;
        ownership::begin_graph_mutation(&state.builder, state.module)?;
        record_final_return_holders(context, state, block, &result)?;
        for output in &block.final_output_values {
            let point =
                state
                    .holder_events
                    .return_output(&state.source, output.span, output.position)?;
            let edges = point
                .successors
                .iter()
                .map(|to| holder_events::HolderEmissionEdge::Outgoing {
                    from: point.id,
                    to: *to,
                })
                .collect::<Vec<_>>();
            for edge in edges {
                emit_connected_holder_actions_locked(context, state, edge, None)?;
            }
        }
        ownership::end_graph_mutation(&state.builder, state.module)?;
    }
    if state.return_body_span == Some(block.span) {
        result = materialize_return_branch(context, state, block, result)?;
        let outputs = crate::ScalarOutputSequence {
            outputs: block
                .final_output_values
                .iter()
                .map(|output| crate::ScalarOutput {
                    ty: output.ty.clone(),
                    span: output.span,
                })
                .collect(),
            span: block.span,
        };
        result = materialize_fresh_return_outputs(context, state, result, &outputs)?;
        state.fresh_return_outputs.clear();
    } else {
        result = materialize_return_branch(context, state, block, result)?;
    }
    let exit = state.holder_events.block_exit(&block.occurrence)?;
    state.holder_events.validated(exit)?;
    let exit_edges = state.holder_events.scope_action_edges(exit)?;
    for edge in exit_edges {
        emit_connected_holder_actions(context, state, edge, None)?;
    }
    release_scope_runtime_array_owners(context, state, &runtime_array_owners, &outer_names)?;
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    state.storage = storage;
    state.values = values;
    state.runtime_array_owners = merge_scope_map(runtime_array_owners, &state.runtime_array_owners);
    state.runtime_array_allocations =
        merge_scope_map(runtime_array_allocations, &state.runtime_array_allocations);
    state.runtime_array_lengths =
        merge_scope_map(runtime_array_lengths, &state.runtime_array_lengths);
    state.runtime_array_live = merge_scope_map(runtime_array_live, &state.runtime_array_live);
    state.backing_values = merge_scope_map(backing_values, &state.backing_values);
    state.backing_slots = merge_scope_map(backing_slots, &state.backing_slots);
    state.imported_cell_slots = merge_scope_map(imported_cell_slots, &state.imported_cell_slots);
    state.backing_captured = backing_captured
        .into_iter()
        .chain(state.backing_captured.iter().cloned())
        .collect();
    state.automatic_return_result_owners = automatic_return_result_owners;
    state.checked_owner_flags = checked_owner_flags;
    Ok(result)
}

fn record_final_return_holders<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    result: &EmitValue<'ctx>,
) -> Result<(), String> {
    for output in &block.final_output_values {
        if !matches!(
            output.ty,
            ScalarType::Callable { .. } | ScalarType::Struct(_)
        ) {
            continue;
        }
        let value = if block.final_output_values.len() == 1 {
            result.clone()
        } else {
            let EmitValue::Aggregate { value, .. } = result else {
                return Err(format!(
                    "return output {} lacks aggregate value at {:?}",
                    output.position, output.span
                ));
            };
            EmitValue::Basic(
                state
                    .builder
                    .build_extract_value(*value, output.position as u32, "return_holder_result")
                    .map_err(builder_error)?
                    .into(),
            )
        };
        memory::record_return_output(context, state, output, &value)?;
    }
    Ok(())
}

fn emit_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        if matches!(&output.value, ScalarExpression::Call { .. }) {
            transfer_automatic_return_result(state, &output.value);
            let value = emit_expression(context, state, &output.value)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, &output.value, output.position)?;
            }
            let value = apply_return_transport(context, state, block, output, false, value)?;
            return Ok(value);
        }
        let value = emit_typed_expression(context, state, &output.value, &output.ty)?;
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, &output.value, output.position)?;
        }
        let value = apply_return_transport(context, state, block, output, false, value)?;
        transfer_automatic_return_result(state, &output.value);
        return Ok(value);
    }
    if let Some(group) = recombine_conditional_final_outputs(&state.source, block)? {
        let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } = group.expression
        else {
            unreachable!("verified conditional source group")
        };
        for output in &block.final_output_values {
            if state.return_body_span == Some(block.span) {
                transfer_runtime_array_return(
                    state,
                    output.position,
                    &output.ty,
                    false,
                    block,
                    &output.value,
                )?;
            }
            transfer_automatic_return_result(state, &output.value);
        }
        let condition = take_basic(emit_expression(context, state, &condition)?)?.into_int_value();
        return emit_if_value(
            context,
            state,
            condition,
            &then_branch,
            &else_branch,
            |state, branch| emit_block(context, state, branch),
        );
    }
    let outputs = crate::ScalarOutputSequence {
        outputs: block
            .final_output_values
            .iter()
            .map(|value| crate::ScalarOutput {
                ty: value.ty.clone(),
                span: value.span,
            })
            .collect(),
        span: block.span,
    };
    let values = block
        .final_output_values
        .iter()
        .map(|output| {
            let value = emit_typed_expression(context, state, &output.value, &output.ty)?;
            record_return_length(context, state, &output.ty, &output.value, output.position)?;
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for (position, output) in block.final_output_values.iter().enumerate() {
            record_forwarded_return_owner(context, state, &output.value, position)?;
        }
    }
    let mut transported = Vec::new();
    let shared = block
        .final_output_values
        .iter()
        .all(|output| output.value == block.final_output_values[0].value);
    for (output, value) in block.final_output_values.iter().zip(values) {
        transfer_automatic_return_result(state, &output.value);
        transported.push(apply_return_transport(
            context, state, block, output, shared, value,
        )?);
    }
    build_aggregate(context, state, &outputs, transported)
}

fn apply_return_transport<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    output: &ScalarOutputValue,
    shared: bool,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    if state.return_body_span != Some(block.span) {
        if matches!(output.ty, ScalarType::RuntimeArray { .. })
            && state.return_length_slots.contains_key(&output.position)
            && matches!(output.value, ScalarExpression::Name { .. })
        {
            record_return_length(context, state, &output.ty, &output.value, output.position)?;
        }
        return Ok(value);
    }
    if output.ty == ScalarType::Unit {
        return Ok(value);
    }
    if block.final_output_values.len() == 1 && !matches!(output.value, ScalarExpression::If { .. })
    {
        record_return_length(context, state, &output.ty, &output.value, output.position)?;
    }
    let value = materialize_validated_return(
        context,
        state,
        output.position,
        shared,
        &output.value,
        value,
    )?;
    transfer_runtime_array_return(
        state,
        output.position,
        &output.ty,
        shared,
        block,
        &output.value,
    )?;
    Ok(value)
}

fn apply_expression_body_return<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    outputs: &crate::ScalarOutputSequence,
    expression: &ScalarExpression,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    if outputs
        .outputs
        .iter()
        .all(|output| output.ty == ScalarType::Unit)
    {
        return Ok(value);
    }
    let count = outputs.outputs.len();
    let extracted = match value {
        EmitValue::Aggregate { value, .. } => {
            let mut items = Vec::new();
            for index in 0..count {
                let extracted = state
                    .builder
                    .build_extract_value(value, index as u32, "return_output")
                    .map_err(builder_error)?;
                items.push(EmitValue::Basic(extracted.into()));
            }
            items
        }
        other => vec![other; count],
    };
    let mut transported = Vec::new();
    for (index, item) in extracted.into_iter().enumerate() {
        if outputs.outputs[index].ty == ScalarType::Unit {
            transported.push(item);
            continue;
        }
        record_return_length(
            context,
            state,
            &outputs.outputs[index].ty,
            expression,
            index,
        )?;
        let item = materialize_validated_return(context, state, index, true, expression, item)?;
        transfer_runtime_array_return(
            state,
            index,
            &outputs.outputs[index].ty,
            true,
            block,
            expression,
        )?;
        transported.push(item);
    }
    if count == 1 {
        return transported
            .into_iter()
            .next()
            .ok_or_else(|| "function return has no transported output".to_owned());
    }
    build_aggregate(context, state, outputs, transported)
}

fn emit_expression_body_return_handoff<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    result: &EmitValue<'ctx>,
) -> Result<(), String> {
    if !block.final_output_values.iter().any(|output| {
        matches!(
            output.ty,
            ScalarType::Callable { .. } | ScalarType::Struct(_)
        )
    }) {
        return Ok(());
    }
    ownership::begin_graph_mutation(&state.builder, state.module)?;
    record_final_return_holders(context, state, block, result)?;
    for output in &block.final_output_values {
        let point =
            state
                .holder_events
                .return_output(&state.source, output.span, output.position)?;
        let edges = point
            .successors
            .iter()
            .map(|to| holder_events::HolderEmissionEdge::Outgoing {
                from: point.id,
                to: *to,
            })
            .collect::<Vec<_>>();
        for edge in edges {
            emit_connected_holder_actions_locked(context, state, edge, None)?;
        }
    }
    ownership::end_graph_mutation(&state.builder, state.module)
}

fn apply_call_argument_transport<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    span: (u32, u32),
    arguments: &[ScalarExpression],
    parameters: &[ScalarType],
    values: Vec<EmitValue<'ctx>>,
) -> Result<Vec<EmitValue<'ctx>>, String> {
    let mut transported = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        let (Some(argument), Some(parameter)) = (arguments.get(index), parameters.get(index))
        else {
            transported.push(value);
            continue;
        };
        if !matches!(parameter, ScalarType::RuntimeArray { .. }) {
            transported.push(value);
            continue;
        }
        match argument {
            ScalarExpression::Name { name, .. } => match storage_backing_type(state, name) {
                Some(ScalarType::Array {
                    element, length, ..
                }) => {
                    require_call_argument_path(state, span, index, name)?;
                    let source = take_basic(value)?.into_pointer_value();
                    let destination = ownership::materialize_fixed_copy(
                        context,
                        state,
                        source,
                        &element,
                        length,
                        ownership::FixedCopyPublication::RequiresHolderAction,
                    )?;
                    transported.push(EmitValue::Basic(destination.into()));
                }
                Some(ScalarType::RuntimeArray { .. }) => {
                    if state
                        .runtime_array_owners
                        .get(name)
                        .copied()
                        .unwrap_or(false)
                    {
                        require_call_argument_paths(state, span, name)?;
                        eprintln!("[D3DBG] call-transport retain name={name}");
                        transfer_backing_ownership(state, name);
                    }
                    transported.push(value);
                }
                _ => transported.push(value),
            },
            ScalarExpression::Call { .. } => transported.push(value),
            _ => {
                return Err(ownership::missing_backing_transport(
                    &format!("call argument {index}"),
                    span,
                ));
            }
        }
    }
    Ok(transported)
}

fn require_call_argument_path(
    state: &EmitState<'_, '_>,
    span: (u32, u32),
    index: usize,
    name: &str,
) -> Result<(), String> {
    let validated = state
        .backing
        .call_paths
        .get(&span)
        .map(|paths| {
            paths
                .iter()
                .any(|path| path.iter().any(|fact| fact.position == index))
        })
        .unwrap_or(false);
    if validated {
        Ok(())
    } else {
        Err(ownership::missing_backing_transport(
            &format!("call argument {index} (`{name}`)"),
            span,
        ))
    }
}

fn require_call_argument_paths(
    state: &EmitState<'_, '_>,
    span: (u32, u32),
    name: &str,
) -> Result<(), String> {
    if state.backing.call_paths.contains_key(&span) {
        Ok(())
    } else {
        Err(ownership::missing_backing_transport(
            &format!("call argument (`{name}`)"),
            span,
        ))
    }
}

use imports::import_call_result_captures;

fn emit_project_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        if matches!(&output.value, ScalarExpression::Call { .. }) {
            transfer_automatic_return_result(state, &output.value);
            let value = emit_project_expression(context, state, &output.value, module, modules)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, &output.value, output.position)?;
            }
            let value = apply_return_transport(context, state, block, output, false, value)?;
            return Ok(value);
        }
        let value = emit_project_typed_expression(
            context,
            state,
            &output.value,
            &output.ty,
            module,
            modules,
        )?;
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, &output.value, output.position)?;
        }
        let value = apply_return_transport(context, state, block, output, false, value)?;
        transfer_automatic_return_result(state, &output.value);
        return Ok(value);
    }
    if let Some(group) = recombine_conditional_final_outputs(&state.source, block)? {
        let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } = group.expression
        else {
            unreachable!("verified conditional source group")
        };
        for output in &block.final_output_values {
            if state.return_body_span == Some(block.span) {
                transfer_runtime_array_return(
                    state,
                    output.position,
                    &output.ty,
                    false,
                    block,
                    &output.value,
                )?;
            }
            transfer_automatic_return_result(state, &output.value);
        }
        let condition = take_basic(emit_project_expression(
            context, state, &condition, module, modules,
        )?)?
        .into_int_value();
        return emit_if_value(
            context,
            state,
            condition,
            &then_branch,
            &else_branch,
            |state, branch| emit_project_block(context, state, branch, module, modules),
        );
    }
    let outputs = crate::ScalarOutputSequence {
        outputs: block
            .final_output_values
            .iter()
            .map(|value| crate::ScalarOutput {
                ty: value.ty.clone(),
                span: value.span,
            })
            .collect(),
        span: block.span,
    };
    let values = block
        .final_output_values
        .iter()
        .map(|output| {
            let value = emit_project_typed_expression(
                context,
                state,
                &output.value,
                &output.ty,
                module,
                modules,
            )?;
            record_return_length(context, state, &output.ty, &output.value, output.position)?;
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for (position, output) in block.final_output_values.iter().enumerate() {
            record_forwarded_return_owner(context, state, &output.value, position)?;
        }
    }
    let mut transported = Vec::new();
    let shared = block
        .final_output_values
        .iter()
        .all(|output| output.value == block.final_output_values[0].value);
    for (output, value) in block.final_output_values.iter().zip(values) {
        transfer_automatic_return_result(state, &output.value);
        transported.push(apply_return_transport(
            context, state, block, output, shared, value,
        )?);
    }
    build_aggregate(context, state, &outputs, transported)
}

fn recombine_conditional_final_outputs(
    source: &wosy_syntax::SourceIdentity,
    block: &ScalarBlock,
) -> Result<Option<crate::scalar::ConditionalOutputGroup>, String> {
    crate::scalar::group_conditional_final_outputs(source, block)
        .map_err(|error| format!("{error:?}"))
}

fn emit_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
) -> Result<EmitValue<'ctx>, String> {
    let (plan, evaluation) = state.holder_events.assignment_evaluation(assignment)?;
    let expected = plan
        .targets
        .iter()
        .map(|target| Some(target.ty.clone()))
        .collect::<Vec<_>>();
    let values = materialize_assignment_values(
        context,
        state,
        &assignment.values,
        &expected,
        |state, value, expected| match expected {
            Some(ty) => emit_typed_expression(context, state, value, ty),
            None => emit_expression(context, state, value),
        },
    )?;
    let mut destinations = Vec::with_capacity(assignment.targets.len());
    for position in 0..assignment.targets.len() {
        destinations.push(freeze_assignment_destination(
            context,
            state,
            assignment,
            position,
            &values[position].1,
            None,
        )?);
    }
    let prepared = snapshot_assignment_outputs(context, state, &plan, &evaluation, &destinations)?;
    for (position, (target, (value, ty, call_flag))) in
        assignment.targets.iter().zip(values).enumerate()
    {
        let (destination, selection) = match &destinations[position] {
            FrozenAssignmentDestination::Unit => {
                state.values.insert(target.target.clone(), EmitValue::Unit);
                continue;
            }
            FrozenAssignmentDestination::Storage { address, selection } => (*address, selection),
        };
        let expression = if assignment.values.len() == 1 {
            &assignment.values[0]
        } else {
            &assignment.values[position]
        };
        let action_edges = if matches!(ty, ScalarType::Callable { .. } | ScalarType::Struct(_)) {
            let assign = state.holder_events.point(
                &state.source,
                target.target_span,
                holder_events::HolderPointKind::Assign,
            )?;
            state.holder_events.validated(assign)?;
            assign
                .successors
                .iter()
                .map(|to| holder_events::HolderEmissionEdge::Outgoing {
                    from: assign.id,
                    to: *to,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let value = transition_joined_checked_assignment(
            context,
            state,
            target,
            expression,
            position,
            destination,
            &ty,
            value,
            call_flag,
        )?;
        ownership::begin_graph_mutation(&state.builder, state.module)?;
        if let FrozenAssignmentSelection::Callable(selection) = selection {
            record_selected_element(
                context,
                state,
                selection.point,
                selection.index_point,
                selection.index,
                destination,
            )?;
        }
        validate_fixed_copy_replacement_release(state, assignment, position, target, &ty)?;
        release_replaced_callable_array(context, state, target, &ty, expression)?;
        let value =
            transition_runtime_array_assignment(context, state, target, &ty, expression, value)?;
        let replaced_cell =
            prepare_counted_replacement(context, state, assignment, position, target, &ty)?;
        let copy_edges = state
            .holder_events
            .assignment_copy_edges(assignment, position)?;
        for edge in &action_edges {
            emit_connected_holder_actions_locked(context, state, *edge, Some(true))?;
        }
        for edge in &copy_edges {
            emit_connected_holder_actions_locked(context, state, *edge, Some(true))?;
        }
        store_value_locked(context, state, destination, &ty, value)?;
        emit_assignment_fixed_copy_actions_locked(
            context, state, assignment, position, target, &ty,
        )?;
        for edge in action_edges {
            emit_connected_holder_actions_locked(context, state, edge, Some(false))?;
        }
        for edge in copy_edges {
            emit_connected_holder_actions_locked(context, state, edge, Some(false))?;
        }
        if let Some(cell) = replaced_cell {
            finish_counted_replacement(context, state, cell, target, expression)?;
        } else {
            apply_assignment_ownership(context, state, target, &ty, expression, destination)?;
        }
        ownership::end_graph_mutation(&state.builder, state.module)?;
    }
    finish_assignment_outputs(context, state, &plan, &evaluation, prepared)
}
fn emit_assignment_fixed_copy_actions_locked<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    position: usize,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
    {
        return Ok(());
    }
    let span = assignment_backing_span(state, target);
    let Some((RuntimeBackingTransfer::CopyFixed { source }, _, _)) =
        state.backing.transfers.get(&span)
    else {
        return Ok(());
    };
    if !matches!(target.place, crate::ScalarPlace::Name { .. }) {
        return Err(format!(
            "fixed callable copy at {}..{} lacks source-qualified named destination",
            span.0, span.1
        ));
    }
    let destination = state
        .holder_events
        .assignment_binding(assignment, position)?;
    for edge in
        state
            .holder_events
            .fixed_copy_action_edges(&destination, source, target.target_span)?
    {
        emit_connected_holder_actions_locked(context, state, edge, Some(false))?;
    }
    Ok(())
}
fn validate_fixed_copy_replacement_release(
    state: &EmitState<'_, '_>,
    assignment: &ScalarAssignment,
    position: usize,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
    {
        return Ok(());
    }
    let span = assignment_backing_span(state, target);
    let Some((RuntimeBackingTransfer::CopyFixed { source }, _, _)) =
        state.backing.transfers.get(&span)
    else {
        return Ok(());
    };
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Err(format!(
            "fixed callable replacement at {}..{} lacks a named destination",
            span.0, span.1
        ));
    };
    let previous = state.backing_values.get(name).ok_or_else(|| {
        format!(
            "fixed callable replacement of {name} lacks old owned backing at {}..{}",
            span.0, span.1
        )
    })?;
    let destination = state
        .holder_events
        .assignment_binding(assignment, position)?;
    for edge in
        state
            .holder_events
            .fixed_copy_action_edges(&destination, source, target.target_span)?
    {
        let actions = state.holder_events.connected_actions(edge)?;
        for value in previous {
            if !actions.iter().any(|action| {
                matches!(action,
                    crate::scalar::CallableHolderAction::DropArrayElements { backing, .. }
                        if backing.owner == destination && &backing.value == value
                )
            }) {
                return Err(format!("fixed callable replacement of {name} lacks connected old-element Drop at {}..{}", span.0, span.1));
            }
        }
    }
    Ok(())
}
fn emit_project_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let (plan, evaluation) = state.holder_events.assignment_evaluation(assignment)?;
    let expected = plan
        .targets
        .iter()
        .map(|target| Some(target.ty.clone()))
        .collect::<Vec<_>>();
    let values = materialize_assignment_values(
        context,
        state,
        &assignment.values,
        &expected,
        |state, value, expected| match expected {
            Some(ty) => emit_project_typed_expression(context, state, value, ty, module, modules),
            None => emit_project_expression(context, state, value, module, modules),
        },
    )?;
    let mut destinations = Vec::with_capacity(assignment.targets.len());
    for position in 0..assignment.targets.len() {
        destinations.push(freeze_assignment_destination(
            context,
            state,
            assignment,
            position,
            &values[position].1,
            Some(ProjectCallScope { module, modules }),
        )?);
    }
    let prepared = snapshot_assignment_outputs(context, state, &plan, &evaluation, &destinations)?;
    for (position, (target, (value, ty, call_flag))) in
        assignment.targets.iter().zip(values).enumerate()
    {
        let (destination, selection) = match &destinations[position] {
            FrozenAssignmentDestination::Unit => {
                state.values.insert(target.target.clone(), EmitValue::Unit);
                continue;
            }
            FrozenAssignmentDestination::Storage { address, selection } => (*address, selection),
        };
        let expression = if assignment.values.len() == 1 {
            &assignment.values[0]
        } else {
            &assignment.values[position]
        };
        let action_edges = if matches!(ty, ScalarType::Callable { .. } | ScalarType::Struct(_)) {
            let assign = state.holder_events.point(
                &state.source,
                target.target_span,
                holder_events::HolderPointKind::Assign,
            )?;
            state.holder_events.validated(assign)?;
            assign
                .successors
                .iter()
                .map(|to| holder_events::HolderEmissionEdge::Outgoing {
                    from: assign.id,
                    to: *to,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let value = transition_joined_checked_assignment(
            context,
            state,
            target,
            expression,
            position,
            destination,
            &ty,
            value,
            call_flag,
        )?;
        ownership::begin_graph_mutation(&state.builder, state.module)?;
        if let FrozenAssignmentSelection::Callable(selection) = selection {
            record_selected_element(
                context,
                state,
                selection.point,
                selection.index_point,
                selection.index,
                destination,
            )?;
        }
        validate_fixed_copy_replacement_release(state, assignment, position, target, &ty)?;
        release_replaced_callable_array(context, state, target, &ty, expression)?;
        let value =
            transition_runtime_array_assignment(context, state, target, &ty, expression, value)?;
        let replaced_cell =
            prepare_counted_replacement(context, state, assignment, position, target, &ty)?;
        let copy_edges = state
            .holder_events
            .assignment_copy_edges(assignment, position)?;
        for edge in &action_edges {
            emit_connected_holder_actions_locked(context, state, *edge, Some(true))?;
        }
        for edge in &copy_edges {
            emit_connected_holder_actions_locked(context, state, *edge, Some(true))?;
        }
        store_value_locked(context, state, destination, &ty, value)?;
        emit_assignment_fixed_copy_actions_locked(
            context, state, assignment, position, target, &ty,
        )?;
        for edge in action_edges {
            emit_connected_holder_actions_locked(context, state, edge, Some(false))?;
        }
        for edge in copy_edges {
            emit_connected_holder_actions_locked(context, state, edge, Some(false))?;
        }
        if let Some(cell) = replaced_cell {
            finish_counted_replacement(context, state, cell, target, expression)?;
        } else {
            apply_assignment_ownership(context, state, target, &ty, expression, destination)?;
        }
        ownership::end_graph_mutation(&state.builder, state.module)?;
    }
    finish_assignment_outputs(context, state, &plan, &evaluation, prepared)
}

fn transition_joined_checked_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    expression: &ScalarExpression,
    output: usize,
    destination: PointerValue<'ctx>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
    call_flag: Option<inkwell::values::IntValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(value);
    };
    let Some((owner_slot, _)) = state.joined_return_bindings.get(name).cloned() else {
        return Ok(value);
    };
    let classification = *state
        .joined_assignment_owners
        .get(&(target.target_span.start, target.target_span.end))
        .ok_or_else(|| {
            format!(
                "missing validated assignment owner at {:?}",
                target.target_span
            )
        })?;
    let ScalarType::CheckedReference { inner, .. } = ty else {
        return Err(format!(
            "joined return binding {name} lacks checked-reference type"
        ));
    };
    let (value, owned) = match classification {
        CheckedAssignmentOwner::Local => {
            if let ScalarExpression::Name { name: source, .. } = expression {
                if let Some((slot, _)) = state.joined_return_bindings.get(source) {
                    let owned = state
                        .builder
                        .build_load(context.bool_type(), *slot, "joined_assignment_owned")
                        .map_err(builder_error)?
                        .into_int_value();
                    (value, owned)
                } else {
                    let copied =
                        emit_automatic_return_result(context, state, take_basic(value)?, inner)?;
                    (
                        EmitValue::Basic(copied),
                        context.bool_type().const_int(1, false),
                    )
                }
            } else {
                let copied =
                    emit_automatic_return_result(context, state, take_basic(value)?, inner)?;
                (
                    EmitValue::Basic(copied),
                    context.bool_type().const_int(1, false),
                )
            }
        }
        CheckedAssignmentOwner::External => {
            let owned = match expression {
                ScalarExpression::Call { span, .. } => match state
                    .checked_call_owners
                    .get(&(span.start, span.end, output))
                    .ok_or_else(|| format!("missing validated checked-call owner at {span:?}"))?
                {
                    CheckedCallOwner::Fresh => context.bool_type().const_int(1, false),
                    CheckedCallOwner::Borrowed => context.bool_type().const_zero(),
                    CheckedCallOwner::Mixed => call_flag
                        .ok_or_else(|| format!("missing mixed-call owner flag at {span:?}"))?,
                },
                ScalarExpression::Name { name: source, .. } => {
                    if let Some((slot, _)) = state.joined_return_bindings.get(source) {
                        state
                            .builder
                            .build_load(context.bool_type(), *slot, "joined_assignment_owned")
                            .map_err(builder_error)?
                            .into_int_value()
                    } else if let Some(flag) = state.checked_owner_flags.get(source) {
                        *flag
                    } else if state.automatic_return_result_owners.contains_key(source) {
                        context.bool_type().const_int(1, false)
                    } else {
                        return Err(format!("missing validated external owner for {source}"));
                    }
                }
                _ => {
                    return Err(format!(
                        "joined return assignment at {:?} has no typed owner source",
                        target.target_span
                    ))
                }
            };
            (value, owned)
        }
        CheckedAssignmentOwner::Borrowed => (value, context.bool_type().const_zero()),
    };
    let address = take_basic(value.clone())?.into_int_value();
    let previous_owner = state
        .builder
        .build_load(context.bool_type(), owner_slot, "joined_previous_owner")
        .map_err(builder_error)?
        .into_int_value();
    let previous_address = state
        .builder
        .build_load(
            basic_type(context, ty, state.target_layout)?,
            destination,
            "joined_previous_address",
        )
        .map_err(builder_error)?
        .into_int_value();
    let different = state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            previous_address,
            address,
            "joined_owner_replaced",
        )
        .map_err(builder_error)?;
    let live = state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            previous_address,
            previous_address.get_type().const_zero(),
            "joined_previous_live",
        )
        .map_err(builder_error)?;
    let release = state
        .builder
        .build_and(previous_owner, different, "joined_previous_owned")
        .map_err(builder_error)?;
    let release = state
        .builder
        .build_and(release, live, "joined_previous_release")
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing joined assignment block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing joined assignment function".to_owned())?;
    let free = context.append_basic_block(function, "joined_previous_release.live");
    let next = context.append_basic_block(function, "joined_previous_release.next");
    state
        .builder
        .build_conditional_branch(release, free, next)
        .map_err(builder_error)?;
    state.builder.position_at_end(free);
    let pointer = state
        .builder
        .build_int_to_ptr(
            previous_address,
            context.ptr_type(AddressSpace::default()),
            "joined_previous_pointer",
        )
        .map_err(builder_error)?;
    declare_core_runtime(context, state.module);
    let release = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(release, &[pointer.into()], "joined_previous_free")
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(next)
        .map_err(builder_error)?;
    state.builder.position_at_end(next);
    state
        .builder
        .build_store(owner_slot, owned)
        .map_err(builder_error)?;
    if let ScalarExpression::Name { name: source, .. } = expression {
        if source != name {
            if let Some((slot, _)) = state.joined_return_bindings.get(source) {
                state
                    .builder
                    .build_store(*slot, context.bool_type().const_zero())
                    .map_err(builder_error)?;
            }
        }
    }
    Ok(value)
}

fn release_replaced_callable_array<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    expression: &ScalarExpression,
) -> Result<(), String> {
    let ScalarType::RuntimeArray { element, .. } = ty else {
        return Ok(());
    };
    if !memory::has_callable_holders(element, state.structs)? {
        return Ok(());
    }
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(());
    };
    let ScalarExpression::Name { name: source, .. } = expression else {
        return Err(format!(
            "local callable array `{name}` replacement requires a connected source"
        ));
    };
    if name == source {
        return Err(format!(
            "local callable array `{name}` cannot replace itself"
        ));
    }
    let source_length = match storage_backing_type(state, source) {
        Some(ScalarType::Array { length, .. }) => {
            let slot = entry_alloca(state, context.i64_type().into(), "fixed_replacement_length")?;
            state
                .builder
                .build_store(slot, context.i64_type().const_int(length, false))
                .map_err(builder_error)?;
            slot
        }
        Some(ScalarType::RuntimeArray { .. }) => *state
            .runtime_array_lengths
            .get(source)
            .ok_or_else(|| format!("local callable array `{source}` lacks evaluated length"))?,
        _ => {
            return Err(format!(
                "local callable array `{source}` lacks source-qualified array layout"
            ))
        }
    };
    release_backing_entry_locked(context, state, name)?;
    state
        .runtime_array_lengths
        .insert(name.clone(), source_length);
    Ok(())
}

fn transition_runtime_array_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    expression: &ScalarExpression,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(value);
    }
    if matches!(target.place, crate::ScalarPlace::Dereference { .. }) {
        return Ok(value);
    }
    let span = assignment_backing_span(state, target);
    if matches!(target.place, crate::ScalarPlace::Name { .. }) {
        apply_runtime_value(context, state, span, ty, expression, value)
    } else {
        apply_place_value(context, state, ty, expression, value)
    }
}

fn assignment_backing_span(
    state: &EmitState<'_, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
) -> (u32, u32) {
    let place_span = match &target.place {
        crate::ScalarPlace::Name { span, .. }
        | crate::ScalarPlace::Field { span, .. }
        | crate::ScalarPlace::Index { span, .. }
        | crate::ScalarPlace::Dereference { span, .. } => (span.start, span.end),
    };
    let target_span = (target.target_span.start, target.target_span.end);
    if state.backing.transfers.contains_key(&target_span)
        || state.backing.copies.contains_key(&target_span)
    {
        target_span
    } else {
        place_span
    }
}

fn apply_place_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    ty: &ScalarType,
    expression: &ScalarExpression,
    value: EmitValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let ScalarExpression::Name { name, .. } = expression else {
        return Ok(value);
    };
    let Some(ScalarType::Array {
        element, length, ..
    }) = storage_backing_type(state, name)
    else {
        return Ok(value);
    };
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(value);
    }
    let source = take_basic(value)?.into_pointer_value();
    let destination = ownership::materialize_fixed_copy(
        context,
        state,
        source,
        &element,
        length,
        ownership::FixedCopyPublication::RequiresHolderAction,
    )?;
    Ok(EmitValue::Basic(destination.into()))
}

fn apply_assignment_ownership<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    expression: &ScalarExpression,
    destination: PointerValue<'ctx>,
) -> Result<(), String> {
    let key = match place_key(state, &target.place) {
        Ok(key) => key,
        Err(_) => return Ok(()),
    };
    let span = assignment_backing_span(state, target);
    if matches!(ty, ScalarType::RuntimeArray { .. }) {
        if matches!(target.place, crate::ScalarPlace::Name { .. }) {
            apply_runtime_ownership(state, &key, span, ty, expression)?;
        } else {
            apply_place_ownership(context, state, &key, expression)?;
        }
        return Ok(());
    }
    if matches!(ty, ScalarType::Struct(_)) {
        apply_struct_backings(
            context,
            state,
            &key,
            destination,
            ty,
            expression,
            Some(span),
            true,
        )?;
    }
    Ok(())
}

fn apply_place_ownership<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    key: &str,
    expression: &ScalarExpression,
) -> Result<(), String> {
    release_backing_entry_locked(context, state, key)?;
    match expression {
        ScalarExpression::Name { name, .. } => match storage_backing_type(state, name) {
            Some(ScalarType::Array { .. }) => {
                register_backing_entry(state, key.into(), Vec::new());
            }
            Some(ScalarType::RuntimeArray { .. }) => {
                if state
                    .runtime_array_owners
                    .get(name)
                    .copied()
                    .unwrap_or(false)
                {
                    transfer_backing_ownership(state, name);
                    register_backing_entry(state, key.into(), Vec::new());
                }
            }
            _ => {}
        },
        ScalarExpression::Call { .. } => {
            register_backing_entry(state, key.into(), Vec::new());
        }
        _ => {}
    }
    Ok(())
}

fn assignment_target_type<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
) -> Option<ScalarType> {
    assignment_place_type(state, &target.place)
}

fn assignment_place_type<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
) -> Option<ScalarType> {
    match place {
        crate::ScalarPlace::Name { name, .. } => state
            .storage
            .get(name.as_str())
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name.as_str()).map(|(_, ty)| ty.clone())),
        crate::ScalarPlace::Field { field, .. } => match field {
            ScalarFieldReference::Resolved(field) => {
                structure(state.structs, field.structure.clone())
                    .ok()?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == *field)
                    .map(|field| field.ty.clone())
            }
            ScalarFieldReference::Unresolved { .. } => None,
        },
        crate::ScalarPlace::Dereference { pointer, .. } => {
            pointer_target_type(state, pointer, None)
        }
        crate::ScalarPlace::Index { base, .. } => match assignment_place_type(state, base)? {
            ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                Some(*element)
            }
            _ => None,
        },
    }
}

fn core_pointer_cast_dereference_type(
    expression: &ScalarExpression,
) -> Option<(&ScalarType, bool)> {
    let ScalarExpression::Call {
        receiver: Some(receiver),
        name,
        type_arguments,
        ..
    } = expression
    else {
        return None;
    };
    if receiver != "core" || name != "pointer_cast" {
        return None;
    }
    match &type_arguments.first()?.ty {
        ScalarType::RawPointer(inner) => Some((inner, false)),
        ScalarType::CheckedReference { inner, .. } => Some((inner, true)),
        _ => None,
    }
}

fn pointer_target_type<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Option<ScalarType> {
    match expression {
        ScalarExpression::Assignment(assignment) => state
            .holder_events
            .assignment_result_shape(assignment)
            .and_then(|shape| match &shape.ty {
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                    Some(*inner.clone())
                }
                _ => None,
            }),
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty)
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
            .and_then(|ty| match ty {
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                    Some(*inner.clone())
                }
                _ => None,
            }),
        ScalarExpression::RawAddress { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            if let Some((inner, _)) = core_pointer_cast_dereference_type(expression) {
                return Some(inner.clone());
            }
            let qualified = project.map_or_else(
                || {
                    receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"))
                },
                |project| {
                    let target = receiver
                        .as_ref()
                        .and_then(|binding| {
                            project
                                .module
                                .namespace_bindings
                                .iter()
                                .find(|namespace| namespace.binding == *binding)
                                .and_then(|namespace| {
                                    project
                                        .modules
                                        .iter()
                                        .find(|candidate| candidate.source == namespace.target)
                                        .copied()
                                })
                        })
                        .unwrap_or(project.module);
                    project_function_name(&target.source, name)
                },
            );
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified))
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .and_then(|output| match &output.ty {
                    ScalarType::CheckedReference { inner, .. } => Some(*inner.clone()),
                    ScalarType::RawPointer(inner) => Some(*inner.clone()),
                    _ => None,
                })
        }
        _ => None,
    }
}

fn dereference_requires_null_guard<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> bool {
    match expression {
        ScalarExpression::Assignment(assignment) => state
            .holder_events
            .assignment_result_shape(assignment)
            .is_some_and(|shape| matches!(shape.ty, ScalarType::CheckedReference { .. })),
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty)
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
            .is_some_and(|ty| matches!(ty, ScalarType::CheckedReference { .. })),
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            if let Some((_, checked)) = core_pointer_cast_dereference_type(expression) {
                return checked;
            }
            let qualified = project.map_or_else(
                || {
                    receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"))
                },
                |project| {
                    let target = receiver
                        .as_ref()
                        .and_then(|binding| {
                            project
                                .module
                                .namespace_bindings
                                .iter()
                                .find(|namespace| namespace.binding == *binding)
                                .and_then(|namespace| {
                                    project
                                        .modules
                                        .iter()
                                        .find(|candidate| candidate.source == namespace.target)
                                        .copied()
                                })
                        })
                        .unwrap_or(project.module);
                    project_function_name(&target.source, name)
                },
            );
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified))
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .is_some_and(|output| matches!(&output.ty, ScalarType::CheckedReference { .. }))
        }
        _ => false,
    }
}

fn emit_checked_dereference_guard<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    address: inkwell::values::IntValue<'ctx>,
) -> Result<(), String> {
    declare_core_runtime(context, state.module);
    let is_null = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            address,
            address.get_type().const_int(0, false),
            "deref_is_null",
        )
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .ok_or_else(|| "checked dereference has no containing function".to_owned())?;
    let panic_block = context.append_basic_block(function, "deref_null_panic");
    let continue_block = context.append_basic_block(function, "deref_null_continue");
    state
        .builder
        .build_conditional_branch(is_null, panic_block, continue_block)
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
    Ok(())
}

fn materialize_assignment_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expressions: &[ScalarExpression],
    expected: &[Option<ScalarType>],
    emit: impl FnMut(
        &mut EmitState<'ctx, 'module>,
        &ScalarExpression,
        Option<&ScalarType>,
    ) -> Result<EmitValue<'ctx>, String>,
) -> Result<
    Vec<(
        EmitValue<'ctx>,
        ScalarType,
        Option<inkwell::values::IntValue<'ctx>>,
    )>,
    String,
> {
    let mut emit = emit;
    let mut position = 0;
    let mut values = Vec::new();
    for expression in expressions {
        let value = emit(
            state,
            expression,
            expected.get(position).and_then(Option::as_ref),
        )?;
        let call_flags = state.call_owner_flags.clone();
        let output_types = match &value {
            EmitValue::Aggregate { outputs, .. } => outputs.clone(),
            EmitValue::Unit => vec![ScalarType::Unit],
            EmitValue::Basic(value) => vec![expected
                .get(position)
                .and_then(Option::as_ref)
                .cloned()
                .unwrap_or_else(|| basic_value_type(*value))],
        };
        for (output_position, ty) in output_types.into_iter().enumerate() {
            let value = extract_output(state, value.clone(), output_position)?;
            if ty == ScalarType::Unit {
                values.push((EmitValue::Unit, ty, None));
                position += 1;
                continue;
            }
            let temporary = entry_alloca(
                state,
                storage_type(context, &ty, state.structs, state.target_layout)?,
                "assignment_value",
            )?;
            store_value(context, state, temporary, &ty, value)?;
            let value = if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                EmitValue::Basic(temporary.into())
            } else {
                EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            basic_type(context, &ty, state.target_layout)?,
                            temporary,
                            "assignment_value",
                        )
                        .map_err(builder_error)?,
                )
            };
            values.push((value, ty, call_flags.get(&output_position).copied()));
            position += 1;
        }
    }
    Ok(values)
}

fn basic_value_type(value: BasicValueEnum<'_>) -> ScalarType {
    match value {
        BasicValueEnum::IntValue(value) => match value.get_type().get_bit_width() {
            1 => ScalarType::Bool,
            8 => ScalarType::I8,
            16 => ScalarType::I16,
            32 => ScalarType::I32,
            64 => ScalarType::I64,
            128 => ScalarType::I128,
            width => panic!("unsupported LLVM integer width {width}"),
        },
        BasicValueEnum::FloatValue(value)
            if value.get_type() == value.get_type().get_context().f32_type() =>
        {
            ScalarType::F32
        }
        BasicValueEnum::FloatValue(_) => ScalarType::F64,
        BasicValueEnum::PointerValue(_) => ScalarType::RawPointer(Box::new(ScalarType::I8)),
        BasicValueEnum::ArrayValue(_)
        | BasicValueEnum::StructValue(_)
        | BasicValueEnum::VectorValue(_)
        | BasicValueEnum::ScalableVectorValue(_) => {
            panic!("unsupported inferred LLVM assignment value")
        }
    }
}

fn emit_block_item<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    item: &ScalarBlockItem,
) -> Result<EmitValue<'ctx>, String> {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let value = emit_binding_value(context, state, binding)?;
            store_binding_outputs(context, state, binding, value)?;
            Ok(EmitValue::Unit)
        }
        ScalarBlockItem::Expression(expression) => emit_expression(context, state, expression),
        ScalarBlockItem::Assignment(assignment) => emit_assignment(context, state, assignment),
        ScalarBlockItem::While(expression) => emit_while(context, state, expression),
    }
}

fn emit_project_block_item<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    item: &ScalarBlockItem,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            if binding.is_allocation {
                let receiver = binding
                    .receivers
                    .first()
                    .ok_or_else(|| "runtime allocation has no receiver".to_owned())?;
                let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
                    return Err("allocation binding requires a runtime array type".to_owned());
                };
                let length = receiver
                    .allocation_length
                    .as_ref()
                    .ok_or_else(|| "runtime allocation has no length".to_owned())?;
                let length = take_basic(emit_project_typed_expression(
                    context,
                    state,
                    length,
                    &ScalarType::U64,
                    module,
                    modules,
                )?)?
                .into_int_value();
                let slot = emit_runtime_array_allocation(context, state, element, length)?;
                let length_slot = entry_alloca(
                    state,
                    context.i64_type().into(),
                    &format!("{}_length", receiver.name),
                )?;
                state
                    .builder
                    .build_store(length_slot, length)
                    .map_err(builder_error)?;
                state
                    .runtime_array_lengths
                    .insert(receiver.name.clone(), length_slot);
                if !state
                    .mutable_runtime_allocations
                    .contains(&(receiver.name_span.start, receiver.name_span.end))
                {
                    state
                        .storage
                        .insert(receiver.name.clone(), (slot, receiver.ty.clone()));
                    state
                        .runtime_array_owners
                        .insert(receiver.name.clone(), true);
                    state
                        .runtime_array_allocations
                        .insert(receiver.name.clone(), true);
                    return Ok(EmitValue::Unit);
                }
                let backing = entry_alloca(
                    state,
                    context.ptr_type(AddressSpace::default()).into(),
                    &format!("{}_backing", receiver.name),
                )?;
                state
                    .builder
                    .build_store(backing, slot)
                    .map_err(builder_error)?;
                let live = entry_alloca(
                    state,
                    context.bool_type().into(),
                    &format!("{}_live", receiver.name),
                )?;
                state
                    .builder
                    .build_store(live, context.bool_type().const_int(1, false))
                    .map_err(builder_error)?;
                state.runtime_array_live.insert(receiver.name.clone(), live);
                state
                    .storage
                    .insert(receiver.name.clone(), (backing, receiver.ty.clone()));
                state
                    .runtime_array_owners
                    .insert(receiver.name.clone(), true);
                state
                    .runtime_array_allocations
                    .insert(receiver.name.clone(), false);
                return Ok(EmitValue::Unit);
            }
            let value = emit_project_typed_expression(
                context,
                state,
                &binding.value,
                &binding.declared_type,
                module,
                modules,
            )?;
            store_binding_outputs(context, state, binding, value)?;
            Ok(EmitValue::Unit)
        }
        ScalarBlockItem::Expression(expression) => {
            emit_project_expression(context, state, expression, module, modules)
        }
        ScalarBlockItem::Assignment(assignment) => {
            emit_project_assignment(context, state, assignment, module, modules)
        }
        ScalarBlockItem::While(expression) => {
            emit_project_while(context, state, expression, module, modules)
        }
    }
}

fn emit_while<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarWhile,
) -> Result<EmitValue<'ctx>, String> {
    let loop_test = state.holder_events.while_point(
        &expression.occurrence,
        crate::scalar::ScalarWhilePointRole::LoopTest,
    )?;
    let branch = state.holder_events.while_point(
        &expression.occurrence,
        crate::scalar::ScalarWhilePointRole::Branch,
    )?;
    state.holder_events.arm(branch.id, 0)?;
    state.holder_events.arm(branch.id, 1)?;
    let back = state.holder_events.while_point(
        &expression.occurrence,
        crate::scalar::ScalarWhilePointRole::BodyScopeExit,
    )?;
    state.holder_events.outgoing(back.id, loop_test.id)?;
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let condition_block =
        context.append_basic_block(function, &format!("while.cond.{}", state.next_block));
    state.next_block += 1;
    let body = context.append_basic_block(function, &format!("while.body.{}", state.next_block));
    state.next_block += 1;
    let exit = context.append_basic_block(function, &format!("while.exit.{}", state.next_block));
    state.next_block += 1;
    state
        .builder
        .build_unconditional_branch(condition_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(condition_block);
    let condition =
        take_basic(emit_expression(context, state, &expression.condition)?)?.into_int_value();
    state
        .builder
        .build_conditional_branch(condition, body, exit)
        .map_err(builder_error)?;
    state.builder.position_at_end(body);
    let _ = emit_block(context, state, &expression.body)?;
    state
        .builder
        .build_unconditional_branch(condition_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(exit);
    Ok(EmitValue::Unit)
}

fn emit_project_while<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarWhile,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let loop_test = state.holder_events.while_point(
        &expression.occurrence,
        crate::scalar::ScalarWhilePointRole::LoopTest,
    )?;
    let branch = state.holder_events.while_point(
        &expression.occurrence,
        crate::scalar::ScalarWhilePointRole::Branch,
    )?;
    state.holder_events.arm(branch.id, 0)?;
    state.holder_events.arm(branch.id, 1)?;
    let back = state.holder_events.while_point(
        &expression.occurrence,
        crate::scalar::ScalarWhilePointRole::BodyScopeExit,
    )?;
    state.holder_events.outgoing(back.id, loop_test.id)?;
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let condition =
        context.append_basic_block(function, &format!("while.cond.{}", state.next_block));
    state.next_block += 1;
    let body = context.append_basic_block(function, &format!("while.body.{}", state.next_block));
    state.next_block += 1;
    let exit = context.append_basic_block(function, &format!("while.exit.{}", state.next_block));
    state.next_block += 1;
    state
        .builder
        .build_unconditional_branch(condition)
        .map_err(builder_error)?;
    state.builder.position_at_end(condition);
    let value = take_basic(emit_project_expression(
        context,
        state,
        &expression.condition,
        module,
        modules,
    )?)?
    .into_int_value();
    state
        .builder
        .build_conditional_branch(value, body, exit)
        .map_err(builder_error)?;
    state.builder.position_at_end(body);
    let _ = emit_project_block(context, state, &expression.body, module, modules)?;
    state
        .builder
        .build_unconditional_branch(condition)
        .map_err(builder_error)?;
    state.builder.position_at_end(exit);
    Ok(EmitValue::Unit)
}

mod expression;
use expression::{
    emit_expression, emit_if_value, emit_project_expression, emit_project_typed_expression,
};
mod return_memory;
use return_memory::{
    emit_automatic_return_result, emit_return, materialize_fresh_return_outputs,
    materialize_return_branch, record_forwarded_return_owner, transfer_automatic_return_result,
};
fn project_function_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    let mut symbol = String::from("wosy_fn");
    for component in [
        source.package.as_str(),
        source.path.as_str(),
        source.revision.as_str(),
        name,
    ] {
        symbol.push_str("__");
        for byte in component.as_bytes() {
            write!(symbol, "{byte:02x}").expect("writing to a String cannot fail");
        }
    }
    symbol
}

fn llvm_extern_identity(
    source: &wosy_syntax::SourceIdentity,
    binding: &str,
    import_module: &str,
    import_name: &str,
    signature: &ScalarType,
) -> LlvmExternIdentity {
    let internal_name = encoded_llvm_name(
        "wosy_extern",
        [
            source.package.as_str(),
            source.path.as_str(),
            source.revision.as_str(),
            binding,
            import_module,
            import_name,
        ],
    );
    LlvmExternIdentity {
        internal_name,
        import_module: import_module.to_owned(),
        import_name: match import_name.strip_prefix('_') {
            Some(private_name) => private_name.to_owned(),
            None => import_name.to_owned(),
        },
        signature: signature.clone(),
    }
}

fn project_extern_lookup_key(
    source: &wosy_syntax::SourceIdentity,
    binding: &str,
    import_module: &str,
    name: &str,
) -> String {
    encoded_llvm_name(
        "wosy_extern_lookup",
        [
            source.package.as_str(),
            source.path.as_str(),
            source.revision.as_str(),
            binding,
            import_module,
            name,
        ],
    )
}

fn encoded_llvm_name<'a>(prefix: &str, components: impl IntoIterator<Item = &'a str>) -> String {
    let mut symbol = prefix.to_owned();
    for component in components {
        symbol.push_str("__");
        for byte in component.as_bytes() {
            write!(symbol, "{byte:02x}").expect("writing to a String cannot fail");
        }
    }
    symbol
}

fn private_literal_symbol(prefix: &str, literal: &wosy_syntax::SourceSpan) -> String {
    encoded_llvm_name(
        prefix,
        [
            literal.source.project.as_str(),
            literal.source.package.as_str(),
            literal.source.path.as_str(),
            literal.source.revision.as_str(),
            &literal.range.start.to_string(),
            &literal.range.end.to_string(),
        ],
    )
}

fn generic_specializations<'a>(
    items: &[ScalarItem],
    function: impl Fn(&str) -> Option<&'a ScalarFunction>,
    symbol: impl Fn(&str) -> String,
) -> Result<Vec<(String, ScalarFunction, String)>, String> {
    let calls = generic_calls_in_items(items)
        .into_iter()
        .filter_map(|(receiver, name, arguments)| receiver.is_none().then_some((name, arguments)));
    generic_specializations_from_calls(calls, function, symbol)
}

fn generic_calls_in_items(items: &[ScalarItem]) -> Vec<(Option<String>, String, Vec<ScalarType>)> {
    let mut calls = Vec::new();
    for item in items {
        match item {
            ScalarItem::Binding(binding) => collect_generic_calls(&binding.value, &mut calls),
            ScalarItem::Executable(item) => collect_generic_calls_in_item(item, &mut calls),
            ScalarItem::Function(function) if function.generic_parameters.is_empty() => {
                collect_generic_calls_in_block(&function.body, &mut calls);
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Function(_) => {}
        }
    }
    calls
}

fn generic_specializations_from_calls<'a>(
    calls: impl IntoIterator<Item = (String, Vec<ScalarType>)>,
    function: impl Fn(&str) -> Option<&'a ScalarFunction>,
    symbol: impl Fn(&str) -> String,
) -> Result<Vec<(String, ScalarFunction, String)>, String> {
    let mut specializations = BTreeMap::new();
    for (name, arguments) in calls {
        let Some(function) = function(&name) else {
            continue;
        };
        if function.generic_parameters.is_empty() {
            continue;
        }
        if function.generic_parameters.len() != arguments.len() {
            return Err("validated generic call has invalid type argument arity".to_owned());
        }
        let signature = substitute_generic_signature(function, &arguments);
        let base = symbol(&name);
        let lookup = generic_specialization_key(&base, &arguments);
        let name = generic_specialization_name(&base, &arguments);
        specializations.entry(lookup).or_insert_with(|| {
            let mut function = function.clone();
            let substitutions = function
                .generic_parameters
                .iter()
                .zip(&arguments)
                .map(|(parameter, argument)| (parameter.name.as_str(), argument))
                .collect::<BTreeMap<_, _>>();
            substitute_generic_block(&mut function.body, &substitutions);
            function.signature = signature;
            function.generic_parameters.clear();
            (function, name)
        });
    }
    Ok(specializations
        .into_iter()
        .map(|(lookup, (function, name))| (lookup, function, name))
        .collect())
}

fn selected_overload_specializations(
    items: &[ScalarItem],
    symbol: impl Fn(&str) -> String,
    selections: impl IntoIterator<Item = (String, ScalarOverloadSelection)>,
) -> Vec<(String, ScalarFunction, String)> {
    let overloads = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                Some((function.name.as_str(), function))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut selections = selections.into_iter().collect::<Vec<_>>();
    let mut specializations = BTreeMap::new();
    for (name, overload) in &overloads {
        for (arm_index, arm) in overload.overload_arms.iter().enumerate() {
            if arm.generic_parameters.is_empty() {
                selections.push((
                    (*name).to_owned(),
                    ScalarOverloadSelection {
                        arm_index,
                        substitutions: BTreeMap::new(),
                    },
                ));
            }
        }
    }
    for (name, selection) in selections {
        let Some(overload) = overloads.get(name.as_str()) else {
            continue;
        };
        let Some(arm) = overload.overload_arms.get(selection.arm_index) else {
            continue;
        };
        let base = symbol(&overload.name);
        let arguments = arm
            .generic_parameters
            .iter()
            .map(|parameter| selection.substitutions[&parameter.name].clone())
            .collect::<Vec<_>>();
        let mut arm = arm.clone();
        let substitutions = arm
            .generic_parameters
            .iter()
            .zip(&arguments)
            .map(|(parameter, argument)| (parameter.name.as_str(), argument))
            .collect::<BTreeMap<_, _>>();
        substitute_generic_block(&mut arm.body, &substitutions);
        arm.signature = substitute_generic_signature(&arm, &arguments);
        arm.generic_parameters.clear();
        let lookup = selected_overload_lookup_key(&base, &selection);
        let name = generic_specialization_name(
            &encoded_llvm_name(
                "wosy_overload",
                [base.as_str(), &selection.arm_index.to_string()],
            ),
            &arguments,
        );
        specializations.entry(lookup).or_insert((arm, name));
    }
    specializations
        .into_iter()
        .map(|(lookup, (function, name))| (lookup, function, name))
        .collect()
}

fn selected_overload_selections(
    items: &[ScalarItem],
) -> Vec<(Option<String>, String, ScalarOverloadSelection)> {
    let mut selections = Vec::new();
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                collect_selected_overloads(&binding.value, &mut selections)
            }
            ScalarItem::Function(function) => {
                collect_selected_overloads_in_block(&function.body, &mut selections)
            }
            ScalarItem::Executable(item) => {
                collect_selected_overloads_in_item(item, &mut selections)
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
        }
    }
    selections
}

fn collect_selected_overloads(
    expression: &ScalarExpression,
    selections: &mut Vec<(Option<String>, String, ScalarOverloadSelection)>,
) {
    match expression {
        ScalarExpression::Assignment(assignment) => {
            for value in &assignment.values {
                collect_selected_overloads(value, selections);
            }
            for target in &assignment.targets {
                collect_selected_overloads_in_place(&target.place, selections);
            }
        }
        ScalarExpression::FunctionLiteral { body, .. } => {
            collect_selected_overloads_in_block(body, selections)
        }
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            overload_selection: Some(selection),
            ..
        } => {
            selections.push((receiver.clone(), name.clone(), selection.clone()));
            for argument in arguments {
                collect_selected_overloads(argument, selections);
            }
        }
        ScalarExpression::Call { arguments, .. } => {
            for argument in arguments {
                collect_selected_overloads(argument, selections);
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            collect_selected_overloads(left, selections);
            collect_selected_overloads(right, selections);
        }
        ScalarExpression::Unary { operand, .. } => collect_selected_overloads(operand, selections),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_selected_overloads(condition, selections);
            collect_selected_overloads_in_block(then_branch, selections);
            collect_selected_overloads_in_block(else_branch, selections);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            collect_selected_overloads(condition, selections);
            collect_selected_overloads_in_block(then_branch, selections);
        }
        ScalarExpression::Block(block) => collect_selected_overloads_in_block(block, selections),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                collect_selected_overloads(&field.value, selections);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                collect_selected_overloads(element, selections);
            }
        }
        ScalarExpression::Dereference { place, .. } => {
            collect_selected_overloads_in_place(place, selections);
        }
        ScalarExpression::IndexedRead { place, .. } => {
            collect_selected_overloads_in_place(place, selections);
        }
        ScalarExpression::RawAddress { .. }
        | ScalarExpression::CheckedAddress { .. }
        | ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

fn collect_selected_overloads_in_place(
    place: &crate::ScalarPlace,
    selections: &mut Vec<(Option<String>, String, ScalarOverloadSelection)>,
) {
    match place {
        crate::ScalarPlace::Name { .. } => {}
        crate::ScalarPlace::Dereference { pointer, .. } => {
            collect_selected_overloads(pointer, selections)
        }
        crate::ScalarPlace::Field { base, .. } => {
            collect_selected_overloads_in_place(base, selections)
        }
        crate::ScalarPlace::Index { base, index, .. } => {
            collect_selected_overloads_in_place(base, selections);
            collect_selected_overloads(index, selections);
        }
    }
}

fn collect_selected_overloads_in_block(
    block: &ScalarBlock,
    selections: &mut Vec<(Option<String>, String, ScalarOverloadSelection)>,
) {
    for item in &block.items {
        collect_selected_overloads_in_item(item, selections);
    }
    for output in &block.final_output_values {
        collect_selected_overloads(&output.value, selections);
    }
}

fn collect_selected_overloads_in_item(
    item: &ScalarBlockItem,
    selections: &mut Vec<(Option<String>, String, ScalarOverloadSelection)>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            collect_selected_overloads(&binding.value, selections)
        }
        ScalarBlockItem::Expression(expression) => {
            collect_selected_overloads(expression, selections)
        }
        ScalarBlockItem::Assignment(assignment) => {
            for value in &assignment.values {
                collect_selected_overloads(value, selections);
            }
            for target in &assignment.targets {
                collect_selected_overloads_in_place(&target.place, selections);
            }
        }
        ScalarBlockItem::While(while_expression) => {
            collect_selected_overloads(&while_expression.condition, selections);
            collect_selected_overloads_in_block(&while_expression.body, selections);
        }
    }
}

fn selected_overload_lookup_key(name: &str, selection: &ScalarOverloadSelection) -> String {
    let substitutions = selection
        .substitutions
        .values()
        .map(generic_type_name)
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}<arm:{},{}>", selection.arm_index, substitutions)
}

fn collect_generic_calls(
    expression: &ScalarExpression,
    calls: &mut Vec<(Option<String>, String, Vec<ScalarType>)>,
) {
    match expression {
        ScalarExpression::Assignment(assignment) => {
            for value in &assignment.values {
                collect_generic_calls(value, calls);
            }
            for target in &assignment.targets {
                collect_generic_calls_in_place(&target.place, calls);
            }
        }
        ScalarExpression::FunctionLiteral { body, .. } => {
            collect_generic_calls_in_block(body, calls)
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            ..
        } => {
            calls.push((
                receiver.clone(),
                name.clone(),
                type_arguments
                    .iter()
                    .map(|argument| argument.ty.clone())
                    .collect(),
            ));
            for argument in arguments {
                collect_generic_calls(argument, calls);
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            collect_generic_calls(left, calls);
            collect_generic_calls(right, calls);
        }
        ScalarExpression::Unary { operand, .. } => collect_generic_calls(operand, calls),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_generic_calls(condition, calls);
            collect_generic_calls_in_block(then_branch, calls);
            collect_generic_calls_in_block(else_branch, calls);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            collect_generic_calls(condition, calls);
            collect_generic_calls_in_block(then_branch, calls);
        }
        ScalarExpression::Block(block) => collect_generic_calls_in_block(block, calls),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                collect_generic_calls(&field.value, calls);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                collect_generic_calls(element, calls);
            }
        }
        ScalarExpression::Dereference { place, .. } => {
            collect_generic_calls_in_place(place, calls);
        }
        ScalarExpression::IndexedRead { place, .. } => {
            collect_generic_calls_in_place(place, calls);
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. }
        | ScalarExpression::RawAddress { .. }
        | ScalarExpression::CheckedAddress { .. } => {}
    }
}

fn collect_generic_calls_in_place(
    place: &crate::ScalarPlace,
    calls: &mut Vec<(Option<String>, String, Vec<ScalarType>)>,
) {
    match place {
        crate::ScalarPlace::Name { .. } => {}
        crate::ScalarPlace::Dereference { pointer, .. } => collect_generic_calls(pointer, calls),
        crate::ScalarPlace::Field { base, .. } => collect_generic_calls_in_place(base, calls),
        crate::ScalarPlace::Index { base, index, .. } => {
            collect_generic_calls_in_place(base, calls);
            collect_generic_calls(index, calls);
        }
    }
}

fn collect_generic_calls_in_block(
    block: &ScalarBlock,
    calls: &mut Vec<(Option<String>, String, Vec<ScalarType>)>,
) {
    for item in &block.items {
        collect_generic_calls_in_item(item, calls);
    }
    for output in &block.final_output_values {
        collect_generic_calls(&output.value, calls);
    }
}

fn collect_generic_calls_in_item(
    item: &ScalarBlockItem,
    calls: &mut Vec<(Option<String>, String, Vec<ScalarType>)>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => collect_generic_calls(&binding.value, calls),
        ScalarBlockItem::Expression(expression) => collect_generic_calls(expression, calls),
        ScalarBlockItem::Assignment(assignment) => {
            for value in &assignment.values {
                collect_generic_calls(value, calls);
            }
            for target in &assignment.targets {
                collect_generic_calls_in_place(&target.place, calls);
            }
        }
        ScalarBlockItem::While(while_expression) => {
            collect_generic_calls(&while_expression.condition, calls);
            collect_generic_calls_in_block(&while_expression.body, calls);
        }
    }
}

fn substitute_generic_signature(function: &ScalarFunction, arguments: &[ScalarType]) -> ScalarType {
    let substitutions = function
        .generic_parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.name.as_str(), argument))
        .collect::<BTreeMap<_, _>>();
    substitute_generic_type(&function.signature, &substitutions)
}

fn substitute_generic_block(block: &mut ScalarBlock, substitutions: &BTreeMap<&str, &ScalarType>) {
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                binding.declared_type =
                    substitute_generic_type(&binding.declared_type, substitutions);
                for receiver in &mut binding.receivers {
                    receiver.ty = substitute_generic_type(&receiver.ty, substitutions);
                }
                for output in &mut binding.output_sequence.outputs {
                    output.ty = substitute_generic_type(&output.ty, substitutions);
                }
                for output in &mut binding.output_values {
                    output.ty = substitute_generic_type(&output.ty, substitutions);
                    substitute_generic_expression(&mut output.value, substitutions);
                }
                substitute_generic_expression(&mut binding.value, substitutions);
            }
            ScalarBlockItem::Expression(expression) => {
                substitute_generic_expression(expression, substitutions)
            }
            ScalarBlockItem::Assignment(assignment) => {
                for target in &mut assignment.targets {
                    substitute_generic_place(&mut target.place, substitutions);
                }
                for value in &mut assignment.values {
                    substitute_generic_expression(value, substitutions);
                }
            }
            ScalarBlockItem::While(loop_expression) => {
                substitute_generic_expression(&mut loop_expression.condition, substitutions);
                substitute_generic_block(&mut loop_expression.body, substitutions);
            }
        }
    }
    for output in &mut block.final_output_values {
        output.ty = substitute_generic_type(&output.ty, substitutions);
        substitute_generic_expression(&mut output.value, substitutions);
    }
}

fn substitute_generic_expression(
    expression: &mut ScalarExpression,
    substitutions: &BTreeMap<&str, &ScalarType>,
) {
    match expression {
        ScalarExpression::Assignment(assignment) => {
            for target in &mut assignment.targets {
                substitute_generic_place(&mut target.place, substitutions);
            }
            for value in &mut assignment.values {
                substitute_generic_expression(value, substitutions);
            }
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. }
        | ScalarExpression::IndexedRead { place, .. } => {
            substitute_generic_place(place, substitutions)
        }
        ScalarExpression::Call {
            type_arguments,
            arguments,
            ..
        } => {
            for argument in type_arguments {
                argument.ty = substitute_generic_type(&argument.ty, substitutions);
            }
            for argument in arguments {
                substitute_generic_expression(argument, substitutions);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            substitute_generic_expression(condition, substitutions);
            substitute_generic_block(then_branch, substitutions);
            substitute_generic_block(else_branch, substitutions);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            substitute_generic_expression(condition, substitutions);
            substitute_generic_block(then_branch, substitutions);
        }
        ScalarExpression::Block(block) | ScalarExpression::FunctionLiteral { body: block, .. } => {
            substitute_generic_block(block, substitutions);
        }
        ScalarExpression::Binary { left, right, .. } => {
            substitute_generic_expression(left, substitutions);
            substitute_generic_expression(right, substitutions);
        }
        ScalarExpression::Unary { operand, .. } => {
            substitute_generic_expression(operand, substitutions)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                substitute_generic_expression(&mut field.value, substitutions);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                substitute_generic_expression(element, substitutions);
            }
        }
        _ => {}
    }
}

fn substitute_generic_place(
    place: &mut crate::ScalarPlace,
    substitutions: &BTreeMap<&str, &ScalarType>,
) {
    match place {
        crate::ScalarPlace::Name { .. } => {}
        crate::ScalarPlace::Field { base, .. } => substitute_generic_place(base, substitutions),
        crate::ScalarPlace::Dereference { pointer, .. } => {
            substitute_generic_expression(pointer, substitutions)
        }
        crate::ScalarPlace::Index { base, index, .. } => {
            substitute_generic_place(base, substitutions);
            substitute_generic_expression(index, substitutions);
        }
    }
}

fn substitute_generic_type(
    ty: &ScalarType,
    substitutions: &BTreeMap<&str, &ScalarType>,
) -> ScalarType {
    match ty {
        ScalarType::Named { name, .. } => substitutions
            .get(name.as_str())
            .map_or_else(|| ty.clone(), |replacement| (*replacement).clone()),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(substitute_generic_type(inner, substitutions)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(substitute_generic_type(inner, substitutions)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(substitute_generic_type(element, substitutions)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::RuntimeArray { element, span } => ScalarType::RuntimeArray {
            element: Box::new(substitute_generic_type(element, substitutions)),
            span: *span,
        },
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: crate::ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| crate::ScalarOutput {
                        ty: substitute_generic_type(&output.ty, substitutions),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| substitute_generic_type(parameter, substitutions))
                .collect(),
        },
        _ => ty.clone(),
    }
}

fn specialization_lookup_key(
    name: &str,
    arguments: &[crate::scalar::ScalarTypeArgument],
) -> String {
    if arguments.is_empty() {
        name.to_owned()
    } else {
        generic_specialization_key(
            name,
            &arguments
                .iter()
                .map(|argument| argument.ty.clone())
                .collect::<Vec<_>>(),
        )
    }
}

fn generic_specialization_key(name: &str, arguments: &[ScalarType]) -> String {
    let tuple = arguments
        .iter()
        .map(generic_type_name)
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}<{tuple}>")
}

fn generic_specialization_name(name: &str, arguments: &[ScalarType]) -> String {
    let tuple = arguments
        .iter()
        .map(generic_type_name)
        .collect::<Vec<_>>()
        .join(",");
    encoded_llvm_name("wosy_generic", [name, tuple.as_str()])
}

fn generic_type_name(ty: &ScalarType) -> String {
    match ty {
        ScalarType::Unit => "unit".into(),
        ScalarType::Bool => "bool".into(),
        ScalarType::I8 => "i8".into(),
        ScalarType::I16 => "i16".into(),
        ScalarType::I32 => "i32".into(),
        ScalarType::I64 => "i64".into(),
        ScalarType::I128 => "i128".into(),
        ScalarType::U8 => "u8".into(),
        ScalarType::U16 => "u16".into(),
        ScalarType::U32 => "u32".into(),
        ScalarType::U64 => "u64".into(),
        ScalarType::U128 => "u128".into(),
        ScalarType::F32 => "f32".into(),
        ScalarType::F64 => "f64".into(),
        ScalarType::Char => "char".into(),
        ScalarType::ArtifactId => "artifact_id".into(),
        ScalarType::RawPointer(inner) => format!("ptr({})", generic_type_name(inner)),
        ScalarType::CheckedReference { mutability, inner } => format!(
            "checked_{}({})",
            match mutability {
                crate::scalar::ScalarReferenceMutability::Shared => "shared",
                crate::scalar::ScalarReferenceMutability::Mutable => "mutable",
            },
            generic_type_name(inner)
        ),
        ScalarType::Array {
            element, length, ..
        } => format!("array({};{length})", generic_type_name(element)),
        ScalarType::RuntimeArray { element, .. } => {
            format!("array({})", generic_type_name(element))
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => format!(
            "fn({})->({})",
            parameters
                .iter()
                .map(generic_type_name)
                .collect::<Vec<_>>()
                .join(","),
            outputs
                .outputs
                .iter()
                .map(|output| generic_type_name(&output.ty))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ScalarType::Named { name, .. } => format!("named({name})"),
        ScalarType::Qualified {
            receiver, member, ..
        } => format!("qualified({receiver}.{member})"),
        ScalarType::Struct(id) => format!(
            "struct({}:{}:{}:{})",
            id.source.package, id.source.path, id.source.revision, id.index
        ),
        ScalarType::Enum(id) => format!(
            "enum({}:{}:{}:{})",
            id.source.package, id.source.path, id.source.revision, id.index
        ),
        ScalarType::Error => "error".into(),
    }
}

fn project_global_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    project_function_name(source, &format!("global_{name}"))
}

fn project_runtime_array_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    project_global_name(source, name)
}

fn project_namespace_target<'a>(
    module: &'a ScalarModule,
    modules: &'a [&ScalarModule],
    receiver: &str,
) -> Result<&'a ScalarModule, String> {
    let namespace = module
        .namespace_bindings
        .iter()
        .find(|namespace| namespace.binding == receiver)
        .ok_or_else(|| format!("unknown namespace binding {receiver}"))?;
    modules
        .iter()
        .find(|candidate| candidate.source == namespace.target)
        .copied()
        .ok_or_else(|| format!("missing namespace target {}", namespace.target.path))
}

fn project_member_global<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    target: &ScalarModule,
    name: &str,
) -> Result<(Option<GlobalValue<'ctx>>, ScalarType), String> {
    let binding = target.items.iter().find_map(|item| match item {
        ScalarItem::Binding(binding) if binding.name == name => Some(binding),
        ScalarItem::Binding(_)
        | ScalarItem::Extern(_)
        | ScalarItem::Namespace(_)
        | ScalarItem::Function(_)
        | ScalarItem::Executable(_) => None,
    });
    let binding = binding.ok_or_else(|| format!("unknown LLVM member {name}"))?;
    let global = state
        .all_globals
        .get(&project_global_name(&target.source, name))
        .map(|(global, _)| *global);
    if binding.declared_type != ScalarType::Unit && global.is_none() {
        return Err(format!("unknown LLVM member storage {name}"));
    }
    Ok((global, binding.declared_type.clone()))
}

fn insert_unit_values<'ctx>(values: &mut BTreeMap<String, EmitValue<'ctx>>, items: &[ScalarItem]) {
    for item in items {
        if let ScalarItem::Binding(binding) = item {
            if binding.declared_type == ScalarType::Unit {
                values.insert(binding.name.clone(), EmitValue::Unit);
            }
        }
    }
}

fn module_globals<'ctx>(
    module: &ScalarModule,
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
) -> BTreeMap<String, (GlobalValue<'ctx>, ScalarType)> {
    module
        .items
        .iter()
        .flat_map(|item| match item {
            ScalarItem::Binding(binding) => std::iter::once(binding.name.clone())
                .chain(
                    binding
                        .receivers
                        .iter()
                        .map(|receiver| receiver.name.clone()),
                )
                .filter_map(|name| {
                    globals
                        .get(&project_global_name(&module.source, &name))
                        .cloned()
                        .map(|global| (name, global))
                })
                .collect::<Vec<_>>(),
            ScalarItem::Namespace(_)
            | ScalarItem::Extern(_)
            | ScalarItem::Function(_)
            | ScalarItem::Executable(_) => Vec::new(),
        })
        .collect()
}

fn register_global_holder_storage<'ctx>(
    holder_events: &mut holder_events::HolderEvents<'ctx>,
    emitting_source: &wosy_syntax::SourceIdentity,
    own_items: &[ScalarItem],
    own_globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    foreign_modules: &[&ScalarModule],
    all_globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
) -> Result<(), String> {
    for item in own_items {
        let ScalarItem::Binding(binding) = item else {
            continue;
        };
        let mut names = vec![(binding.name.clone(), binding.name_span)];
        names.extend(
            binding
                .receivers
                .iter()
                .map(|receiver| (receiver.name.clone(), receiver.name_span)),
        );
        for (name, name_span) in names {
            let Some((global, _)) = own_globals.get(&name) else {
                continue;
            };
            holder_events.bindings.insert(
                ReferenceBindingId {
                    source: emitting_source.clone(),
                    function_span: wosy_syntax::ByteSpan::new(0, 0),
                    declaration_span: name_span,
                    block_span: wosy_syntax::ByteSpan::new(0, 0),
                    kind: ReferenceBindingKind::Declared,
                },
                global.as_pointer_value(),
            );
        }
    }
    for module in foreign_modules {
        if module.source == *emitting_source {
            continue;
        }
        for item in &module.items {
            let ScalarItem::Binding(binding) = item else {
                continue;
            };
            let mut names = vec![(binding.name.clone(), binding.name_span)];
            names.extend(
                binding
                    .receivers
                    .iter()
                    .map(|receiver| (receiver.name.clone(), receiver.name_span)),
            );
            for (name, name_span) in names {
                let Some((global, _)) =
                    all_globals.get(&project_global_name(&module.source, &name))
                else {
                    continue;
                };
                holder_events.bindings.insert(
                    ReferenceBindingId {
                        source: module.source.clone(),
                        function_span: wosy_syntax::ByteSpan::new(0, 0),
                        declaration_span: name_span,
                        block_span: wosy_syntax::ByteSpan::new(0, 0),
                        kind: ReferenceBindingKind::Declared,
                    },
                    global.as_pointer_value(),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
