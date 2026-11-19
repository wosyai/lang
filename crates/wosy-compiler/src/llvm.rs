use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

use crate::scalar::control::{
    module_initialization_region, CallableBodyCallNodes, InitializerCallNodes,
    ModuleInitializationCallTarget, ModuleInitializationRegion, ModuleInitializationStepKind,
    NamespaceTarget,
};
use crate::scalar::layout::ScalarTargetLayouts;
use crate::scalar::{
    CfgPointKind, CoreOperationId, InvalidationCandidate, InvalidationInputs, ReferenceBindingKind,
    ReferenceOrigin, ReleaseTarget, ResolvedCallTarget, ScalarAllocationIdentity,
    ScalarAutomaticReturnResult, ScalarAutomaticReturnResultState, ScalarBindingOutputOrigin,
    ScalarFieldReference, ScalarOverloadSelection, ScalarReferenceCfg,
};
use crate::{
    BinaryOperator, ScalarAssignment, ScalarBlock, ScalarBlockItem, ScalarBuildValidation,
    ScalarExpression, ScalarFunction, ScalarItem, ScalarModule, ScalarProject,
    ScalarProjectValidation, ScalarStruct, ScalarTargetLayout, ScalarType, ScalarValidation,
    ScalarWhile,
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
    initializer_nodes: Option<&'module InitializerCallNodes<'module>>,
    body_call_nodes: Option<&'module CallableBodyCallNodes<'module>>,
    initializer_symbols: Option<&'module InitializerSymbols>,
    builder: &'ctx Builder<'ctx>,
    module: &'module Module<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    return_bindings: BTreeMap<String, ScalarExpression>,
    fresh_return_outputs: BTreeSet<usize>,
    branch_return_outputs:
        BTreeMap<(u32, u32), (BTreeSet<usize>, BTreeSet<usize>, BTreeMap<usize, String>)>,
    joined_return_bindings: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    joined_assignment_owners: BTreeMap<(u32, u32), CheckedAssignmentOwner>,
    return_output_types: Vec<ScalarType>,
    checked_call_owners: BTreeMap<(u32, u32, usize), CheckedCallOwner>,
    checked_owner_flags: BTreeMap<String, inkwell::values::IntValue<'ctx>>,
    call_owner_flags: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    return_owner_flags: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    return_owner_slots: BTreeMap<usize, PointerValue<'ctx>>,
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
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
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

fn mutable_runtime_allocations(reference_cfg: &ScalarReferenceCfg) -> BTreeSet<(u32, u32)> {
    let mut result = BTreeSet::new();
    for point in &reference_cfg.points {
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

fn resolved_invalidations(
    reference_cfg: &ScalarReferenceCfg,
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
    for point in &reference_cfg.points {
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
        if !reference_cfg.points.iter().any(|call| {
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
enum RuntimeArrayResultProvenance {
    LocalOwner,
    ParameterAlias,
    FixedWidening,
    FreshCall,
}

impl RuntimeArrayResultProvenance {
    fn owns(self) -> bool {
        matches!(self, Self::LocalOwner | Self::FreshCall)
    }
}

#[derive(Clone)]
struct RuntimeArrayAssignmentProvenance {
    result: RuntimeArrayResultProvenance,
    source_name: Option<String>,
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
    let program = &validation.program;
    let initialization_region = module_initialization_region(&program.source, &program.items, &[]);
    emit_single_module_llvm(
        &program.source,
        &program.items,
        &initialization_region,
        &program.structs,
        &program.layouts,
        program.target_layout,
        GenericInitializerInput::RawItems,
        &|_, _, function| Some(&function.reference_cfg),
    )
}

pub fn emit_scalar_build_single_module_llvm(
    validation: &ScalarBuildValidation<'_>,
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics().is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".into());
    }
    let prepared = validation.prepared();
    let modules = &prepared.project().modules;
    let [module] = modules.as_slice() else {
        return Err(format!(
            "single-module LLVM emission requires exactly one prepared module, found {}",
            modules.len()
        ));
    };
    let initialization_region = prepared.initialization_region(0).ok_or_else(|| {
        format!(
            "internal error: missing module initialization region for {:?} (module 0)",
            module.source,
        )
    })?;
    emit_single_module_llvm(
        &module.source,
        &module.items,
        initialization_region,
        &module.structs,
        &module.layouts,
        module.target_layout,
        GenericInitializerInput::PreparedRows(prepared),
        &|item, arm, _| validation.reference_cfg(0, item, arm),
    )
}

fn emit_single_module_llvm<'a>(
    source: &'a wosy_syntax::SourceIdentity,
    items: &'a [ScalarItem],
    initialization_region: &'a ModuleInitializationRegion,
    structs: &[ScalarStruct],
    layouts: &ScalarTargetLayouts,
    target_layout: ScalarTargetLayout,
    generic_input: GenericInitializerInput<'a>,
    reference_cfg: &dyn Fn(
        usize,
        Option<usize>,
        &'a ScalarFunction,
    ) -> Option<&'a ScalarReferenceCfg>,
) -> Result<LlvmPartition, String> {
    let initializer_nodes = match generic_input {
        GenericInitializerInput::RawItems => None,
        GenericInitializerInput::PreparedRows(_) => {
            Some(initialization_region.bind_call_nodes(items)?)
        }
    };
    let require_cfg = |item_ordinal, arm_index, function: &'a ScalarFunction| {
        reference_cfg(item_ordinal, arm_index, function).ok_or_else(|| {
            let span = function.span;
            format!(
                "internal error: missing reference CFG for {} in {source:?} at {}..{} (module 0, item {item_ordinal}, arm {arm_index:?})",
                function.name, span.start, span.end,
            )
        })
    };
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&source.path));
    let builder = context.create_builder();
    let layouts = crate::scalar::layout::ScalarLayoutIndex::program(layouts);
    let structs = structs.iter().collect::<Vec<_>>();
    let mut globals = BTreeMap::new();
    for item in items {
        if let ScalarItem::Binding(binding) = item {
            for (name, ty) in binding_receivers(binding).iter() {
                if *ty == ScalarType::Unit {
                    continue;
                }
                let llvm_ty = storage_type(&context, ty, &layouts, target_layout)?;
                let global = module.add_global(llvm_ty, None, name);
                global.set_linkage(Linkage::Internal);
                global.set_initializer(&llvm_ty.const_zero());
                globals.insert(name.clone(), (global, ty.clone()));
            }
        }
    }
    let mut signatures = items
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
    let mut initializer_symbols = InitializerSymbols::default();
    let mut definitions = Vec::new();
    for (item_ordinal, item) in items.iter().enumerate() {
        if initializer_nodes.is_some() {
            if let ScalarItem::Binding(binding) = item {
                initializer_symbols.callable_binding(source, binding);
            }
        }
        let ScalarItem::Function(function) = item else {
            continue;
        };
        if initializer_nodes.is_some() {
            initializer_symbols.function(source, function, function.name.clone());
        }
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
            continue;
        }
        let cfg = require_cfg(item_ordinal, None, function)?;
        let value = module.add_function(
            &function.name,
            definition_function_type(&context, function, cfg, target_layout)?,
            None,
        );
        for (index, name) in function.parameters.iter().enumerate() {
            if let Some(parameter) = value.get_nth_param(index as u32) {
                parameter.set_name(name);
            }
        }
        functions.insert(function.name.clone(), value);
        call_targets.insert(function.name.clone(), function.name.clone());
        definitions.push((function, cfg, function.name.as_str(), item_ordinal, None));
    }
    let ordinary_count = definitions.len();
    let specializations = match generic_input {
        GenericInitializerInput::RawItems => generic_specializations(items, str::to_owned)?,
        GenericInitializerInput::PreparedRows(_) => prepared_generic_specializations(
            std::iter::once((0, source, items, initialization_region)),
            |_, name| name.to_owned(),
        )?
        .remove(&0)
        .expect("single prepared module has a specialization set"),
    };
    let mut prepared_overloads = prepared_initializer_overloads(
        initializer_nodes.iter().map(|_| initialization_region),
        &initializer_symbols,
    )?;
    collect_prepared_body_overload_selections(
        generic_input,
        0,
        items,
        &initializer_symbols,
        &mut prepared_overloads,
    )?;
    let overloads = selected_overload_specializations(
        items,
        str::to_owned,
        selected_overload_selections(items, generic_input)
            .into_iter()
            .filter_map(|(receiver, name, selection)| {
                receiver.is_none().then_some((name, selection))
            })
            .chain(prepared_overloads.remove(source).into_iter().flatten()),
    );
    for (
        lookup,
        LlvmSpecialization {
            function,
            name,
            template,
            item_ordinal,
            arm_index,
        },
    ) in specializations.iter().chain(&overloads)
    {
        let cfg = require_cfg(*item_ordinal, *arm_index, template)?;
        let value = module.add_function(
            name,
            definition_function_type(&context, function, cfg, target_layout)?,
            None,
        );
        for (index, parameter) in function.parameters.iter().enumerate() {
            if let Some(argument) = value.get_nth_param(index as u32) {
                argument.set_name(parameter);
            }
        }
        call_targets.insert(lookup.clone(), name.clone());
        signatures.insert(name.clone(), function.signature.clone());
        functions.insert(name.clone(), value);
        definitions.push((
            function.as_ref(),
            cfg,
            name.as_str(),
            *item_ordinal,
            *arm_index,
        ));
    }
    for item in items {
        if let ScalarItem::Extern(extern_decl) = item {
            for function in &extern_decl.functions {
                let key = format!("{}.{}", extern_decl.binding, function.name);
                let crate::scalar::ScalarExternModule::Valid(import_module) =
                    &extern_decl.actual_module
                else {
                    continue;
                };
                let identity = llvm_extern_identity(
                    source,
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
                if initializer_nodes.is_some() {
                    initializer_symbols.external(
                        source,
                        function.name_span,
                        &identity.internal_name,
                    );
                }
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
    let mut runtime_array_results = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for (item_ordinal, item) in items.iter().enumerate() {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        if let Some(target) = call_targets.get(&function.name) {
            let cfg = require_cfg(item_ordinal, None, function)?;
            insert_function_result_provenance(
                &mut runtime_array_results,
                &mut dynamic_owner_outputs,
                target,
                function,
                cfg,
            );
        }
    }
    for &(function, cfg, name, _, _) in &definitions[ordinary_count..] {
        insert_function_result_provenance(
            &mut runtime_array_results,
            &mut dynamic_owner_outputs,
            name,
            function,
            cfg,
        );
    }
    for (function, cfg, name, item, arm) in definitions {
        let nodes = generic_input.bind_body(0, item, arm, &function.body)?;
        emit_function(
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
            &globals,
            function,
            cfg,
            nodes.as_ref(),
            &initializer_symbols,
            name,
            items,
            &module,
            &structs,
            &layouts,
            target_layout,
        )?;
    }
    emit_main(
        &context,
        &builder,
        &functions,
        &call_targets,
        &signatures,
        &runtime_array_results,
        &dynamic_owner_outputs,
        &globals,
        items,
        initialization_region,
        initializer_nodes.as_ref(),
        &initializer_symbols,
        &module,
        &structs,
        &layouts,
        target_layout,
    )?;
    finish_partition(
        ManuallyDrop::into_inner(module),
        source.path.clone(),
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
    let regions = validation
        .project
        .modules
        .iter()
        .map(|module| {
            module_initialization_region(&module.source, &module.items, &module.namespace_bindings)
        })
        .collect::<Vec<_>>();
    emit_project_llvm(
        &validation.project,
        &|module| regions.get(module),
        GenericInitializerInput::RawItems,
        &|_, _, _, function| Some(&function.reference_cfg),
    )
}

pub fn emit_scalar_build_project_llvm(
    validation: &ScalarBuildValidation<'_>,
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics().is_empty() {
        return Err("cannot emit LLVM for an invalid scalar project".into());
    }
    let prepared = validation.prepared();
    emit_project_llvm(
        prepared.project(),
        &|module| prepared.initialization_region(module),
        GenericInitializerInput::PreparedRows(prepared),
        &|module, item, arm, _| validation.reference_cfg(module, item, arm),
    )
}

fn emit_project_llvm<'a>(
    project: &'a ScalarProject,
    initialization_region: &dyn Fn(usize) -> Option<&'a ModuleInitializationRegion>,
    generic_input: GenericInitializerInput<'a>,
    reference_cfg: &dyn Fn(
        usize,
        usize,
        Option<usize>,
        &'a ScalarFunction,
    ) -> Option<&'a ScalarReferenceCfg>,
) -> Result<LlvmPartition, String> {
    let ordered_modules = project
        .initialization_order
        .iter()
        .map(|source| {
            project
                .modules
                .iter()
                .enumerate()
                .find(|(_, module)| module.source == *source)
                .ok_or_else(|| format!("missing module {}", source.path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let modules = ordered_modules
        .iter()
        .map(|&(_, module)| module)
        .collect::<Vec<_>>();
    let regions = ordered_modules
        .iter()
        .map(|&(ordinal, module)| {
            initialization_region(ordinal).ok_or_else(|| {
                format!(
                    "internal error: missing module initialization region for {:?} (module {ordinal})",
                    module.source,
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let initializer_nodes = match generic_input {
        GenericInitializerInput::RawItems => None,
        GenericInitializerInput::PreparedRows(_) => Some(
            modules
                .iter()
                .zip(&regions)
                .map(|(module, region)| region.bind_call_nodes(&module.items))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let require_cfg = |module_ordinal, item_ordinal, arm_index, function: &'a ScalarFunction| {
        reference_cfg(module_ordinal, item_ordinal, arm_index, function).ok_or_else(|| {
            let source = &project.modules[module_ordinal].source;
            let span = function.span;
            format!(
                "internal error: missing reference CFG for {} in {source:?} at {}..{} (module {module_ordinal}, item {item_ordinal}, arm {arm_index:?})",
                function.name, span.start, span.end,
            )
        })
    };
    let all_structs = modules
        .iter()
        .flat_map(|module| module.structs.iter())
        .collect::<Vec<_>>();
    let layouts = crate::scalar::layout::ScalarLayoutIndex::project(&project.modules);
    let module_name = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .source
        .path
        .clone();
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&module_name));
    let builder = context.create_builder();
    let target_layout = project
        .modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .target_layout;
    let mut globals = BTreeMap::new();
    let mut functions = BTreeMap::new();
    let mut signatures = BTreeMap::new();
    let mut externs = Vec::new();
    let mut call_targets = BTreeMap::new();
    let mut initializer_symbols = InitializerSymbols::default();
    let mut definitions = Vec::new();
    for &(module_ordinal, source_module) in &ordered_modules {
        for (item_ordinal, item) in source_module.items.iter().enumerate() {
            if let ScalarItem::Binding(binding) = item {
                if initializer_nodes.is_some() {
                    initializer_symbols.callable_binding(&source_module.source, binding);
                }
                for (name, ty) in binding_receivers(binding).iter() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let name = project_global_name(&source_module.source, name);
                    let llvm_ty = storage_type(&context, ty, &layouts, target_layout)?;
                    let global = module.add_global(llvm_ty, None, &name);
                    global.set_linkage(Linkage::Internal);
                    global.set_initializer(&llvm_ty.const_zero());
                    globals.insert(name, (global, ty.clone()));
                }
            }
            if let ScalarItem::Function(function) = item {
                let name = project_function_name(&source_module.source, &function.name);
                if initializer_nodes.is_some() {
                    initializer_symbols.function(&source_module.source, function, name.clone());
                }
                if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
                    continue;
                }
                let cfg = require_cfg(module_ordinal, item_ordinal, None, function)?;
                let value = module.add_function(
                    &name,
                    definition_function_type(&context, function, cfg, target_layout)?,
                    None,
                );
                for (index, parameter) in function.parameters.iter().enumerate() {
                    if let Some(argument) = value.get_nth_param(index as u32) {
                        argument.set_name(parameter);
                    }
                }
                functions.insert(name.clone(), value);
                call_targets.insert(name.clone(), name.clone());
                signatures.insert(name.clone(), function.signature.clone());
                definitions.push((
                    module_ordinal,
                    source_module,
                    function,
                    cfg,
                    name,
                    item_ordinal,
                    None,
                ));
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
                    if initializer_nodes.is_some() {
                        initializer_symbols.external(
                            &source_module.source,
                            function.name_span,
                            &identity.internal_name,
                        );
                    }
                    call_targets.insert(key, identity.internal_name.clone());
                    signatures.insert(identity.internal_name.clone(), function.signature.clone());
                    functions.insert(identity.internal_name.clone(), value);
                    externs.push(identity);
                }
            }
        }
    }
    let mut prepared_specializations = match generic_input {
        GenericInitializerInput::RawItems => BTreeMap::new(),
        GenericInitializerInput::PreparedRows(_) => prepared_generic_specializations(
            ordered_modules
                .iter()
                .zip(&regions)
                .map(|(&(ordinal, module), &region)| {
                    (ordinal, &module.source, module.items.as_slice(), region)
                }),
            project_function_name,
        )?,
    };
    let mut project_specializations = Vec::new();
    for &(module_ordinal, source_module) in &ordered_modules {
        let specializations = match generic_input {
            GenericInitializerInput::RawItems => {
                generic_specializations(&source_module.items, |name| {
                    project_function_name(&source_module.source, name)
                })?
            }
            GenericInitializerInput::PreparedRows(_) => prepared_specializations
                .remove(&module_ordinal)
                .expect("prepared project module has a specialization set"),
        };
        for (lookup, specialization) in specializations {
            let LlvmSpecialization {
                function,
                name,
                template,
                item_ordinal,
                arm_index,
            } = specialization;
            let cfg = require_cfg(module_ordinal, item_ordinal, arm_index, template)?;
            let value = module.add_function(
                &name,
                definition_function_type(&context, &function, cfg, target_layout)?,
                None,
            );
            for (index, parameter) in function.parameters.iter().enumerate() {
                if let Some(argument) = value.get_nth_param(index as u32) {
                    argument.set_name(parameter);
                }
            }
            call_targets.insert(lookup, name.clone());
            signatures.insert(name.clone(), function.signature.clone());
            functions.insert(name.clone(), value);
            project_specializations.push((
                module_ordinal,
                source_module,
                function,
                cfg,
                name,
                item_ordinal,
                arm_index,
            ));
        }
    }
    let mut project_overloads = Vec::new();
    let mut prepared_overloads = prepared_initializer_overloads(
        initializer_nodes
            .iter()
            .flat_map(|_| regions.iter().copied()),
        &initializer_symbols,
    )?;
    for &(ordinal, source_module) in &ordered_modules {
        collect_prepared_body_overload_selections(
            generic_input,
            ordinal,
            &source_module.items,
            &initializer_symbols,
            &mut prepared_overloads,
        )?;
    }
    for &(module_ordinal, source_module) in &ordered_modules {
        let selections = modules.iter().flat_map(|calling_module| {
            selected_overload_selections(&calling_module.items, generic_input)
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
        for (lookup, specialization) in selected_overload_specializations(
            &source_module.items,
            |name| project_function_name(&source_module.source, name),
            selections.chain(
                prepared_overloads
                    .remove(&source_module.source)
                    .into_iter()
                    .flatten(),
            ),
        ) {
            let LlvmSpecialization {
                function,
                name,
                template,
                item_ordinal,
                arm_index,
            } = specialization;
            let cfg = require_cfg(module_ordinal, item_ordinal, arm_index, template)?;
            let value = module.add_function(
                &name,
                definition_function_type(&context, &function, cfg, target_layout)?,
                None,
            );
            for (index, parameter) in function.parameters.iter().enumerate() {
                if let Some(argument) = value.get_nth_param(index as u32) {
                    argument.set_name(parameter);
                }
            }
            call_targets.insert(lookup, name.clone());
            signatures.insert(name.clone(), function.signature.clone());
            functions.insert(name.clone(), value);
            project_overloads.push((
                module_ordinal,
                source_module,
                function,
                cfg,
                name,
                item_ordinal,
                arm_index,
            ));
        }
    }
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    let mut runtime_array_results = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for (_, _, function, cfg, name, _, _) in &definitions {
        insert_function_result_provenance(
            &mut runtime_array_results,
            &mut dynamic_owner_outputs,
            name,
            function,
            cfg,
        );
    }
    for (_, _, function, cfg, name, _, _) in
        project_specializations.iter().chain(&project_overloads)
    {
        insert_function_result_provenance(
            &mut runtime_array_results,
            &mut dynamic_owner_outputs,
            name,
            function,
            cfg,
        );
    }
    for (ordinal, source_module, function, cfg, name, item, arm) in definitions
        .iter()
        .map(|(ordinal, module, function, cfg, name, item, arm)| {
            (*ordinal, *module, *function, *cfg, name, *item, *arm)
        })
        .chain(
            project_specializations
                .iter()
                .chain(&project_overloads)
                .map(|(ordinal, module, function, cfg, name, item, arm)| {
                    (
                        *ordinal,
                        *module,
                        function.as_ref(),
                        *cfg,
                        name,
                        *item,
                        *arm,
                    )
                }),
        )
    {
        let nodes = generic_input.bind_body(ordinal, item, arm, &function.body)?;
        emit_project_function(
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
            function,
            cfg,
            nodes.as_ref(),
            &initializer_symbols,
            source_module,
            &modules,
            &globals,
            name,
            &module,
            &all_structs,
            &layouts,
            target_layout,
        )?;
    }
    emit_project_main(
        &context,
        &builder,
        &functions,
        &call_targets,
        &signatures,
        &runtime_array_results,
        &dynamic_owner_outputs,
        &modules,
        &regions,
        initializer_nodes.as_deref(),
        &initializer_symbols,
        &globals,
        &module,
        &all_structs,
        &layouts,
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
    reference_cfg: &ScalarReferenceCfg,
    target_layout: ScalarTargetLayout,
) -> Result<FunctionType<'ctx>, String> {
    let (ordinary, _) = function_type(context, &function.signature, target_layout)?;
    if !dynamic_return_owner(function.span, reference_cfg) {
        return Ok(ordinary);
    }
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &function.signature
    else {
        return Err("function has no callable signature".to_owned());
    };
    let mut arguments = parameters
        .iter()
        .map(|ty| basic_type(context, ty, target_layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    for _ in dynamic_return_outputs(function.span, reference_cfg) {
        arguments.push(context.ptr_type(AddressSpace::default()).into());
    }
    match outputs.outputs.as_slice() {
        [output] => Ok(basic_type(context, &output.ty, target_layout)?.fn_type(&arguments, false)),
        _ => Ok(aggregate_type(context, outputs, target_layout)?.fn_type(&arguments, false)),
    }
}

mod memory;
use memory::{
    aggregate_type, allocation_layout, basic_type, build_aggregate, builder_error,
    emit_array_literal, emit_place_value, emit_place_value_in_project, emit_struct_literal,
    emit_typed_expression, emit_utf8_literal, entry_alloca, extract_output, float_constant,
    integer_constant, integer_scalar_type, integer_type, integer_width, place_pointer,
    pointer_integer_type, storage_type, store_value, struct_member_place, structure, take_basic,
    value_type,
};
fn emit_binding_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
) -> Result<EmitValue<'ctx>, String> {
    if binding.output_sequence.outputs.len() == 1 {
        return emit_typed_expression(context, state, &binding.value, &binding.declared_type);
    }
    match binding.output_origin {
        ScalarBindingOutputOrigin::SingleExpression => {
            emit_expression(context, state, &binding.value)
        }
        ScalarBindingOutputOrigin::IndependentExpressions => {
            let canonical = state.has_prepared_calls();
            let values = binding
                .output_values
                .iter()
                .enumerate()
                .map(|(position, output)| {
                    let expression = if canonical && position == 0 {
                        &binding.value
                    } else {
                        &output.value
                    };
                    emit_typed_expression(context, state, expression, &output.ty)
                })
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
        return Ok(());
    }
    let receivers = binding_receivers(binding);
    for (position, (name, ty)) in receivers.iter().enumerate() {
        let value = extract_output(state, value.clone(), position)?;
        if *ty == ScalarType::Unit {
            state.values.insert(name.clone(), EmitValue::Unit);
            continue;
        }
        let slot = entry_alloca(
            state,
            storage_type(context, ty, state.layouts, state.target_layout)?,
            name,
        )?;
        store_value(context, state, slot, ty, value)?;
        state.storage.insert(name.clone(), (slot, ty.clone()));
        transition_runtime_array_binding_initialization(state, name, ty, &binding.value)?;
        transition_automatic_return_result_binding_initialization(
            state,
            name,
            ty,
            &binding.value,
            position,
        )?;
    }
    Ok(())
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

fn transition_runtime_array_binding_initialization(
    state: &mut EmitState<'_, '_>,
    name: &str,
    ty: &ScalarType,
    value: &ScalarExpression,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(());
    }
    let provenance = runtime_array_assignment_provenance(state, value)?;
    let owns = provenance.result.owns();
    state.runtime_array_owners.insert(name.into(), owns);
    state.runtime_array_allocations.insert(name.into(), false);
    if owns {
        if let Some(source) = provenance.source_name.filter(|source| source != name) {
            state.runtime_array_owners.insert(source, false);
        }
    }
    Ok(())
}

fn runtime_array_assignment_provenance(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
) -> Result<RuntimeArrayAssignmentProvenance, String> {
    Ok(match expression {
        ScalarExpression::Name { name, .. } => RuntimeArrayAssignmentProvenance {
            result: if state
                .runtime_array_owners
                .get(name)
                .copied()
                .unwrap_or(false)
            {
                RuntimeArrayResultProvenance::LocalOwner
            } else {
                RuntimeArrayResultProvenance::ParameterAlias
            },
            source_name: Some(name.clone()),
        },
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            let target = state
                .prepared_call(expression)?
                .expect("prepared call node");
            RuntimeArrayAssignmentProvenance {
                result: target
                    .symbol()
                    .and_then(|target| state.runtime_array_results.get(target))
                    .copied()
                    .unwrap_or(RuntimeArrayResultProvenance::FreshCall),
                source_name: None,
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            let lookup = overload_selection
                .as_ref()
                .map(|selection| selected_overload_lookup_key(&qualified, selection));
            let target = state
                .call_targets
                .get(lookup.as_ref().unwrap_or(&qualified))
                .or_else(|| {
                    state
                        .call_targets
                        .get(&specialization_lookup_key(&qualified, type_arguments))
                })
                .or_else(|| state.call_targets.get(&qualified));
            RuntimeArrayAssignmentProvenance {
                result: target
                    .and_then(|target| state.runtime_array_results.get(target))
                    .copied()
                    .unwrap_or(RuntimeArrayResultProvenance::FreshCall),
                source_name: None,
            }
        }
        _ => RuntimeArrayAssignmentProvenance {
            result: RuntimeArrayResultProvenance::FixedWidening,
            source_name: None,
        },
    })
}

fn insert_function_result_provenance(
    runtime_array_results: &mut BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &mut BTreeMap<String, BTreeSet<usize>>,
    name: &str,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
) {
    if let Some(provenance) = function_runtime_array_result_provenance(function) {
        runtime_array_results.insert(name.into(), provenance);
    }
    dynamic_owner_outputs.insert(
        name.into(),
        dynamic_return_outputs(function.span, reference_cfg),
    );
}

mod ownership;
use ownership::{
    binding_receivers, checked_call_owners, dynamic_return_outputs, dynamic_return_owner,
    emit_runtime_array_allocation, fresh_return_outputs, function_runtime_array_result_provenance,
    initialize_joined_return_bindings, joined_assignment_owners, joined_return_binding_types,
    release_scope_runtime_array_owners, return_bindings, transfer_runtime_array_return,
};
fn emit_project_binding_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
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
            let canonical = state.has_prepared_calls();
            let values = binding
                .output_values
                .iter()
                .enumerate()
                .map(|(position, output)| {
                    let expression = if canonical && position == 0 {
                        &binding.value
                    } else {
                        &output.value
                    };
                    emit_project_typed_expression(
                        context, state, expression, &output.ty, module, modules,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            build_aggregate(context, state, &binding.output_sequence, values)
        }
    }
}

fn emit_function<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    globals: &'ctx BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    body_call_nodes: Option<&'module CallableBodyCallNodes<'module>>,
    initializer_symbols: &'module InitializerSymbols,
    name: &str,
    items: &[ScalarItem],
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let (fresh_return_outputs, branch_return_outputs) =
        fresh_return_outputs(function, reference_cfg)?;
    let joined_bindings = joined_return_binding_types(function, &branch_return_outputs)?;
    let joined_assignment_owners =
        joined_assignment_owners(function.span, reference_cfg, &joined_bindings)?;
    let mut state = EmitState {
        initializer_nodes: None,
        body_call_nodes,
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: return_bindings(&function.body),
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
        checked_call_owners: checked_call_owners(reference_cfg)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations: mutable_runtime_allocations(reference_cfg),
        invalidations: resolved_invalidations(reference_cfg)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_joined_return_bindings(context, &mut state, joined_bindings)?;
    insert_unit_values(&mut state.values, items);
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (position, output) in dynamic_return_outputs(function.span, reference_cfg)
            .into_iter()
            .enumerate()
        {
            let slot = value
                .get_nth_param((parameters.len() + position) as u32)
                .ok_or_else(|| format!("missing mixed-return owner slot {output}"))?
                .into_pointer_value();
            state.return_owner_slots.insert(output, slot);
        }
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(index as u32)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            let slot = builder
                .build_alloca(
                    storage_type(context, parameter, layouts, target_layout)?,
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
                state
                    .runtime_array_owners
                    .insert(function.parameters[index].clone(), false);
                state
                    .runtime_array_allocations
                    .insert(function.parameters[index].clone(), false);
            }
        }
    }
    let outputs = match &function.signature {
        ScalarType::Callable { outputs, .. } => outputs,
        _ => return Err("function has no callable signature".into()),
    };
    if self_tail::eligible(context, &state, value, function, reference_cfg) {
        return self_tail::emit(context, &mut state, value, function, None);
    }
    let result = match function.body.items.as_slice() {
        [ScalarBlockItem::Expression(expression)]
            if canonical_tail_if(&function.body, BlockEmission::Return(outputs)).is_none() =>
        {
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
        _ => emit_block_at(
            context,
            &mut state,
            &function.body,
            BlockEmission::Return(outputs),
        )?,
    };
    if let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() {
        for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
            record_forwarded_return_owner(context, &mut state, expression, output)?;
        }
    }
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    emit_return(context, &mut state, result, outputs)
}

fn emit_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    globals: &'ctx BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    items: &[ScalarItem],
    initialization_region: &ModuleInitializationRegion,
    initializer_nodes: Option<&'module InitializerCallNodes<'module>>,
    initializer_symbols: &'module InitializerSymbols,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        initializer_nodes,
        body_call_nodes: None,
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        checked_call_owners: BTreeMap::new(),
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
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
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, items);
    for step in &initialization_region.steps {
        match (&step.kind, &items[step.item]) {
            (ModuleInitializationStepKind::BindingInitialization, ScalarItem::Binding(binding)) => {
                let value = emit_binding_value(context, &mut state, binding)?;
                if binding.is_allocation {
                    store_binding_outputs(context, &mut state, binding, value)?;
                    continue;
                }
                for (position, (name, ty)) in binding_receivers(binding).iter().enumerate() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let value = extract_output(&mut state, value.clone(), position)?;
                    let (global, _) = state
                        .globals
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("unknown LLVM global {name}"))?;
                    store_value(context, &mut state, global.as_pointer_value(), ty, value)?;
                    if matches!(ty, ScalarType::RuntimeArray { .. }) {
                        state
                            .storage
                            .insert(name.clone(), (global.as_pointer_value(), ty.clone()));
                    }
                    transition_runtime_array_binding_initialization(
                        &mut state,
                        name,
                        ty,
                        &binding.value,
                    )?;
                }
            }
            (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                emit_block_item(context, &mut state, item)?;
            }
            (ModuleInitializationStepKind::NamespaceDispatch { .. }, ScalarItem::Namespace(_)) => {}
            _ => panic!(
                "module initialization step disagrees with item {} in {:?}",
                step.item, initialization_region.source,
            ),
        }
    }
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new())?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn emit_project_function<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    body_call_nodes: Option<&'module CallableBodyCallNodes<'module>>,
    initializer_symbols: &'module InitializerSymbols,
    source_module: &ScalarModule,
    modules: &[&'module ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    name: &str,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let (fresh_return_outputs, branch_return_outputs) =
        fresh_return_outputs(function, reference_cfg)?;
    let joined_bindings = joined_return_binding_types(function, &branch_return_outputs)?;
    let joined_assignment_owners =
        joined_assignment_owners(function.span, reference_cfg, &joined_bindings)?;
    let mut state = EmitState {
        initializer_nodes: None,
        body_call_nodes,
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: return_bindings(&function.body),
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
        checked_call_owners: checked_call_owners(reference_cfg)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations: mutable_runtime_allocations(reference_cfg),
        invalidations: resolved_invalidations(reference_cfg)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: module_globals(source_module, globals),
        all_globals: globals.clone(),
        structs,
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_joined_return_bindings(context, &mut state, joined_bindings)?;
    insert_unit_values(&mut state.values, &source_module.items);
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (position, output) in dynamic_return_outputs(function.span, reference_cfg)
            .into_iter()
            .enumerate()
        {
            let slot = value
                .get_nth_param((parameters.len() + position) as u32)
                .ok_or_else(|| format!("missing mixed-return owner slot {output}"))?
                .into_pointer_value();
            state.return_owner_slots.insert(output, slot);
        }
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(index as u32)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            let slot = builder
                .build_alloca(
                    storage_type(context, parameter, layouts, target_layout)?,
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
                state
                    .runtime_array_owners
                    .insert(function.parameters[index].clone(), false);
                state
                    .runtime_array_allocations
                    .insert(function.parameters[index].clone(), false);
            }
        }
    }
    let outputs = match &function.signature {
        ScalarType::Callable { outputs, .. } => outputs,
        _ => return Err("function has no callable signature".into()),
    };
    if self_tail::eligible(context, &state, value, function, reference_cfg) {
        return self_tail::emit(
            context,
            &mut state,
            value,
            function,
            Some(ProjectCallScope {
                module: source_module,
                modules,
            }),
        );
    }
    let result = match function.body.items.as_slice() {
        [ScalarBlockItem::Expression(expression)]
            if canonical_tail_if(&function.body, BlockEmission::Return(outputs)).is_none() =>
        {
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
        _ => emit_project_block_at(
            context,
            &mut state,
            &function.body,
            source_module,
            modules,
            BlockEmission::Return(outputs),
        )?,
    };
    if let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() {
        for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
            record_forwarded_return_owner(context, &mut state, expression, output)?;
        }
    }
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    emit_return(context, &mut state, result, outputs)
}

fn emit_project_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    modules: &[&ScalarModule],
    regions: &[&ModuleInitializationRegion],
    initializer_nodes: Option<&'module [InitializerCallNodes<'module>]>,
    initializer_symbols: &'module InitializerSymbols,
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        initializer_nodes: None,
        body_call_nodes: None,
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        checked_call_owners: BTreeMap::new(),
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
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
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_project_module(
        context,
        &mut state,
        0,
        modules,
        regions,
        initializer_nodes,
        globals,
        &mut Vec::new(),
    )?;
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new())?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn initialize_project_module<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    position: usize,
    modules: &[&'module ScalarModule],
    regions: &[&ModuleInitializationRegion],
    initializer_nodes: Option<&'module [InitializerCallNodes<'module>]>,
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    initialized: &mut Vec<wosy_syntax::SourceIdentity>,
) -> Result<(), String> {
    let module = modules[position];
    let region = regions[position];
    assert_eq!(region.source, module.source);
    if initialized.contains(&module.source) {
        return Ok(());
    }
    initialized.push(module.source.clone());

    let previous_nodes = state.initializer_nodes;
    state.initializer_nodes = initializer_nodes.map(|nodes| &nodes[position]);

    let values = state.values.clone();
    let storage = state.storage.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
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
    state.checked_owner_flags.clear();
    state.automatic_return_result_owners.clear();
    insert_unit_values(&mut state.values, &module.items);

    for step in &region.steps {
        match (&step.kind, &module.items[step.item]) {
            (
                ModuleInitializationStepKind::NamespaceDispatch {
                    binding,
                    binding_span,
                    target,
                },
                ScalarItem::Namespace(namespace),
            ) => {
                assert_eq!(binding, &namespace.binding);
                assert_eq!(*binding_span, namespace.binding_span);
                let NamespaceTarget::Bound(target) = target else {
                    return Err(format!("unknown namespace binding {binding}"));
                };
                let position = modules
                    .iter()
                    .position(|candidate| &candidate.source == target)
                    .ok_or_else(|| format!("missing namespace target {}", target.path))?;
                initialize_project_module(
                    context,
                    state,
                    position,
                    modules,
                    regions,
                    initializer_nodes,
                    globals,
                    initialized,
                )?;
                state.globals = module_globals(module, globals);
            }
            (ModuleInitializationStepKind::BindingInitialization, ScalarItem::Binding(binding)) => {
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
                    state
                        .builder
                        .build_store(global.as_pointer_value(), pointer)
                        .map_err(builder_error)?;
                    state.storage.insert(
                        receiver.name.clone(),
                        (global.as_pointer_value(), receiver.ty.clone()),
                    );
                    state
                        .runtime_array_owners
                        .insert(receiver.name.clone(), true);
                    state
                        .runtime_array_allocations
                        .insert(receiver.name.clone(), false);
                    continue;
                }
                for (position, (name, ty)) in binding_receivers(binding).iter().enumerate() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let value = extract_output(state, value.clone(), position)?;
                    let (global, _) = state
                        .all_globals
                        .get(&project_global_name(&module.source, name))
                        .cloned()
                        .ok_or_else(|| format!("unknown LLVM global {name}"))?;
                    store_value(context, state, global.as_pointer_value(), ty, value)?;
                    if matches!(ty, ScalarType::RuntimeArray { .. }) {
                        state
                            .storage
                            .insert(name.clone(), (global.as_pointer_value(), ty.clone()));
                    }
                    transition_runtime_array_binding_initialization(
                        state,
                        name,
                        ty,
                        &binding.value,
                    )?;
                }
            }
            (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                emit_project_block_item(context, state, item, module, modules)?;
            }
            _ => panic!(
                "module initialization step disagrees with item {} in {:?}",
                step.item, module.source,
            ),
        }
    }

    state.initializer_nodes = previous_nodes;
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
    let mut retained_return_results = automatic_return_result_owners;
    retained_return_results.extend(std::mem::take(&mut state.automatic_return_result_owners));
    state.automatic_return_result_owners = retained_return_results;
    state.checked_owner_flags = checked_owner_flags;
    Ok(())
}

#[derive(Clone, Copy)]
enum BlockEmission<'a> {
    Ordinary,
    Return(&'a crate::ScalarOutputSequence),
    TailArm(&'a crate::ScalarOutputSequence),
}

impl<'a> BlockEmission<'a> {
    fn ordered_outputs(self) -> Option<&'a crate::ScalarOutputSequence> {
        match self {
            Self::Return(outputs) | Self::TailArm(outputs) if outputs.outputs.len() > 1 => {
                Some(outputs)
            }
            _ => None,
        }
    }

    fn explicit_ordered_outputs(
        self,
        expression_count: usize,
    ) -> Result<Option<&'a crate::ScalarOutputSequence>, String> {
        let (outputs, site) = match self {
            Self::TailArm(outputs) => (outputs, "conditional arm"),
            Self::Return(outputs) if expression_count > 1 && outputs.outputs.len() > 1 => {
                (outputs, "callable return")
            }
            _ => return Ok(None),
        };
        if expression_count != outputs.outputs.len() {
            return Err(format!("{site} has the wrong number of LLVM outputs"));
        }
        Ok(Some(outputs))
    }
}

struct CanonicalTailIf<'syntax, 'signature> {
    condition: &'syntax ScalarExpression,
    then_branch: &'syntax ScalarBlock,
    else_branch: &'syntax ScalarBlock,
    outputs: &'signature crate::ScalarOutputSequence,
}

fn canonical_tail_if<'syntax, 'signature>(
    block: &'syntax ScalarBlock,
    emission: BlockEmission<'signature>,
) -> Option<CanonicalTailIf<'syntax, 'signature>> {
    let outputs = emission.ordered_outputs()?;
    if block.final_output_values.len() != 1 {
        return None;
    }
    let (expression, _) = block.final_expressions().next()?;
    match expression {
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => Some(CanonicalTailIf {
            condition,
            then_branch,
            else_branch,
            outputs,
        }),
        _ => None,
    }
}

fn emit_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    emit_block_at(context, state, block, BlockEmission::Ordinary)
}

fn emit_block_at<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    emission: BlockEmission<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let values = state.values.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
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
    if !block.final_output_values.is_empty() || matches!(emission, BlockEmission::TailArm(_)) {
        result = emit_final_outputs(context, state, block, emission)?;
    }
    result = finish_block_return(context, state, block, result, emission)?;
    release_scope_runtime_array_owners(context, state, &runtime_array_owners)?;
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    state.storage = storage;
    state.values = values;
    state.runtime_array_owners = runtime_array_owners
        .into_iter()
        .map(|(name, owner)| {
            let updated = *state.runtime_array_owners.get(&name).unwrap_or(&owner);
            (name, updated)
        })
        .collect();
    state.runtime_array_allocations = runtime_array_allocations;
    state.runtime_array_lengths = runtime_array_lengths;
    state.runtime_array_live = runtime_array_live;
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
    emit_project_block_at(
        context,
        state,
        block,
        module,
        modules,
        BlockEmission::Ordinary,
    )
}

fn emit_project_block_at<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
    emission: BlockEmission<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let values = state.values.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
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
    if !block.final_output_values.is_empty() || matches!(emission, BlockEmission::TailArm(_)) {
        result = emit_project_final_outputs(context, state, block, module, modules, emission)?;
    }
    result = finish_block_return(context, state, block, result, emission)?;
    release_scope_runtime_array_owners(context, state, &runtime_array_owners)?;
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    state.storage = storage;
    state.values = values;
    state.runtime_array_owners = runtime_array_owners
        .into_iter()
        .map(|(name, owner)| {
            let updated = *state.runtime_array_owners.get(&name).unwrap_or(&owner);
            (name, updated)
        })
        .collect();
    state.runtime_array_allocations = runtime_array_allocations;
    state.runtime_array_lengths = runtime_array_lengths;
    state.runtime_array_live = runtime_array_live;
    state.automatic_return_result_owners = automatic_return_result_owners;
    state.checked_owner_flags = checked_owner_flags;
    Ok(result)
}

fn final_output_expression(
    block: &ScalarBlock,
    position: usize,
    canonical: bool,
) -> &ScalarExpression {
    if !canonical {
        return &block.final_output_values[position].value;
    }
    let first_output = block.items.len() - block.final_output_values.len();
    let ScalarBlockItem::Expression(expression) = &block.items[first_output + position] else {
        unreachable!("canonical final output is an expression item")
    };
    expression
}

fn emit_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    emission: BlockEmission<'_>,
) -> Result<EmitValue<'ctx>, String> {
    if let Some(tail) = canonical_tail_if(block, emission) {
        let condition =
            take_basic(emit_expression(context, state, tail.condition)?)?.into_int_value();
        return emit_if_value(
            context,
            state,
            condition,
            tail.then_branch,
            tail.else_branch,
            |state, branch| {
                emit_block_at(context, state, branch, BlockEmission::TailArm(tail.outputs))
            },
        );
    }
    if let Some(outputs) = emission.explicit_ordered_outputs(block.final_output_values.len())? {
        return emit_ordered_final_outputs(context, state, block, outputs, None);
    }
    let canonical = state.has_prepared_calls();
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        let expression = final_output_expression(block, 0, canonical);
        if matches!(expression, ScalarExpression::Call { .. }) {
            transfer_runtime_array_return(state, block, expression, &output.ty);
            transfer_automatic_return_result(state, expression);
            let value = emit_expression(context, state, expression)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, expression, output.position)?;
            }
            return Ok(value);
        }
        let value = emit_typed_expression(context, state, expression, &output.ty)?;
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, expression, output.position)?;
        }
        transfer_runtime_array_return(state, block, expression, &output.ty);
        transfer_automatic_return_result(state, expression);
        return Ok(value);
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
        .enumerate()
        .map(|(position, output)| {
            let expression = final_output_expression(block, position, canonical);
            emit_typed_expression(context, state, expression, &output.ty)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for position in 0..block.final_output_values.len() {
            let expression = final_output_expression(block, position, canonical);
            record_forwarded_return_owner(context, state, expression, position)?;
        }
    }
    for (position, output) in block.final_output_values.iter().enumerate() {
        let expression = final_output_expression(block, position, canonical);
        transfer_runtime_array_return(state, block, expression, &output.ty);
        transfer_automatic_return_result(state, expression);
    }
    build_aggregate(context, state, &outputs, values)
}

fn emit_project_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
    emission: BlockEmission<'_>,
) -> Result<EmitValue<'ctx>, String> {
    if let Some(tail) = canonical_tail_if(block, emission) {
        let condition = take_basic(emit_project_expression(
            context,
            state,
            tail.condition,
            module,
            modules,
        )?)?
        .into_int_value();
        return emit_if_value(
            context,
            state,
            condition,
            tail.then_branch,
            tail.else_branch,
            |state, branch| {
                emit_project_block_at(
                    context,
                    state,
                    branch,
                    module,
                    modules,
                    BlockEmission::TailArm(tail.outputs),
                )
            },
        );
    }
    if let Some(outputs) = emission.explicit_ordered_outputs(block.final_output_values.len())? {
        return emit_ordered_final_outputs(context, state, block, outputs, Some((module, modules)));
    }
    let canonical = state.has_prepared_calls();
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        let expression = final_output_expression(block, 0, canonical);
        if matches!(expression, ScalarExpression::Call { .. }) {
            transfer_runtime_array_return(state, block, expression, &output.ty);
            transfer_automatic_return_result(state, expression);
            let value = emit_project_expression(context, state, expression, module, modules)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, expression, output.position)?;
            }
            return Ok(value);
        }
        let value =
            emit_project_typed_expression(context, state, expression, &output.ty, module, modules)?;
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, expression, output.position)?;
        }
        transfer_runtime_array_return(state, block, expression, &output.ty);
        transfer_automatic_return_result(state, expression);
        return Ok(value);
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
        .enumerate()
        .map(|(position, output)| {
            let expression = final_output_expression(block, position, canonical);
            emit_project_typed_expression(context, state, expression, &output.ty, module, modules)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for position in 0..block.final_output_values.len() {
            let expression = final_output_expression(block, position, canonical);
            record_forwarded_return_owner(context, state, expression, position)?;
        }
    }
    for (position, output) in block.final_output_values.iter().enumerate() {
        let expression = final_output_expression(block, position, canonical);
        transfer_runtime_array_return(state, block, expression, &output.ty);
        transfer_automatic_return_result(state, expression);
    }
    build_aggregate(context, state, &outputs, values)
}

fn emit_ordered_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    outputs: &crate::ScalarOutputSequence,
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<EmitValue<'ctx>, String> {
    let values = block
        .final_expressions()
        .zip(&outputs.outputs)
        .map(|((expression, _), output)| match project {
            Some((module, modules)) => emit_project_typed_expression(
                context, state, expression, &output.ty, module, modules,
            ),
            None => emit_typed_expression(context, state, expression, &output.ty),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for (position, (expression, _)) in block.final_expressions().enumerate() {
            record_forwarded_return_owner(context, state, expression, position)?;
        }
    }
    for ((expression, _), output) in block.final_expressions().zip(&outputs.outputs) {
        transfer_runtime_array_return(state, block, expression, &output.ty);
        transfer_automatic_return_result(state, expression);
    }
    build_aggregate(context, state, outputs, values)
}

enum AssignmentDestination<'ctx> {
    Unit,
    Existing(PointerValue<'ctx>),
    NewStorage {
        name: String,
        slot: PointerValue<'ctx>,
    },
}

struct JoinedAssignmentPrevious<'ctx> {
    owner_slot: PointerValue<'ctx>,
    storage: PointerValue<'ctx>,
    owner: inkwell::values::IntValue<'ctx>,
    address: inkwell::values::IntValue<'ctx>,
}

fn capture_joined_assignment_previous<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    destination: &AssignmentDestination<'ctx>,
) -> Result<Option<JoinedAssignmentPrevious<'ctx>>, String> {
    if matches!(destination, AssignmentDestination::Unit) {
        return Ok(None);
    }
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(None);
    };
    let Some((owner_slot, _)) = state.joined_return_bindings.get(name).cloned() else {
        return Ok(None);
    };
    if !matches!(ty, ScalarType::CheckedReference { .. }) {
        return Err(format!(
            "joined return binding {name} lacks checked-reference type"
        ));
    }
    let AssignmentDestination::Existing(destination) = destination else {
        unreachable!("joined destination has existing storage")
    };
    let owner = state
        .builder
        .build_load(context.bool_type(), owner_slot, "joined_previous_owner")
        .map_err(builder_error)?
        .into_int_value();
    let address = state
        .builder
        .build_load(
            basic_type(context, ty, state.target_layout)?,
            *destination,
            "joined_previous_address",
        )
        .map_err(builder_error)?
        .into_int_value();
    Ok(Some(JoinedAssignmentPrevious {
        owner_slot,
        storage: *destination,
        owner,
        address,
    }))
}

#[derive(Clone, Copy)]
struct JoinedAssignmentSource<'ctx> {
    owner_slot: PointerValue<'ctx>,
    storage: PointerValue<'ctx>,
    owned: inkwell::values::IntValue<'ctx>,
}

struct AssignmentValue<'ctx> {
    value: EmitValue<'ctx>,
    ty: ScalarType,
    provenance: RuntimeArrayAssignmentProvenance,
    call_flag: Option<inkwell::values::IntValue<'ctx>>,
    joined_source: Option<JoinedAssignmentSource<'ctx>>,
    runtime_array_source: Option<PointerValue<'ctx>>,
}

struct RuntimeArrayAssignment<'ctx> {
    owns: bool,
    previous: Option<ownership::RuntimeArrayRelease<'ctx>>,
}

fn stage_runtime_array_assignments<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    values: &[AssignmentValue<'ctx>],
    destinations: &[AssignmentDestination<'ctx>],
) -> Result<Vec<Option<RuntimeArrayAssignment<'ctx>>>, String> {
    let mut owns = values
        .iter()
        .map(|value| value.provenance.result.owns())
        .collect::<Vec<_>>();
    let mut transferred = std::collections::HashSet::new();
    let mut sources = Vec::new();
    let mut self_replacements = Vec::new();
    for (position, ((target, value), destination)) in assignment
        .targets
        .iter()
        .zip(values)
        .zip(destinations)
        .enumerate()
    {
        if !matches!(value.ty, ScalarType::RuntimeArray { .. })
            || !matches!(target.place, crate::ScalarPlace::Name { .. })
        {
            continue;
        }
        let Some(source) = value.runtime_array_source else {
            continue;
        };
        if matches!(destination, AssignmentDestination::Existing(slot) if *slot == source) {
            self_replacements.push((position, source));
        } else {
            owns[position] = transferred.insert(source);
            if owns[position] {
                sources.push((
                    value
                        .provenance
                        .source_name
                        .as_ref()
                        .expect("runtime-array lexical source"),
                    source,
                ));
            }
        }
    }
    let mut retains_storage = vec![false; values.len()];
    for (position, source) in self_replacements {
        owns[position] = !transferred.contains(&source);
        retains_storage[position] = true;
    }
    let mut staged = Vec::with_capacity(values.len());
    for (position, ((target, value), destination)) in assignment
        .targets
        .iter()
        .zip(values)
        .zip(destinations)
        .enumerate()
    {
        let crate::ScalarPlace::Name { name, .. } = &target.place else {
            staged.push(None);
            continue;
        };
        if !matches!(value.ty, ScalarType::RuntimeArray { .. }) {
            staged.push(None);
            continue;
        }
        let protected = retains_storage[position]
            || matches!(destination, AssignmentDestination::Existing(slot) if transferred.contains(slot));
        let previous = if protected {
            None
        } else {
            ownership::capture_runtime_array_release(context, state, name)?
        };
        staged.push(Some(RuntimeArrayAssignment {
            owns: owns[position],
            previous,
        }));
    }
    for (name, slot) in sources {
        if state.storage.get(name).map(|(storage, _)| *storage) == Some(slot) {
            state.runtime_array_owners.insert(name.clone(), false);
        }
    }
    Ok(staged)
}

fn stage_joined_assignment_owners<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    values: &[AssignmentValue<'ctx>],
    previous: &mut [Option<JoinedAssignmentPrevious<'ctx>>],
) -> Result<Vec<Option<inkwell::values::IntValue<'ctx>>>, String> {
    let mut owners = vec![None; previous.len()];
    let mut transferred = HashMap::new();
    let mut sources = Vec::new();
    let mut self_replacements = Vec::new();
    for (position, ((target, value), previous)) in assignment
        .targets
        .iter()
        .zip(values)
        .zip(previous.iter())
        .enumerate()
    {
        let (Some(source), Some(previous)) = (value.joined_source, previous) else {
            continue;
        };
        let classification = state
            .joined_assignment_owners
            .get(&(target.target_span.start, target.target_span.end))
            .ok_or_else(|| {
                format!(
                    "missing validated assignment owner at {:?}",
                    target.target_span
                )
            })?;
        if matches!(classification, CheckedAssignmentOwner::Borrowed) {
            continue;
        }
        if source.owner_slot == previous.owner_slot && source.storage == previous.storage {
            self_replacements.push((position, source));
            continue;
        }
        match transferred.entry((source.owner_slot, source.storage)) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(source.owned);
                owners[position] = Some(source.owned);
                sources.push(source);
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                owners[position] = Some(context.bool_type().const_zero())
            }
        }
    }
    for (position, source) in self_replacements {
        owners[position] = Some(
            if transferred.contains_key(&(source.owner_slot, source.storage)) {
                context.bool_type().const_zero()
            } else {
                source.owned
            },
        );
    }
    for previous in previous.iter_mut().flatten() {
        if let Some(owned) = transferred.get(&(previous.owner_slot, previous.storage)) {
            let retained = state
                .builder
                .build_not(*owned, "joined_owner_not_transferred")
                .map_err(builder_error)?;
            previous.owner = state
                .builder
                .build_and(previous.owner, retained, "joined_previous_eligible")
                .map_err(builder_error)?;
        }
    }
    for source in sources {
        state
            .builder
            .build_store(source.owner_slot, context.bool_type().const_zero())
            .map_err(builder_error)?;
    }
    Ok(owners)
}

fn retain_assignment_destination<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
) -> Result<AssignmentDestination<'ctx>, String> {
    if *ty == ScalarType::Unit {
        return Ok(AssignmentDestination::Unit);
    }
    match &target.place {
        crate::ScalarPlace::Name { name, .. } => {
            match state.storage.get(name).map(|(slot, _)| *slot).or_else(|| {
                state
                    .globals
                    .get(name)
                    .map(|(global, _)| global.as_pointer_value())
            }) {
                Some(destination) => Ok(AssignmentDestination::Existing(destination)),
                None => {
                    let slot = entry_alloca(
                        state,
                        storage_type(context, ty, state.layouts, state.target_layout)?,
                        name,
                    )?;
                    Ok(AssignmentDestination::NewStorage {
                        name: name.clone(),
                        slot,
                    })
                }
            }
        }
        place => Ok(AssignmentDestination::Existing(place_pointer(
            context, state, place, None,
        )?)),
    }
}

fn install_assignment_destination<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    destination: AssignmentDestination<'ctx>,
) -> Option<PointerValue<'ctx>> {
    match destination {
        AssignmentDestination::Unit => {
            state.values.insert(target.target.clone(), EmitValue::Unit);
            None
        }
        AssignmentDestination::Existing(pointer) => Some(pointer),
        AssignmentDestination::NewStorage { name, slot } => {
            state.storage.insert(name, (slot, ty.clone()));
            Some(slot)
        }
    }
}

fn emit_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
) -> Result<EmitValue<'ctx>, String> {
    let expected = assignment
        .targets
        .iter()
        .map(|target| assignment_target_type(state, target))
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
    let destinations = assignment
        .targets
        .iter()
        .zip(&values)
        .map(|(target, value)| retain_assignment_destination(context, state, target, &value.ty))
        .collect::<Result<Vec<_>, String>>()?;
    let mut previous = assignment
        .targets
        .iter()
        .zip(&values)
        .zip(&destinations)
        .map(|((target, value), destination)| {
            capture_joined_assignment_previous(context, state, target, &value.ty, destination)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let owners =
        stage_joined_assignment_owners(context, state, assignment, &values, &mut previous)?;
    let runtime_arrays =
        stage_runtime_array_assignments(context, state, assignment, &values, &destinations)?;
    for (position, (((((target, staged), destination), previous), joined_owner), runtime_array)) in
        assignment
            .targets
            .iter()
            .zip(values)
            .zip(destinations)
            .zip(previous)
            .zip(owners)
            .zip(runtime_arrays)
            .enumerate()
    {
        let AssignmentValue {
            value,
            ty,
            provenance: _,
            call_flag,
            joined_source: _,
            runtime_array_source: _,
        } = staged;
        let Some(destination) = install_assignment_destination(state, target, &ty, destination)
        else {
            continue;
        };
        let expression = if assignment.values.len() == 1 {
            &assignment.values[0]
        } else {
            &assignment.values[position]
        };
        let value = transition_joined_checked_assignment(
            context,
            state,
            target,
            expression,
            position,
            previous,
            &ty,
            value,
            call_flag,
            joined_owner,
        )?;
        transition_runtime_array_assignment(context, state, target, runtime_array)?;
        store_value(context, state, destination, &ty, value)?;
    }
    Ok(EmitValue::Unit)
}
fn emit_project_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let expected = assignment
        .targets
        .iter()
        .map(|target| {
            if let Some(receiver) = target.receiver.as_ref().filter(|receiver| {
                module
                    .namespace_bindings
                    .iter()
                    .any(|namespace| namespace.binding == **receiver)
            }) {
                let target_module = project_namespace_target(module, modules, receiver)?;
                Ok(Some(
                    project_member_global(state, target_module, &target.target)?.1,
                ))
            } else {
                Ok(assignment_target_type(state, target))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
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
    let destinations = assignment
        .targets
        .iter()
        .zip(&values)
        .map(|(target, value)| {
            let ty = &value.ty;
            if *ty == ScalarType::Unit {
                return Ok(AssignmentDestination::Unit);
            }
            if let Some(receiver) = target.receiver.as_ref().filter(|receiver| {
                module
                    .namespace_bindings
                    .iter()
                    .any(|namespace| namespace.binding == **receiver)
            }) {
                let target_module = project_namespace_target(module, modules, receiver)?;
                let (global, _) = project_member_global(state, target_module, &target.target)?;
                let pointer = global
                    .ok_or_else(|| format!("unknown LLVM member storage {}", target.target))?
                    .as_pointer_value();
                Ok(AssignmentDestination::Existing(pointer))
            } else {
                retain_assignment_destination(context, state, target, ty)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut previous = assignment
        .targets
        .iter()
        .zip(&values)
        .zip(&destinations)
        .map(|((target, value), destination)| {
            capture_joined_assignment_previous(context, state, target, &value.ty, destination)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let owners =
        stage_joined_assignment_owners(context, state, assignment, &values, &mut previous)?;
    let runtime_arrays =
        stage_runtime_array_assignments(context, state, assignment, &values, &destinations)?;
    for (position, (((((target, staged), destination), previous), joined_owner), runtime_array)) in
        assignment
            .targets
            .iter()
            .zip(values)
            .zip(destinations)
            .zip(previous)
            .zip(owners)
            .zip(runtime_arrays)
            .enumerate()
    {
        let AssignmentValue {
            value,
            ty,
            provenance: _,
            call_flag,
            joined_source: _,
            runtime_array_source: _,
        } = staged;
        let Some(destination) = install_assignment_destination(state, target, &ty, destination)
        else {
            continue;
        };
        let expression = if assignment.values.len() == 1 {
            &assignment.values[0]
        } else {
            &assignment.values[position]
        };
        let value = transition_joined_checked_assignment(
            context,
            state,
            target,
            expression,
            position,
            previous,
            &ty,
            value,
            call_flag,
            joined_owner,
        )?;
        transition_runtime_array_assignment(context, state, target, runtime_array)?;
        store_value(context, state, destination, &ty, value)?;
    }
    Ok(EmitValue::Unit)
}

fn transition_joined_checked_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    expression: &ScalarExpression,
    output: usize,
    previous: Option<JoinedAssignmentPrevious<'ctx>>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
    call_flag: Option<inkwell::values::IntValue<'ctx>>,
    joined_owner: Option<inkwell::values::IntValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let Some(JoinedAssignmentPrevious {
        owner_slot,
        storage: _,
        owner: previous_owner,
        address: previous_address,
    }) = previous
    else {
        return Ok(value);
    };
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        unreachable!("joined destination is a name")
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
            if let ScalarExpression::Name { .. } = expression {
                if let Some(owned) = joined_owner {
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
                    if let Some(owned) = joined_owner {
                        owned
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
    Ok(value)
}

fn transition_runtime_array_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    staged: Option<RuntimeArrayAssignment<'ctx>>,
) -> Result<(), String> {
    let Some(staged) = staged else { return Ok(()) };
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(());
    };
    if let Some(previous) = staged.previous {
        ownership::consume_runtime_array_release(context, state, previous)?;
    }
    state.runtime_array_owners.insert(name.clone(), staged.owns);
    state.runtime_array_allocations.insert(name.clone(), false);
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
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            match state.prepared_call_output(expression).ok().flatten()? {
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                    Some(*inner)
                }
                _ => None,
            }
        }
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
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty)
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
            .is_some_and(|ty| matches!(ty, ScalarType::CheckedReference { .. })),
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            matches!(
                state.prepared_call_output(expression),
                Ok(Some(ScalarType::CheckedReference { .. }))
            )
        }
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
) -> Result<Vec<AssignmentValue<'ctx>>, String> {
    let mut emit = emit;
    let mut position = 0;
    let mut values = Vec::new();
    for expression in expressions {
        let provenance = runtime_array_assignment_provenance(state, expression)?;
        let runtime_array_source = if provenance.result.owns() {
            provenance.source_name.as_ref().map(|name| {
                state
                    .storage
                    .get(name)
                    .expect("owning runtime-array lexical storage")
                    .0
            })
        } else {
            None
        };
        let value = emit(
            state,
            expression,
            expected.get(position).and_then(Option::as_ref),
        )?;
        let joined_source = if let ScalarExpression::Name { name, .. } = expression {
            if let Some((owner_slot, _)) = state.joined_return_bindings.get(name) {
                let storage = state
                    .storage
                    .get(name)
                    .expect("joined source has lexical storage")
                    .0;
                let owned = state
                    .builder
                    .build_load(context.bool_type(), *owner_slot, "joined_assignment_owned")
                    .map_err(builder_error)?
                    .into_int_value();
                Some(JoinedAssignmentSource {
                    owner_slot: *owner_slot,
                    storage,
                    owned,
                })
            } else {
                None
            }
        } else {
            None
        };
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
                values.push(AssignmentValue {
                    value: EmitValue::Unit,
                    ty,
                    provenance: provenance.clone(),
                    call_flag: None,
                    joined_source: None,
                    runtime_array_source: None,
                });
                position += 1;
                continue;
            }
            let temporary = entry_alloca(
                state,
                storage_type(context, &ty, state.layouts, state.target_layout)?,
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
            values.push(AssignmentValue {
                value,
                ty,
                provenance: provenance.clone(),
                call_flag: call_flags.get(&output_position).copied(),
                joined_source,
                runtime_array_source,
            });
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
mod self_tail;
use expression::{
    emit_expression, emit_if_value, emit_project_expression, emit_project_typed_expression,
};
mod return_memory;
use return_memory::{
    emit_automatic_return_result, emit_return, finish_block_return, record_forwarded_return_owner,
    transfer_automatic_return_result,
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

struct LlvmSpecialization<'a> {
    function: Cow<'a, ScalarFunction>,
    name: String,
    template: &'a ScalarFunction,
    item_ordinal: usize,
    arm_index: Option<usize>,
}

#[derive(Clone, Copy)]
enum GenericInitializerInput<'a> {
    RawItems,
    PreparedRows(&'a crate::PreparedScalarProject),
}

impl GenericInitializerInput<'_> {
    fn bind_body<'a>(
        &'a self,
        module: usize,
        item: usize,
        arm: Option<usize>,
        body: &'a ScalarBlock,
    ) -> Result<Option<CallableBodyCallNodes<'a>>, String> {
        match self {
            Self::RawItems => Ok(None),
            Self::PreparedRows(prepared) => prepared
                .bind_callable_body_call_nodes(module, item, arm, body)
                .map(Some),
        }
    }
}

mod initializer_calls;
use initializer_calls::{
    collect_prepared_body_overload_selections, prepared_initializer_overloads, InitializerSymbols,
    PreparedCall,
};

fn prepared_generic_specializations<'a>(
    modules: impl Iterator<
            Item = (
                usize,
                &'a wosy_syntax::SourceIdentity,
                &'a [ScalarItem],
                &'a ModuleInitializationRegion,
            ),
        > + Clone,
    symbol: impl Fn(&wosy_syntax::SourceIdentity, &str) -> String,
) -> Result<BTreeMap<usize, BTreeMap<String, LlvmSpecialization<'a>>>, String> {
    let mut templates = HashMap::new();
    let mut specializations = BTreeMap::new();
    for (module_ordinal, source, items, _) in modules.clone() {
        specializations.insert(module_ordinal, BTreeMap::new());
        for (item_ordinal, item) in items.iter().enumerate() {
            if let ScalarItem::Function(function) = item {
                if !function.generic_parameters.is_empty() && function.overload_arms.is_empty() {
                    templates.insert(
                        (source, function.name_span),
                        (module_ordinal, item_ordinal, function),
                    );
                }
            }
        }
    }
    for (_, _, _, region) in modules {
        for target in region.call_targets()? {
            let ModuleInitializationCallTarget::Generic {
                declaration,
                type_arguments,
            } = target?
            else {
                continue;
            };
            let &(module_ordinal, item_ordinal, function) = templates
                .get(&(&declaration.source, declaration.range))
                .ok_or_else(|| {
                    format!(
                        "internal error: missing initializer generic declaration {declaration:?}"
                    )
                })?;
            insert_generic_specialization(
                specializations
                    .get_mut(&module_ordinal)
                    .expect("initializer template belongs to an indexed module"),
                function,
                item_ordinal,
                type_arguments,
                &symbol(&declaration.source, &function.name),
            )?;
        }
    }
    Ok(specializations)
}

fn generic_specializations(
    items: &[ScalarItem],
    symbol: impl Fn(&str) -> String,
) -> Result<BTreeMap<String, LlvmSpecialization<'_>>, String> {
    let mut calls = Vec::new();
    for item in items {
        match item {
            ScalarItem::Binding(binding) => collect_generic_calls(&binding.value, &mut calls),
            ScalarItem::Executable(item) => collect_generic_calls_in_item(item, &mut calls),
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Function(_) => {}
        }
    }
    let mut specializations = BTreeMap::new();
    for (name, arguments) in calls {
        let Some((item_ordinal, function)) =
            items
                .iter()
                .enumerate()
                .find_map(|(index, item)| match item {
                    ScalarItem::Function(function) if function.name == name => {
                        Some((index, function))
                    }
                    _ => None,
                })
        else {
            continue;
        };
        if function.generic_parameters.is_empty() {
            continue;
        }
        insert_generic_specialization(
            &mut specializations,
            function,
            item_ordinal,
            &arguments,
            &symbol(&name),
        )?;
    }
    Ok(specializations)
}

fn insert_generic_specialization<'a>(
    specializations: &mut BTreeMap<String, LlvmSpecialization<'a>>,
    function: &'a ScalarFunction,
    item_ordinal: usize,
    arguments: &[ScalarType],
    base: &str,
) -> Result<(), String> {
    if function.generic_parameters.len() != arguments.len() {
        return Err("validated generic call has invalid type argument arity".to_owned());
    }
    let lookup = generic_specialization_key(base, arguments);
    specializations.entry(lookup).or_insert_with(|| {
        let signature = substitute_generic_signature(function, arguments);
        let name = generic_specialization_name(base, arguments);
        let mut body = function.clone();
        body.signature = signature;
        body.generic_parameters.clear();
        LlvmSpecialization {
            function: Cow::Owned(body),
            name,
            template: function,
            item_ordinal,
            arm_index: None,
        }
    });
    Ok(())
}

fn selected_overload_specializations(
    items: &[ScalarItem],
    symbol: impl Fn(&str) -> String,
    selections: impl IntoIterator<Item = (String, ScalarOverloadSelection)>,
) -> BTreeMap<String, LlvmSpecialization<'_>> {
    let overloads = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                Some((function.name.as_str(), (index, function)))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut selections = selections.into_iter().collect::<Vec<_>>();
    let mut specializations = BTreeMap::new();
    for (name, (_, overload)) in &overloads {
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
        let Some(&(item_ordinal, overload)) = overloads.get(name.as_str()) else {
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
        let lookup = selected_overload_lookup_key(&base, &selection);
        let name = generic_specialization_name(
            &encoded_llvm_name(
                "wosy_overload",
                [base.as_str(), &selection.arm_index.to_string()],
            ),
            &arguments,
        );
        specializations
            .entry(lookup)
            .or_insert_with(|| LlvmSpecialization {
                function: if arguments.is_empty() {
                    Cow::Borrowed(arm)
                } else {
                    let mut function = arm.clone();
                    function.signature = substitute_generic_signature(arm, &arguments);
                    function.generic_parameters.clear();
                    Cow::Owned(function)
                },
                name,
                template: arm,
                item_ordinal,
                arm_index: Some(selection.arm_index),
            });
    }
    specializations
}

fn selected_overload_selections(
    items: &[ScalarItem],
    input: GenericInitializerInput<'_>,
) -> Vec<(Option<String>, String, ScalarOverloadSelection)> {
    if !matches!(input, GenericInitializerInput::RawItems) {
        return Vec::new();
    }
    let mut search = OverloadSearch {
        calls: Vec::new(),
        prepared: false,
    };
    for item in items {
        match item {
            ScalarItem::Binding(binding) => collect_selected_overloads(&binding.value, &mut search),
            ScalarItem::Function(function) => {
                collect_selected_overloads_in_block(&function.body, &mut search)
            }
            ScalarItem::Executable(item) => collect_selected_overloads_in_item(item, &mut search),
            _ => {}
        }
    }
    search
        .calls
        .into_iter()
        .filter_map(|expression| match expression {
            ScalarExpression::Call {
                receiver,
                name,
                overload_selection: Some(selection),
                ..
            } => Some((receiver.clone(), name.clone(), selection.clone())),
            _ => None,
        })
        .collect()
}

struct OverloadSearch<'a> {
    calls: Vec<&'a ScalarExpression>,
    prepared: bool,
}

fn collect_selected_overloads<'a>(
    expression: &'a ScalarExpression,
    selections: &mut OverloadSearch<'a>,
) {
    match expression {
        ScalarExpression::Call {
            arguments,
            overload_selection,
            ..
        } => {
            if selections.prepared || overload_selection.is_some() {
                selections.calls.push(expression);
            }
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

fn collect_selected_overloads_in_place<'a>(
    place: &'a crate::ScalarPlace,
    selections: &mut OverloadSearch<'a>,
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

fn collect_selected_overloads_in_block<'a>(
    block: &'a ScalarBlock,
    selections: &mut OverloadSearch<'a>,
) {
    for item in &block.items {
        collect_selected_overloads_in_item(item, selections);
    }
}

fn collect_selected_overloads_in_item<'a>(
    item: &'a ScalarBlockItem,
    selections: &mut OverloadSearch<'a>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            collect_selected_overloads(&binding.value, selections)
        }
        ScalarBlockItem::Expression(expression) => {
            collect_selected_overloads(expression, selections)
        }
        ScalarBlockItem::Assignment(assignment) => {
            let primary = if selections.prepared {
                &assignment.values[0]
            } else {
                &assignment.value
            };
            collect_selected_overloads(primary, selections)
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
    calls: &mut Vec<(String, Vec<ScalarType>)>,
) {
    match expression {
        ScalarExpression::Call {
            receiver: None,
            name,
            type_arguments,
            arguments,
            ..
        } => {
            calls.push((
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
        ScalarExpression::Call { arguments, .. } => {
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
    calls: &mut Vec<(String, Vec<ScalarType>)>,
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

fn collect_generic_calls_in_block(block: &ScalarBlock, calls: &mut Vec<(String, Vec<ScalarType>)>) {
    for item in &block.items {
        collect_generic_calls_in_item(item, calls);
    }
    for output in &block.final_output_values {
        collect_generic_calls(&output.value, calls);
    }
}

fn collect_generic_calls_in_item(
    item: &ScalarBlockItem,
    calls: &mut Vec<(String, Vec<ScalarType>)>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => collect_generic_calls(&binding.value, calls),
        ScalarBlockItem::Expression(expression) => collect_generic_calls(expression, calls),
        ScalarBlockItem::Assignment(assignment) => collect_generic_calls(&assignment.value, calls),
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

#[cfg(test)]
mod tests;
