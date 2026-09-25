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
    CfgPointKind, CoreOperationId, InvalidationCandidate, InvalidationInputs, ReferenceBindingKind,
    ReferenceOrigin, ReleaseTarget, ResolvedCallTarget, ScalarAllocationIdentity,
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
    return_body_span: Option<wosy_syntax::ByteSpan>,
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
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&validation.program.source.path));
    let builder = context.create_builder();
    let target_layout = validation.program.target_layout;
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
        for (index, name) in function.parameters.iter().enumerate() {
            if let Some(parameter) = value.get_nth_param(index as u32) {
                parameter.set_name(name);
            }
        }
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
        for (index, parameter) in function.parameters.iter().enumerate() {
            if let Some(argument) = value.get_nth_param(index as u32) {
                argument.set_name(parameter);
            }
        }
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
        for (index, parameter) in function.parameters.iter().enumerate() {
            if let Some(argument) = value.get_nth_param(index as u32) {
                argument.set_name(parameter);
            }
        }
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
    let mut runtime_array_results = BTreeMap::new();
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
            insert_runtime_array_result_provenance(&mut runtime_array_results, target, function);
            dynamic_owner_outputs.insert(target.clone(), dynamic_return_outputs(function));
        }
    }
    for (_, function, name) in specializations.iter().chain(overloads.iter()) {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
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
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
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
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
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
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
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
        &functions,
        &call_targets,
        &signatures,
        &runtime_array_results,
        &dynamic_owner_outputs,
        &globals,
        &validation.program.items,
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
                for (index, parameter) in function.parameters.iter().enumerate() {
                    if let Some(argument) = value.get_nth_param(index as u32) {
                        argument.set_name(parameter);
                    }
                }
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
        let specializations = generic_specializations(
            &source_module.items,
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
            for (index, parameter) in function.parameters.iter().enumerate() {
                if let Some(argument) = value.get_nth_param(index as u32) {
                    argument.set_name(parameter);
                }
            }
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
            for (index, parameter) in function.parameters.iter().enumerate() {
                if let Some(argument) = value.get_nth_param(index as u32) {
                    argument.set_name(parameter);
                }
            }
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
    let mut runtime_array_results = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for (_, function, name) in &definitions {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for (_, function, name) in &project_specializations {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for (_, function, name) in &project_overloads {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
        dynamic_owner_outputs.insert(name.clone(), dynamic_return_outputs(function));
    }
    for (source_module, function, name) in definitions {
        emit_project_function(
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
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
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
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
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
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
        &functions,
        &call_targets,
        &signatures,
        &runtime_array_results,
        &dynamic_owner_outputs,
        &modules,
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
    let (ordinary, _) = function_type(context, &function.signature, target_layout)?;
    if !dynamic_return_owner(function) {
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
    for _ in dynamic_return_outputs(function) {
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
            storage_type(context, ty, state.structs, state.target_layout)?,
            name,
        )?;
        store_value(context, state, slot, ty, value)?;
        state.storage.insert(name.clone(), (slot, ty.clone()));
        transition_runtime_array_binding_initialization(state, name, ty, &binding.value);
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
) {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return;
    }
    let provenance = runtime_array_assignment_provenance(state, value);
    let owns = provenance.result.owns();
    state.runtime_array_owners.insert(name.into(), owns);
    state.runtime_array_allocations.insert(name.into(), false);
    if owns {
        if let Some(source) = provenance.source_name.filter(|source| source != name) {
            state.runtime_array_owners.insert(source, false);
        }
    }
}

fn runtime_array_assignment_provenance(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
) -> RuntimeArrayAssignmentProvenance {
    match expression {
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
    }
}

fn insert_runtime_array_result_provenance(
    results: &mut BTreeMap<String, RuntimeArrayResultProvenance>,
    name: &str,
    function: &ScalarFunction,
) {
    if let Some(provenance) = function_runtime_array_result_provenance(function) {
        results.insert(name.into(), provenance);
    }
}

mod ownership;
use ownership::{
    binding_receivers, checked_call_owners, dynamic_return_outputs, dynamic_return_owner,
    emit_runtime_array_allocation, fresh_return_outputs, function_runtime_array_result_provenance,
    initialize_joined_return_bindings, joined_assignment_owners, joined_return_binding_types,
    release_runtime_array_owner, release_scope_runtime_array_owners, return_bindings,
    transfer_runtime_array_return,
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
        return_body_span: Some(function.body.span),
        checked_call_owners: checked_call_owners(function)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
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
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (position, output) in dynamic_return_outputs(function).into_iter().enumerate() {
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
    let result = match function.body.items.as_slice() {
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
        return_body_span: None,
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
                    );
                }
            }
            ScalarItem::Executable(item) => {
                emit_block_item(context, &mut state, item)?;
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Function(_) => {}
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
        return_body_span: Some(function.body.span),
        checked_call_owners: checked_call_owners(function)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
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
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (position, output) in dynamic_return_outputs(function).into_iter().enumerate() {
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
    let result = match function.body.items.as_slice() {
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
        return_body_span: None,
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
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    let root = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?;
    initialize_project_module(context, &mut state, root, modules, globals, &mut Vec::new())?;
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
    module: &'module ScalarModule,
    modules: &[&'module ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    initialized: &mut Vec<wosy_syntax::SourceIdentity>,
) -> Result<(), String> {
    if initialized.contains(&module.source) {
        return Ok(());
    }
    initialized.push(module.source.clone());

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
                    );
                }
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
    let mut retained_return_results = automatic_return_result_owners;
    retained_return_results.extend(std::mem::take(&mut state.automatic_return_result_owners));
    state.automatic_return_result_owners = retained_return_results;
    state.checked_owner_flags = checked_owner_flags;
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
    if !block.final_output_values.is_empty() {
        result = emit_project_final_outputs(context, state, block, module, modules)?;
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

fn emit_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        if matches!(&output.value, ScalarExpression::Call { .. }) {
            transfer_runtime_array_return(state, block, &output.value, &output.ty);
            transfer_automatic_return_result(state, &output.value);
            let value = emit_expression(context, state, &output.value)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, &output.value, output.position)?;
            }
            return Ok(value);
        }
        let value = emit_typed_expression(context, state, &output.value, &output.ty)?;
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, &output.value, output.position)?;
        }
        transfer_runtime_array_return(state, block, &output.value, &output.ty);
        transfer_automatic_return_result(state, &output.value);
        return Ok(value);
    }
    if let Some((condition, then_branch, else_branch)) = recombine_conditional_final_outputs(block)
    {
        for output in &block.final_output_values {
            transfer_runtime_array_return(state, block, &output.value, &output.ty);
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
        .map(|output| emit_typed_expression(context, state, &output.value, &output.ty))
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for (position, output) in block.final_output_values.iter().enumerate() {
            record_forwarded_return_owner(context, state, &output.value, position)?;
        }
    }
    for output in &block.final_output_values {
        transfer_runtime_array_return(state, block, &output.value, &output.ty);
        transfer_automatic_return_result(state, &output.value);
    }
    build_aggregate(context, state, &outputs, values)
}

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
            transfer_runtime_array_return(state, block, &output.value, &output.ty);
            transfer_automatic_return_result(state, &output.value);
            let value = emit_project_expression(context, state, &output.value, module, modules)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, &output.value, output.position)?;
            }
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
        transfer_runtime_array_return(state, block, &output.value, &output.ty);
        transfer_automatic_return_result(state, &output.value);
        return Ok(value);
    }
    if let Some((condition, then_branch, else_branch)) = recombine_conditional_final_outputs(block)
    {
        for output in &block.final_output_values {
            transfer_runtime_array_return(state, block, &output.value, &output.ty);
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
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for (position, output) in block.final_output_values.iter().enumerate() {
            record_forwarded_return_owner(context, state, &output.value, position)?;
        }
    }
    for output in &block.final_output_values {
        transfer_runtime_array_return(state, block, &output.value, &output.ty);
        transfer_automatic_return_result(state, &output.value);
    }
    build_aggregate(context, state, &outputs, values)
}

fn recombine_conditional_final_outputs(
    block: &ScalarBlock,
) -> Option<(ScalarExpression, ScalarBlock, ScalarBlock)> {
    let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        ..
    } = &block.final_output_values.first()?.value
    else {
        return None;
    };
    let conditional_span = block.final_output_values.first()?.span;
    let mut then_outputs = Vec::new();
    let mut else_outputs = Vec::new();
    for output in &block.final_output_values {
        let ScalarExpression::If {
            condition: output_condition,
            then_branch: output_then_branch,
            else_branch: output_else_branch,
            ..
        } = &output.value
        else {
            return None;
        };
        if output.span != conditional_span || output_condition != condition {
            return None;
        }
        let [then_output] = output_then_branch.final_output_values.as_slice() else {
            return None;
        };
        let [else_output] = output_else_branch.final_output_values.as_slice() else {
            return None;
        };
        let mut then_output = then_output.clone();
        then_output.ty = output.ty.clone();
        then_outputs.push(then_output);
        let mut else_output = else_output.clone();
        else_output.ty = output.ty.clone();
        else_outputs.push(else_output);
    }
    Some((
        (**condition).clone(),
        recombine_conditional_branch(then_branch, then_outputs),
        recombine_conditional_branch(else_branch, else_outputs),
    ))
}

fn recombine_conditional_branch(
    branch: &ScalarBlock,
    final_output_values: Vec<ScalarOutputValue>,
) -> ScalarBlock {
    let final_start = branch.items.len() - branch.final_output_values.len();
    let mut branch = branch.clone();
    branch.items.truncate(final_start);
    branch.terminated_items.truncate(final_start);
    branch.items.extend(
        final_output_values
            .iter()
            .map(|output| ScalarBlockItem::Expression(output.value.clone())),
    );
    branch
        .terminated_items
        .extend(final_output_values.iter().map(|_| false));
    branch.final_output_values = final_output_values;
    branch.expressions = branch
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    branch
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
    for (position, (target, (value, ty, provenance, call_flag))) in
        assignment.targets.iter().zip(values).enumerate()
    {
        if ty == ScalarType::Unit {
            state.values.insert(target.target.clone(), EmitValue::Unit);
            continue;
        }
        let destination = match &target.place {
            crate::ScalarPlace::Name { name, .. } => {
                match state.storage.get(name).map(|(slot, _)| *slot).or_else(|| {
                    state
                        .globals
                        .get(name)
                        .map(|(global, _)| global.as_pointer_value())
                }) {
                    Some(destination) => destination,
                    None => {
                        let slot = entry_alloca(
                            state,
                            storage_type(context, &ty, state.structs, state.target_layout)?,
                            name,
                        )?;
                        state.storage.insert(name.clone(), (slot, ty.clone()));
                        slot
                    }
                }
            }
            place => place_pointer(context, state, place, None)?,
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
            destination,
            &ty,
            value,
            call_flag,
        )?;
        transition_runtime_array_assignment(context, state, target, &ty, &provenance)?;
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
    for (position, (target, (value, ty, provenance, call_flag))) in
        assignment.targets.iter().zip(values).enumerate()
    {
        if ty == ScalarType::Unit {
            state.values.insert(target.target.clone(), EmitValue::Unit);
            continue;
        }
        let destination = if let Some(receiver) = target.receiver.as_ref().filter(|receiver| {
            module
                .namespace_bindings
                .iter()
                .any(|namespace| namespace.binding == **receiver)
        }) {
            let target_module = project_namespace_target(module, modules, receiver)?;
            let (global, _) = project_member_global(state, target_module, &target.target)?;
            global
                .ok_or_else(|| format!("unknown LLVM member storage {}", target.target))?
                .as_pointer_value()
        } else {
            match &target.place {
                crate::ScalarPlace::Name { name, .. } => {
                    match state.storage.get(name).map(|(slot, _)| *slot).or_else(|| {
                        state
                            .globals
                            .get(name)
                            .map(|(global, _)| global.as_pointer_value())
                    }) {
                        Some(destination) => destination,
                        None => {
                            let slot = entry_alloca(
                                state,
                                storage_type(context, &ty, state.structs, state.target_layout)?,
                                name,
                            )?;
                            state.storage.insert(name.clone(), (slot, ty.clone()));
                            slot
                        }
                    }
                }
                place => place_pointer(context, state, place, None)?,
            }
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
            destination,
            &ty,
            value,
            call_flag,
        )?;
        transition_runtime_array_assignment(context, state, target, &ty, &provenance)?;
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

fn transition_runtime_array_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    provenance: &RuntimeArrayAssignmentProvenance,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(());
    }
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(());
    };
    let source_is_target = provenance.source_name.as_deref() == Some(name);
    if !source_is_target {
        release_runtime_array_owner(context, state, name)?;
    }
    let owns = provenance.result.owns();
    state.runtime_array_owners.insert(name.clone(), owns);
    state.runtime_array_allocations.insert(name.clone(), false);
    if owns && !source_is_target {
        if let Some(source) = provenance.source_name.as_ref() {
            state.runtime_array_owners.insert(source.clone(), false);
        }
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
        RuntimeArrayAssignmentProvenance,
        Option<inkwell::values::IntValue<'ctx>>,
    )>,
    String,
> {
    let mut emit = emit;
    let mut position = 0;
    let mut values = Vec::new();
    for expression in expressions {
        let provenance = runtime_array_assignment_provenance(state, expression);
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
                values.push((EmitValue::Unit, ty, provenance.clone(), None));
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
            values.push((
                value,
                ty,
                provenance.clone(),
                call_flags.get(&output_position).copied(),
            ));
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

fn emit_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            if name == "null" {
                return Ok(EmitValue::Basic(
                    pointer_integer_type(context, state.target_layout)
                        .const_zero()
                        .into(),
                ));
            }
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                    return Ok(EmitValue::Basic(slot.into()));
                }
                if matches!(ty, ScalarType::RuntimeArray { .. })
                    && state
                        .runtime_array_allocations
                        .get(name)
                        .copied()
                        .unwrap_or(false)
                {
                    return Ok(EmitValue::Basic(slot.into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            storage_type(context, &ty, state.structs, state.target_layout)?,
                            slot,
                            name,
                        )
                        .map_err(builder_error)?,
                ));
            }
            if let Some((global, ty)) = state.globals.get(name).cloned() {
                if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                    return Ok(EmitValue::Basic(global.as_pointer_value().into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            storage_type(context, &ty, state.structs, state.target_layout)?,
                            global.as_pointer_value(),
                            name,
                        )
                        .map_err(builder_error)?,
                ));
            }
            state
                .values
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown LLVM value {name}"))
        }
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            if let Some(tag) = enum_tag {
                return Ok(EmitValue::Basic(
                    context.i32_type().const_int(u64::from(*tag), false).into(),
                ));
            }
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            Err(format!("unknown LLVM member {receiver}.{name}"))
        }
        ScalarExpression::Dereference { place, .. } => emit_place_value(context, state, place),
        ScalarExpression::IndexedRead { place, .. } => emit_place_value(context, state, place),
        ScalarExpression::Integer { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(
                    i32::try_from(value.clone()).map_err(|_| "invalid i32 literal")? as u64,
                    true,
                )
                .into(),
        )),
        ScalarExpression::InvalidInteger { .. } => Err("invalid integer literal".to_owned()),
        ScalarExpression::Float { .. } => {
            Err("floating-point literal requires an expected type".to_owned())
        }
        ScalarExpression::InvalidFloat { .. } => Err("invalid floating-point literal".to_owned()),
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Char { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(u64::from(*value as u32), false)
                .into(),
        )),
        ScalarExpression::Utf8 { .. } => {
            Err("string literal requires an expected std.utf8 type".to_owned())
        }
        ScalarExpression::Unary {
            operator, operand, ..
        } => {
            let value = take_basic(emit_expression(context, state, operand)?)?.into_int_value();
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => emit_binary(context, state, operator, left, right),
        ScalarExpression::Call {
            receiver,
            name,
            span,
            type_arguments,
            arguments,
            overload_selection,
            ..
        } => {
            if state.invalidations.contains_key(&(span.start, span.end)) {
                return emit_core_invalidation(context, state, *span, arguments, None);
            }
            if receiver.as_deref() == Some("core") && name == "alloc" {
                return emit_core_alloc(context, state, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "free" {
                return emit_core_free(context, state, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "system_panic" {
                return emit_core_system_panic(context, state, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return emit_int_conversion(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "uint_to_float"
                        | "sint_to_float"
                        | "float_to_sint_trunc"
                        | "float_to_uint_trunc"
                        | "float_trunc"
                        | "float_extend"
                )
            {
                return emit_float_conversion(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return emit_bitcast(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return emit_core_pointer_cast(context, state, name, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "offset" {
                return emit_core_offset(context, state, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core") && name == "load" {
                return emit_core_load(context, state, type_arguments, arguments);
            }
            emit_call(
                context,
                state,
                receiver.as_deref(),
                name,
                type_arguments,
                arguments,
                overload_selection.as_ref(),
            )
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => emit_if(context, state, condition, then_branch, else_branch),
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => emit_unit_if(context, state, condition, then_branch),
        ScalarExpression::Block(block) => emit_block(context, state, block),
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => Ok(EmitValue::Basic(
            state
                .builder
                .build_ptr_to_int(
                    place_pointer(context, state, place, None)?,
                    pointer_integer_type(context, state.target_layout),
                    "address",
                )
                .map_err(builder_error)?
                .into(),
        )),
        ScalarExpression::StructLiteral { .. } => {
            return Err("struct literal requires an expected struct type".to_owned())
        }
        ScalarExpression::ArrayLiteral { .. } => {
            return Err("array literal requires an expected fixed array type".to_owned())
        }
    }
}

fn emit_project_typed_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    expected: &ScalarType,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Utf8 { value, .. } => emit_utf8_literal(context, state, value, expected),
        ScalarExpression::Unary {
            operator, operand, ..
        } if integer_width(expected).is_some() || *expected == ScalarType::Bool => {
            let value = take_basic(emit_project_typed_expression(
                context, state, operand, expected, module, modules,
            )?)?
            .into_int_value();
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Integer { value, .. } if integer_width(expected).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, expected, value)?.into()),
        ),
        ScalarExpression::Float { value, .. } => Ok(EmitValue::Basic(
            float_constant(context, expected, *value)?.into(),
        )),
        ScalarExpression::StructLiteral { fields, .. } => {
            emit_struct_literal(context, state, expected, fields)
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            emit_array_literal(context, state, expected, elements)
        }
        _ => emit_project_expression(context, state, expression, module, modules),
    }
}

fn emit_project_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            if let Some(tag) = enum_tag {
                return Ok(EmitValue::Basic(
                    context.i32_type().const_int(u64::from(*tag), false).into(),
                ));
            }
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            let target = project_namespace_target(module, modules, receiver)?;
            let (global, ty) = project_member_global(state, target, name)?;
            if ty == ScalarType::Unit {
                return Ok(EmitValue::Unit);
            }
            let global = global.ok_or_else(|| format!("unknown LLVM member storage {name}"))?;
            if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                return Ok(EmitValue::Basic(global.as_pointer_value().into()));
            }
            Ok(EmitValue::Basic(
                state
                    .builder
                    .build_load(
                        storage_type(context, &ty, state.structs, state.target_layout)?,
                        global.as_pointer_value(),
                        name,
                    )
                    .map_err(builder_error)?,
            ))
        }
        ScalarExpression::Dereference { place, .. } => emit_place_value_in_project(
            context,
            state,
            place,
            Some(ProjectCallScope { module, modules }),
        ),
        ScalarExpression::IndexedRead { place, .. } => emit_place_value_in_project(
            context,
            state,
            place,
            Some(ProjectCallScope { module, modules }),
        ),
        ScalarExpression::Call {
            receiver,
            name,
            span,
            type_arguments,
            arguments,
            overload_selection,
            ..
        } => {
            if state.invalidations.contains_key(&(span.start, span.end)) {
                return emit_core_invalidation(
                    context,
                    state,
                    *span,
                    arguments,
                    Some(ProjectCallScope { module, modules }),
                );
            }
            if receiver.as_deref() == Some("core") && name == "alloc" {
                return emit_core_alloc_project(context, state, arguments, module, modules);
            }
            if receiver.as_deref() == Some("core") && name == "free" {
                return emit_core_free_project(context, state, arguments, module, modules);
            }
            if receiver.as_deref() == Some("core") && name == "system_panic" {
                return emit_core_system_panic(context, state, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return emit_int_conversion_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "uint_to_float"
                        | "sint_to_float"
                        | "float_to_sint_trunc"
                        | "float_to_uint_trunc"
                        | "float_trunc"
                        | "float_extend"
                )
            {
                return emit_float_conversion_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return emit_bitcast_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return emit_core_pointer_cast_project(
                    context,
                    state,
                    name,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "offset" {
                return emit_core_offset_project(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            if receiver.as_deref() == Some("core") && name == "load" {
                return emit_core_load_project(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                );
            }
            let target = receiver.as_ref().map_or(module, |binding| {
                module
                    .namespace_bindings
                    .iter()
                    .find(|namespace| namespace.binding == *binding)
                    .and_then(|namespace| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == namespace.target)
                            .copied()
                    })
                    .unwrap_or(module)
            });
            let qualified = receiver
                .as_deref()
                .filter(|binding| {
                    target.items.iter().any(|item| {
                        matches!(
                            item,
                            ScalarItem::Extern(extern_decl)
                                if extern_decl.binding == *binding
                                    && extern_decl.functions.iter().any(|function| function.name == *name)
                        )
                    })
                })
                .map_or_else(
                    || project_function_name(&target.source, name),
                    |binding| {
                        let import_module = target
                            .items
                            .iter()
                            .find_map(|item| match item {
                                ScalarItem::Extern(extern_decl)
                                    if extern_decl.binding == binding
                                        && extern_decl.functions.iter().any(|function| function.name == *name) =>
                                {
                                    match &extern_decl.actual_module {
                                        crate::scalar::ScalarExternModule::Valid(import_module) => {
                                            Some(import_module.as_str())
                                        }
                                        crate::scalar::ScalarExternModule::Invalid { .. } => None,
                                    }
                                }
                                _ => None,
                            })
                            .expect("validated extern module");
                        project_extern_lookup_key(
                            &target.source,
                            binding,
                            import_module,
                            name,
                        )
                    },
                );
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
                .or_else(|| state.call_targets.get(&qualified))
                .ok_or_else(|| format!("unknown LLVM callable {qualified}"))?
                .clone();
            let ScalarType::Callable { parameters, .. } = state
                .signatures
                .get(&target)
                .ok_or_else(|| format!("unknown LLVM callable signature {qualified}"))?
            else {
                return Err(format!(
                    "LLVM callable {qualified} has no callable signature"
                ));
            };
            let values = arguments
                .iter()
                .zip(parameters)
                .map(|(argument, parameter)| {
                    emit_project_typed_expression(
                        context, state, argument, parameter, module, modules,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            emit_call_values(context, state, &target, values)
        }
        ScalarExpression::Name { name, .. } => emit_expression(
            context,
            state,
            &ScalarExpression::Name {
                name: name.clone(),
                span: wosy_syntax::ByteSpan::new(0, 0),
            },
        ),
        ScalarExpression::Integer { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(
                    i32::try_from(value.clone()).map_err(|_| "invalid i32 literal")? as u64,
                    true,
                )
                .into(),
        )),
        ScalarExpression::InvalidInteger { .. } => Err("invalid integer literal".to_owned()),
        ScalarExpression::Float { .. } => {
            Err("floating-point literal requires an expected type".to_owned())
        }
        ScalarExpression::InvalidFloat { .. } => Err("invalid floating-point literal".to_owned()),
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Char { value, .. } => Ok(EmitValue::Basic(
            context
                .i32_type()
                .const_int(u64::from(*value as u32), false)
                .into(),
        )),
        ScalarExpression::Utf8 { .. } => {
            Err("string literal requires an expected std.utf8 type".to_owned())
        }
        ScalarExpression::Unary {
            operator, operand, ..
        } => {
            let value = take_basic(emit_project_expression(
                context, state, operand, module, modules,
            )?)?
            .into_int_value();
            emit_unary_value(state, operator, value)
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            let unsigned = project_expression_is_unsigned(state, left, module, modules);
            let left = emit_project_expression(context, state, left, module, modules)?;
            emit_binary_with_rhs(
                context,
                state,
                operator,
                left,
                right,
                ExpressionPath::Project { module, modules },
                unsigned,
            )
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let condition = emit_project_expression(context, state, condition, module, modules)?;
            emit_project_if(
                context,
                state,
                condition,
                then_branch,
                else_branch,
                module,
                modules,
            )
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            let condition = emit_project_expression(context, state, condition, module, modules)?;
            emit_project_unit_if(context, state, condition, then_branch, module, modules)
        }
        ScalarExpression::Block(block) => {
            emit_project_block(context, state, block, module, modules)
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => Ok(EmitValue::Basic(
            state
                .builder
                .build_ptr_to_int(
                    place_pointer(
                        context,
                        state,
                        place,
                        Some(ProjectCallScope { module, modules }),
                    )?,
                    pointer_integer_type(context, state.target_layout),
                    "address",
                )
                .map_err(builder_error)?
                .into(),
        )),
        ScalarExpression::StructLiteral { .. } => {
            return Err("struct literal requires an expected struct type".to_owned())
        }
        ScalarExpression::ArrayLiteral { .. } => {
            return Err("array literal requires an expected fixed array type".to_owned())
        }
    }
}

fn emit_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    receiver: Option<&str>,
    name: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    overload_selection: Option<&ScalarOverloadSelection>,
) -> Result<EmitValue<'ctx>, String> {
    let qualified =
        receiver.map_or_else(|| name.to_owned(), |receiver| format!("{receiver}.{name}"));
    let lookup =
        overload_selection.map(|selection| selected_overload_lookup_key(&qualified, selection));
    let target = state
        .call_targets
        .get(lookup.as_ref().unwrap_or(&qualified))
        .or_else(|| {
            state
                .call_targets
                .get(&specialization_lookup_key(&qualified, type_arguments))
        })
        .or_else(|| state.call_targets.get(&qualified))
        .ok_or_else(|| format!("unknown LLVM callable {qualified}"))?
        .clone();
    let ScalarType::Callable { parameters, .. } = state
        .signatures
        .get(&target)
        .ok_or_else(|| format!("unknown LLVM callable signature {qualified}"))?
    else {
        return Err(format!(
            "LLVM callable {qualified} has no callable signature"
        ));
    };
    let values = arguments
        .iter()
        .zip(parameters)
        .map(|(argument, parameter)| emit_typed_expression(context, state, argument, parameter))
        .collect::<Result<Vec<_>, _>>()?;
    emit_call_values(context, state, &target, values)
}

fn allocation_root_name(place: &crate::ScalarPlace) -> Result<&str, String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => Ok(name),
        crate::ScalarPlace::Index { base, .. } | crate::ScalarPlace::Field { base, .. } => {
            allocation_root_name(base)
        }
        crate::ScalarPlace::Dereference { .. } => {
            Err("core allocation place requires a named owner".to_owned())
        }
    }
}

fn core_effect_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    address: inkwell::values::IntValue<'ctx>,
    candidates: &[InvalidationCandidate],
    project: Option<ProjectCallScope<'_>>,
) -> Result<Vec<(String, inkwell::values::IntValue<'ctx>)>, String> {
    let distinct = candidates
        .iter()
        .map(|candidate| &candidate.place.binding)
        .collect::<BTreeSet<_>>();
    let multiple = distinct.len() > 1;
    let mut owners = BTreeMap::new();
    for candidate in candidates {
        let name = allocation_root_name(&candidate.place.place)?.to_owned();
        let selected = if multiple {
            let pointer = place_pointer(context, state, &candidate.place.place, project)?;
            let pointer = state
                .builder
                .build_ptr_to_int(pointer, address.get_type(), "candidate_address")
                .map_err(builder_error)?;
            state
                .builder
                .build_int_compare(IntPredicate::EQ, pointer, address, "core_effect_target")
                .map_err(builder_error)?
        } else {
            context.bool_type().const_int(1, false)
        };
        if let Some((_, previous)) = owners.get_mut(&candidate.place.binding) {
            *previous = state
                .builder
                .build_or(*previous, selected, "core_effect_owner")
                .map_err(builder_error)?;
        } else {
            owners.insert(candidate.place.binding.clone(), (name, selected));
        }
    }
    Ok(owners.into_values().collect())
}

fn emit_core_invalidation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    span: wosy_syntax::ByteSpan,
    arguments: &[ScalarExpression],
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let (operation, candidates, inputs) = state
        .invalidations
        .get(&(span.start, span.end))
        .cloned()
        .ok_or_else(|| "core invalidation lacks a typed CFG event".to_owned())?;
    let address = take_basic(match project {
        Some(scope) => {
            emit_project_expression(context, state, &arguments[0], scope.module, scope.modules)?
        }
        None => emit_expression(context, state, &arguments[0])?,
    })?
    .into_int_value();
    if operation == CoreOperationId::Invalidate {
        if !matches!(inputs, InvalidationInputs::Invalidate) {
            return Err("inconsistent core.invalidate inputs".to_owned());
        }
        let owners = core_effect_owners(context, state, address, &candidates, project)?;
        let multiple = owners.len() > 1;
        for (name, selected) in owners {
            let live = *state
                .runtime_array_live
                .get(&name)
                .ok_or_else(|| format!("missing allocation state for {name}"))?;
            if multiple {
                let (_, ty) = state
                    .storage
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("missing allocation storage for {name}"))?;
                if !matches!(ty, ScalarType::RuntimeArray { .. }) {
                    return Err(format!("invalid allocation target {name}"));
                }
                let block = state
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| "missing invalidation block".to_owned())?;
                let function = block
                    .get_parent()
                    .ok_or_else(|| "missing invalidation function".to_owned())?;
                let update = context.append_basic_block(function, "invalidate.update");
                let next = context.append_basic_block(function, "invalidate.next");
                state
                    .builder
                    .build_conditional_branch(selected, update, next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(update);
                state
                    .builder
                    .build_store(live, context.bool_type().const_zero())
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_unconditional_branch(next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(next);
            } else {
                state
                    .builder
                    .build_store(live, context.bool_type().const_zero())
                    .map_err(builder_error)?;
            }
        }
        return Ok(EmitValue::Unit);
    }
    if operation == CoreOperationId::Free {
        if !matches!(inputs, InvalidationInputs::Free) || arguments.len() != 1 {
            return Err("inconsistent checked core.free inputs".to_owned());
        }
        declare_core_runtime(context, state.module);
        let deallocator = state
            .module
            .get_function(CORE_FREE_SYMBOL)
            .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
        let owners = core_effect_owners(context, state, address, &candidates, project)?;
        let multiple = owners.len() > 1;
        for (name, selected) in owners {
            let (slot, ty) = state
                .storage
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("missing checked release storage for {name}"))?;
            if !matches!(ty, ScalarType::RuntimeArray { .. })
                || state.runtime_array_allocations.get(&name) != Some(&false)
            {
                return Err(format!(
                    "checked core.free requires an allocation owner {name}"
                ));
            }
            let live_slot = *state
                .runtime_array_live
                .get(&name)
                .ok_or_else(|| format!("missing checked release owner state for {name}"))?;
            if multiple {
                let block = state
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| "missing checked release block".to_owned())?;
                let function = block
                    .get_parent()
                    .ok_or_else(|| "missing checked release function".to_owned())?;
                let update = context.append_basic_block(function, "checked_release.selected");
                let next = context.append_basic_block(function, "checked_release.next");
                state
                    .builder
                    .build_conditional_branch(selected, update, next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(update);
                let live = state
                    .builder
                    .build_load(context.bool_type(), live_slot, "checked_release_live")
                    .map_err(builder_error)?
                    .into_int_value();
                let release = context.append_basic_block(function, "checked_release.live");
                state
                    .builder
                    .build_conditional_branch(live, release, next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(release);
                let backing = state
                    .builder
                    .build_load(
                        context.ptr_type(AddressSpace::default()),
                        slot,
                        "checked_release_backing",
                    )
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_call(deallocator, &[backing.into()], "checked_release")
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_store(live_slot, context.bool_type().const_zero())
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_unconditional_branch(next)
                    .map_err(builder_error)?;
                state.builder.position_at_end(next);
            } else {
                let backing = state
                    .builder
                    .build_load(
                        context.ptr_type(AddressSpace::default()),
                        slot,
                        "checked_release_backing",
                    )
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_call(deallocator, &[backing.into()], "checked_release")
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_store(live_slot, context.bool_type().const_zero())
                    .map_err(builder_error)?;
            }
        }
        return Ok(EmitValue::Unit);
    }
    let InvalidationInputs::Rebind {
        raw_address,
        length,
        ..
    } = inputs
    else {
        return Err("inconsistent core.rebind inputs".to_owned());
    };
    if operation != CoreOperationId::Rebind
        || arguments.len() != 3
        || arguments[1] != raw_address
        || arguments[2] != length
    {
        return Err("core.rebind inputs differ from resolved CFG event".to_owned());
    }
    let raw = take_basic(match project {
        Some(scope) => {
            emit_project_expression(context, state, &raw_address, scope.module, scope.modules)?
        }
        None => emit_expression(context, state, &raw_address)?,
    })?
    .into_int_value();
    let length = take_basic(match project {
        Some(scope) => emit_project_typed_expression(
            context,
            state,
            &length,
            &ScalarType::U64,
            scope.module,
            scope.modules,
        )?,
        None => emit_typed_expression(context, state, &length, &ScalarType::U64)?,
    })?
    .into_int_value();
    let pointer = state
        .builder
        .build_int_to_ptr(
            raw,
            context.ptr_type(AddressSpace::default()),
            "rebound_address",
        )
        .map_err(builder_error)?;
    let owners = core_effect_owners(context, state, address, &candidates, project)?;
    let multiple = owners.len() > 1;
    for (name, selected) in owners {
        let (slot, ty) = state
            .storage
            .get(&name)
            .cloned()
            .ok_or_else(|| format!("missing rebind storage for {name}"))?;
        if !matches!(ty, ScalarType::RuntimeArray { .. })
            || state.runtime_array_allocations.get(&name) != Some(&false)
        {
            return Err(format!(
                "core.rebind requires writable runtime allocation {name}"
            ));
        }
        let length_slot = *state
            .runtime_array_lengths
            .get(&name)
            .ok_or_else(|| format!("missing runtime length for {name}"))?;
        if multiple {
            let current = state
                .builder
                .get_insert_block()
                .ok_or_else(|| "missing rebind block".to_owned())?;
            let function = current
                .get_parent()
                .ok_or_else(|| "missing rebind function".to_owned())?;
            let update = context.append_basic_block(function, "rebind.update");
            let next = context.append_basic_block(function, "rebind.next");
            state
                .builder
                .build_conditional_branch(selected, update, next)
                .map_err(builder_error)?;
            state.builder.position_at_end(update);
            state
                .builder
                .build_store(slot, pointer)
                .map_err(builder_error)?;
            state
                .builder
                .build_store(length_slot, length)
                .map_err(builder_error)?;
            state
                .builder
                .build_unconditional_branch(next)
                .map_err(builder_error)?;
            state.builder.position_at_end(next);
        } else {
            state
                .builder
                .build_store(slot, pointer)
                .map_err(builder_error)?;
            state
                .builder
                .build_store(length_slot, length)
                .map_err(builder_error)?;
        }
    }
    Ok(EmitValue::Unit)
}

fn emit_core_alloc<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [size, alignment] = arguments else {
        return Err("core.alloc has invalid argument arity".to_owned());
    };
    let size = take_basic(emit_typed_expression(
        context,
        state,
        size,
        &ScalarType::U64,
    )?)?;
    let alignment = take_basic(emit_typed_expression(
        context,
        state,
        alignment,
        &ScalarType::U64,
    )?)?;
    emit_core_alloc_values(context, state, size, alignment)
}

fn emit_core_alloc_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [size, alignment] = arguments else {
        return Err("core.alloc has invalid argument arity".to_owned());
    };
    let size = take_basic(emit_project_typed_expression(
        context,
        state,
        size,
        &ScalarType::U64,
        module,
        modules,
    )?)?;
    let alignment = take_basic(emit_project_typed_expression(
        context,
        state,
        alignment,
        &ScalarType::U64,
        module,
        modules,
    )?)?;
    emit_core_alloc_values(context, state, size, alignment)
}

fn emit_core_alloc_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    size: BasicValueEnum<'ctx>,
    alignment: BasicValueEnum<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let allocator = state
        .module
        .get_function(CORE_ALLOC_SYMBOL)
        .ok_or_else(|| "core allocation runtime declaration is missing".to_owned())?;
    let pointer = state
        .builder
        .build_call(allocator, &[size.into(), alignment.into()], "allocation")
        .map_err(builder_error)?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "core allocation runtime returned no pointer".to_owned())?
        .into_pointer_value();
    let address = state
        .builder
        .build_ptr_to_int(
            pointer,
            pointer_integer_type(context, state.target_layout),
            "allocation_address",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(address.into()))
}

fn emit_core_free<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [pointer] = arguments else {
        return Err("core.free has invalid argument arity".to_owned());
    };
    let value = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
    emit_core_free_value(context, state, value)
}

fn emit_core_free_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let [pointer] = arguments else {
        return Err("core.free has invalid argument arity".to_owned());
    };
    let value = take_basic(emit_project_expression(
        context, state, pointer, module, modules,
    )?)?
    .into_int_value();
    emit_core_free_value(context, state, value)
}

fn emit_core_free_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: inkwell::values::IntValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    let deallocator = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    let pointer = state
        .builder
        .build_int_to_ptr(
            value,
            context.ptr_type(AddressSpace::default()),
            "free_pointer",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_call(deallocator, &[pointer.into()], "free")
        .map_err(builder_error)?;
    Ok(EmitValue::Unit)
}

fn emit_core_system_panic<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    if !arguments.is_empty() {
        return Err("core.system_panic has invalid argument arity".to_owned());
    }
    declare_core_runtime(context, state.module);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(panic, &[], "system_panic")
        .map_err(builder_error)?;
    Ok(EmitValue::Unit)
}

fn emit_int_conversion<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = integer_conversion_source_type(state, value, None)?;
    let destination = &type_argument.ty;
    let value =
        take_basic(emit_typed_expression(context, state, value, &source)?)?.into_int_value();
    let converted = if operation == "int_trunc" {
        state
            .builder
            .build_int_truncate(value, integer_type(context, destination)?, "int_trunc")
    } else if is_signed_integer_type(&source) {
        state
            .builder
            .build_int_s_extend(value, integer_type(context, destination)?, "int_extend")
    } else {
        state
            .builder
            .build_int_z_extend(value, integer_type(context, destination)?, "int_extend")
    }
    .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted.into()))
}

fn is_bitcast_scalar_type(ty: &ScalarType) -> bool {
    integer_width(ty).is_some()
        || matches!(
            ty,
            ScalarType::Char | ScalarType::Bool | ScalarType::F32 | ScalarType::F64
        )
}

fn bitcast_source_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    project_module: Option<&ScalarModule>,
) -> Result<ScalarType, String> {
    let ty = match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone())),
        ScalarExpression::Member { receiver, name, .. } => {
            match struct_member_place(state, receiver, name)? {
                Some(crate::ScalarPlace::Field {
                    field: ScalarFieldReference::Resolved(field),
                    ..
                }) => structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == field)
                    .map(|field| field.ty.clone()),
                Some(_) | None => None,
            }
        }
        ScalarExpression::Dereference { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Integer { .. } => Some(ScalarType::I32),
        ScalarExpression::Char { .. } => Some(ScalarType::Char),
        ScalarExpression::Boolean { .. } => Some(ScalarType::Bool),
        ScalarExpression::Float { .. } => Some(ScalarType::F64),
        ScalarExpression::Unary { operand, .. } => {
            bitcast_source_type(state, operand, project_module).ok()
        }
        ScalarExpression::Binary { left, .. } => {
            bitcast_source_type(state, left, project_module).ok()
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } if receiver.as_deref() == Some("core")
            && matches!(
                name.as_str(),
                "int_trunc"
                    | "int_extend"
                    | "uint_to_float"
                    | "sint_to_float"
                    | "float_to_sint_trunc"
                    | "float_to_uint_trunc"
                    | "float_trunc"
                    | "float_extend"
                    | "bitcast"
            ) =>
        {
            type_arguments.first().map(|argument| argument.ty.clone())
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver.as_ref().map_or_else(
                || {
                    project_module.map_or_else(
                        || name.clone(),
                        |module| project_function_name(&module.source, name),
                    )
                },
                |receiver| format!("{receiver}.{name}"),
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
                .map(|output| output.ty.clone())
        }
        _ => None,
    };
    ty.filter(|ty| is_bitcast_scalar_type(ty))
        .ok_or_else(|| "core bitcast has an unknown source type".to_owned())
}

fn emit_bitcast<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = bitcast_source_type(state, value, None)?;
    let destination = &type_argument.ty;
    let value = take_basic(emit_typed_expression(context, state, value, &source)?)?;
    let converted = state
        .builder
        .build_bit_cast(
            value,
            basic_type(context, destination, state.target_layout)?,
            "bitcast",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted))
}
fn emit_int_conversion_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = integer_conversion_source_type(state, value, Some(module))?;
    let destination = &type_argument.ty;
    let value = take_basic(emit_project_typed_expression(
        context, state, value, &source, module, modules,
    )?)?
    .into_int_value();
    let converted = if operation == "int_trunc" {
        state
            .builder
            .build_int_truncate(value, integer_type(context, destination)?, "int_trunc")
    } else if is_signed_integer_type(&source) {
        state
            .builder
            .build_int_s_extend(value, integer_type(context, destination)?, "int_extend")
    } else {
        state
            .builder
            .build_int_z_extend(value, integer_type(context, destination)?, "int_extend")
    }
    .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted.into()))
}

fn emit_bitcast_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let source = bitcast_source_type(state, value, Some(module))?;
    let destination = &type_argument.ty;
    let value = take_basic(emit_project_typed_expression(
        context, state, value, &source, module, modules,
    )?)?;
    let converted = state
        .builder
        .build_bit_cast(
            value,
            basic_type(context, destination, state.target_layout)?,
            "bitcast",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted))
}
fn emit_core_pointer_cast<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let value = take_basic(emit_expression(context, state, value)?)?;
    emit_core_pointer_cast_values(context, state, value, &type_argument.ty)
}

fn emit_core_pointer_cast_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let value = take_basic(emit_project_expression(
        context, state, value, module, modules,
    )?)?;
    emit_core_pointer_cast_values(context, state, value, &type_argument.ty)
}

fn emit_core_pointer_cast_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: BasicValueEnum<'ctx>,
    destination: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let converted = state
        .builder
        .build_bit_cast(
            value,
            basic_type(context, destination, state.target_layout)?,
            "pointer_cast",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted))
}
fn emit_float_conversion<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let destination = &type_argument.ty;
    match operation {
        "uint_to_float" | "sint_to_float" => {
            let source = integer_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_int_value();
            let converted = if is_signed_integer_type(&source) {
                state.builder.build_signed_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_unsigned_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_to_sint_trunc" | "float_to_uint_trunc" => {
            let source = float_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_float_value();
            let converted = if is_signed_integer_type(destination) {
                state.builder.build_float_to_signed_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_float_to_unsigned_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_trunc" => {
            let source = float_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_float_value();
            let converted = state
                .builder
                .build_float_trunc(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_extend" => {
            let source = float_conversion_source_type(state, value, None)?;
            let value = take_basic(emit_typed_expression(context, state, value, &source)?)?
                .into_float_value();
            let converted = state
                .builder
                .build_float_ext(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        _ => Err(format!(
            "core.{operation} is not a floating-point conversion"
        )),
    }
}

fn emit_float_conversion_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operation: &str,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err(format!("core.{operation} has invalid type argument arity"));
    };
    let [value] = arguments else {
        return Err(format!("core.{operation} has invalid argument arity"));
    };
    let destination = &type_argument.ty;
    match operation {
        "uint_to_float" | "sint_to_float" => {
            let source = integer_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_int_value();
            let converted = if is_signed_integer_type(&source) {
                state.builder.build_signed_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_unsigned_int_to_float(
                    value,
                    float_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_to_sint_trunc" | "float_to_uint_trunc" => {
            let source = float_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_float_value();
            let converted = if is_signed_integer_type(destination) {
                state.builder.build_float_to_signed_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            } else {
                state.builder.build_float_to_unsigned_int(
                    value,
                    integer_type(context, destination)?,
                    operation,
                )
            }
            .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_trunc" => {
            let source = float_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_float_value();
            let converted = state
                .builder
                .build_float_trunc(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        "float_extend" => {
            let source = float_conversion_source_type(state, value, Some(module))?;
            let value = take_basic(emit_project_typed_expression(
                context, state, value, &source, module, modules,
            )?)?
            .into_float_value();
            let converted = state
                .builder
                .build_float_ext(value, float_type(context, destination)?, operation)
                .map_err(builder_error)?;
            Ok(EmitValue::Basic(converted.into()))
        }
        _ => Err(format!(
            "core.{operation} is not a floating-point conversion"
        )),
    }
}

fn emit_core_offset<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.offset has invalid type argument arity".to_owned());
    };
    let [pointer, count] = arguments else {
        return Err("core.offset has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
    let count = take_basic(emit_typed_expression(
        context,
        state,
        count,
        &ScalarType::I64,
    )?)?
    .into_int_value();
    emit_core_offset_values(context, state, pointer, count, &type_argument.ty)
}

fn emit_core_offset_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.offset has invalid type argument arity".to_owned());
    };
    let [pointer, count] = arguments else {
        return Err("core.offset has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_project_expression(
        context, state, pointer, module, modules,
    )?)?
    .into_int_value();
    let count = take_basic(emit_project_typed_expression(
        context,
        state,
        count,
        &ScalarType::I64,
        module,
        modules,
    )?)?
    .into_int_value();
    emit_core_offset_values(context, state, pointer, count, &type_argument.ty)
}

fn emit_core_offset_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    pointer: inkwell::values::IntValue<'ctx>,
    count: inkwell::values::IntValue<'ctx>,
    pointee: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let element = storage_type(context, pointee, state.structs, state.target_layout)?;
    let base = state
        .builder
        .build_int_to_ptr(
            pointer,
            context.ptr_type(AddressSpace::default()),
            "offset_base",
        )
        .map_err(builder_error)?;
    let advanced = unsafe {
        state
            .builder
            .build_in_bounds_gep(element, base, &[count], "offset")
    }
    .map_err(builder_error)?;
    let address = state
        .builder
        .build_ptr_to_int(
            advanced,
            pointer_integer_type(context, state.target_layout),
            "offset_address",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(address.into()))
}

fn emit_core_load<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.load has invalid type argument arity".to_owned());
    };
    let [pointer] = arguments else {
        return Err("core.load has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
    emit_core_load_values(context, state, pointer, &type_argument.ty)
}

fn emit_core_load_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let [type_argument] = type_arguments else {
        return Err("core.load has invalid type argument arity".to_owned());
    };
    let [pointer] = arguments else {
        return Err("core.load has invalid argument arity".to_owned());
    };
    let pointer = take_basic(emit_project_expression(
        context, state, pointer, module, modules,
    )?)?
    .into_int_value();
    emit_core_load_values(context, state, pointer, &type_argument.ty)
}

fn emit_core_load_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    pointer: inkwell::values::IntValue<'ctx>,
    pointee: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let base = state
        .builder
        .build_int_to_ptr(
            pointer,
            context.ptr_type(AddressSpace::default()),
            "load_base",
        )
        .map_err(builder_error)?;
    if matches!(
        pointee,
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
    ) {
        return Ok(EmitValue::Basic(base.into()));
    }
    let value = state
        .builder
        .build_load(
            basic_type(context, pointee, state.target_layout)?,
            base,
            "load",
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(value))
}

fn is_signed_integer_type(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128
    )
}

fn is_float_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::F32 | ScalarType::F64)
}

fn float_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<inkwell::types::FloatType<'ctx>, String> {
    match ty {
        ScalarType::F32 => Ok(context.f32_type()),
        ScalarType::F64 => Ok(context.f64_type()),
        _ => Err("expected floating-point type".to_owned()),
    }
}

fn integer_conversion_source_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    project_module: Option<&ScalarModule>,
) -> Result<ScalarType, String> {
    let ty = match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone())),
        ScalarExpression::Member { receiver, name, .. } => {
            match struct_member_place(state, receiver, name)? {
                Some(crate::ScalarPlace::Field {
                    field: ScalarFieldReference::Resolved(field),
                    ..
                }) => structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == field)
                    .map(|field| field.ty.clone()),
                Some(_) | None => None,
            }
        }
        ScalarExpression::Dereference { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Integer { .. } => Some(ScalarType::I32),
        ScalarExpression::Unary { operand, .. } => {
            integer_conversion_source_type(state, operand, project_module).ok()
        }
        ScalarExpression::Binary { left, .. } => {
            integer_conversion_source_type(state, left, project_module).ok()
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } if receiver.as_deref() == Some("core")
            && matches!(
                name.as_str(),
                "int_trunc"
                    | "int_extend"
                    | "uint_to_float"
                    | "sint_to_float"
                    | "float_to_sint_trunc"
                    | "float_to_uint_trunc"
                    | "float_trunc"
                    | "float_extend"
            ) =>
        {
            type_arguments.first().map(|argument| argument.ty.clone())
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver.as_ref().map_or_else(
                || {
                    project_module.map_or_else(
                        || name.clone(),
                        |module| project_function_name(&module.source, name),
                    )
                },
                |receiver| format!("{receiver}.{name}"),
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
                .map(|output| output.ty.clone())
        }
        _ => None,
    };
    ty.filter(|ty| integer_width(ty).is_some())
        .ok_or_else(|| "core integer conversion has an unknown source type".to_owned())
}

fn float_conversion_source_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    project_module: Option<&ScalarModule>,
) -> Result<ScalarType, String> {
    let ty = match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone())),
        ScalarExpression::Member { receiver, name, .. } => {
            match struct_member_place(state, receiver, name)? {
                Some(crate::ScalarPlace::Field {
                    field: ScalarFieldReference::Resolved(field),
                    ..
                }) => structure(state.structs, field.structure.clone())?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == field)
                    .map(|field| field.ty.clone()),
                Some(_) | None => None,
            }
        }
        ScalarExpression::Dereference { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Integer { .. } => Some(ScalarType::I32),
        ScalarExpression::Float { .. } => Some(ScalarType::F64),
        ScalarExpression::Unary { operand, .. } => {
            float_conversion_source_type(state, operand, project_module).ok()
        }
        ScalarExpression::Binary { left, .. } => {
            float_conversion_source_type(state, left, project_module).ok()
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } if receiver.as_deref() == Some("core")
            && matches!(
                name.as_str(),
                "int_trunc"
                    | "int_extend"
                    | "uint_to_float"
                    | "sint_to_float"
                    | "float_to_sint_trunc"
                    | "float_to_uint_trunc"
                    | "float_trunc"
                    | "float_extend"
            ) =>
        {
            type_arguments.first().map(|argument| argument.ty.clone())
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            overload_selection,
            ..
        } => {
            let qualified = receiver.as_ref().map_or_else(
                || {
                    project_module.map_or_else(
                        || name.clone(),
                        |module| project_function_name(&module.source, name),
                    )
                },
                |receiver| format!("{receiver}.{name}"),
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
                .map(|output| output.ty.clone())
        }
        _ => None,
    };
    ty.filter(|ty| integer_width(ty).is_some() || is_float_type(ty))
        .ok_or_else(|| "core float conversion has an unknown source type".to_owned())
}

fn emit_call_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
    values: Vec<EmitValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let function = *state
        .functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM callable {name}"))?;
    let mut arguments = values
        .into_iter()
        .map(|value| match value {
            EmitValue::Basic(value) => Ok(BasicMetadataValueEnum::from(value)),
            EmitValue::Unit => Err("unit argument is invalid".into()),
            EmitValue::Aggregate { .. } => Err("aggregate argument is invalid".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ScalarType::Callable { parameters, .. } = state
        .signatures
        .get(name)
        .ok_or_else(|| format!("unknown LLVM callable signature {name}"))?
    else {
        return Err(format!("LLVM callable {name} has no callable signature"));
    };
    let owner_outputs = if function.count_params() as usize > parameters.len() {
        state
            .dynamic_owner_outputs
            .get(name)
            .ok_or_else(|| format!("missing owner signature for {name}"))?
    } else {
        &BTreeSet::new()
    };
    let mut owner_slots = BTreeMap::new();
    for output in owner_outputs {
        let slot = entry_alloca(
            state,
            context.bool_type().into(),
            &format!("checked_return_owner_{output}"),
        )?;
        arguments.push(slot.into());
        owner_slots.insert(*output, slot);
    }
    let call = state
        .builder
        .build_call(function, &arguments, "call")
        .map_err(builder_error)?;
    state.call_owner_flags = owner_slots
        .into_iter()
        .map(|(output, slot)| {
            state
                .builder
                .build_load(
                    context.bool_type(),
                    slot,
                    &format!("checked_return_owned_{output}"),
                )
                .map(|value| (output, value.into_int_value()))
                .map_err(builder_error)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let outputs = match state.signatures.get(name) {
        Some(ScalarType::Callable { outputs, .. }) => outputs,
        _ => return Err(format!("unknown LLVM callable signature {name}")),
    };
    match (call.try_as_basic_value(), outputs.outputs.as_slice()) {
        (ValueKind::Basic(value), [output]) => {
            if output.ty == ScalarType::Unit {
                return Err("unit callable produced a value".to_owned());
            }
            Ok(EmitValue::Basic(value))
        }
        (ValueKind::Basic(value), outputs) if outputs.len() > 1 => Ok(EmitValue::Aggregate {
            value: value.into_struct_value(),
            outputs: outputs.iter().map(|output| output.ty.clone()).collect(),
        }),
        (ValueKind::Instruction(_), []) | (ValueKind::Instruction(_), [_]) => Ok(EmitValue::Unit),
        (ValueKind::Instruction(_), _) => Err("aggregate callable produced no value".to_owned()),
        (ValueKind::Basic(_), []) | (ValueKind::Basic(_), &[_, _, ..]) => {
            Err("LLVM callable result shape does not match its signature".to_owned())
        }
    }
}

fn emit_binary<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: &ScalarExpression,
    right: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    let unsigned = expression_is_unsigned(state, left);
    let left = emit_expression(context, state, left)?;
    emit_binary_with_rhs(
        context,
        state,
        operator,
        left,
        right,
        ExpressionPath::Single,
        unsigned,
    )
}

fn expression_is_unsigned<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
) -> bool {
    match expression {
        ScalarExpression::Member { receiver, name, .. } => {
            if let Ok(Some(crate::ScalarPlace::Field { field, .. })) =
                struct_member_place(state, receiver, name)
            {
                if let ScalarFieldReference::Resolved(field) = field {
                    if let Ok(structure) = structure(state.structs, field.structure.clone()) {
                        if let Some(field) = structure.fields.get(field.index) {
                            return matches!(
                                &field.ty,
                                ScalarType::U8
                                    | ScalarType::U16
                                    | ScalarType::U32
                                    | ScalarType::U64
                                    | ScalarType::U128
                            );
                        }
                    }
                }
            }
            false
        }
        ScalarExpression::Name { name, .. } => match state.storage.get(name) {
            Some((_, ty)) => matches!(
                ty,
                ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
            ),
            None => match state.globals.get(name) {
                Some((_, ty)) => matches!(
                    ty,
                    ScalarType::U8
                        | ScalarType::U16
                        | ScalarType::U32
                        | ScalarType::U64
                        | ScalarType::U128
                ),
                None => false,
            },
        },
        ScalarExpression::Unary { operand, .. } => expression_is_unsigned(state, operand),
        ScalarExpression::Binary { left, .. } => expression_is_unsigned(state, left),
        _ => false,
    }
}

fn project_expression_is_unsigned<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> bool {
    match expression {
        ScalarExpression::Member { receiver, name, .. } => {
            if let Ok(target) = project_namespace_target(module, modules, receiver) {
                return target.items.iter().any(|item| {
                    matches!(
                        item,
                        ScalarItem::Binding(binding)
                            if binding.name == *name && matches!(
                                &binding.declared_type,
                                ScalarType::U8
                                    | ScalarType::U16
                                    | ScalarType::U32
                                    | ScalarType::U64
                                    | ScalarType::U128
                            )
                    )
                });
            }
            expression_is_unsigned(state, expression)
        }
        ScalarExpression::Unary { operand, .. } => {
            project_expression_is_unsigned(state, operand, module, modules)
        }
        ScalarExpression::Binary { left, .. } => {
            project_expression_is_unsigned(state, left, module, modules)
        }
        _ => expression_is_unsigned(state, expression),
    }
}

enum ExpressionPath<'a> {
    Single,
    Project {
        module: &'a ScalarModule,
        modules: &'a [&'a ScalarModule],
    },
}

fn emit_binary_with_rhs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: EmitValue<'ctx>,
    right: &ScalarExpression,
    path: ExpressionPath<'_>,
    unsigned: bool,
) -> Result<EmitValue<'ctx>, String> {
    if !matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
        let right = match path {
            ExpressionPath::Single => match right {
                ScalarExpression::Integer { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::IntValue(value)) => {
                        let ty = integer_scalar_type(value.get_type().get_bit_width())?;
                        emit_typed_expression(context, state, right, &ty)?
                    }
                    _ => emit_expression(context, state, right)?,
                },
                ScalarExpression::Float { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::FloatValue(value)) => {
                        let ty = if value.get_type() == context.f32_type() {
                            ScalarType::F32
                        } else {
                            ScalarType::F64
                        };
                        emit_typed_expression(context, state, right, &ty)?
                    }
                    _ => emit_expression(context, state, right)?,
                },
                _ => emit_expression(context, state, right)?,
            },
            ExpressionPath::Project { module, modules } => match right {
                ScalarExpression::Integer { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::IntValue(value)) => {
                        let ty = integer_scalar_type(value.get_type().get_bit_width())?;
                        emit_project_typed_expression(context, state, right, &ty, module, modules)?
                    }
                    _ => emit_project_expression(context, state, right, module, modules)?,
                },
                ScalarExpression::Float { .. } => match &left {
                    EmitValue::Basic(BasicValueEnum::FloatValue(value)) => {
                        let ty = if value.get_type() == context.f32_type() {
                            ScalarType::F32
                        } else {
                            ScalarType::F64
                        };
                        emit_project_typed_expression(context, state, right, &ty, module, modules)?
                    }
                    _ => emit_project_expression(context, state, right, module, modules)?,
                },
                _ => emit_project_expression(context, state, right, module, modules)?,
            },
        };
        return emit_binary_values(state, operator, left, right, unsigned);
    }

    let left = take_basic(left)?.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let rhs_block = context.append_basic_block(function, "short_circuit.rhs");
    let merge = context.append_basic_block(function, "short_circuit.merge");
    let left_block = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    let (true_block, false_block) = match operator {
        BinaryOperator::And => (rhs_block, merge),
        BinaryOperator::Or => (merge, rhs_block),
        _ => return Err("invalid short-circuit operator".to_owned()),
    };
    state
        .builder
        .build_conditional_branch(left, true_block, false_block)
        .map_err(builder_error)?;

    state.builder.position_at_end(rhs_block);
    let right = match path {
        ExpressionPath::Single => emit_expression(context, state, right)?,
        ExpressionPath::Project { module, modules } => {
            emit_project_expression(context, state, right, module, modules)?
        }
    };
    let right = take_basic(right)?.into_int_value();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let rhs_end = state.builder.get_insert_block().expect("RHS block");
    state.builder.position_at_end(merge);

    let skipped = left
        .get_type()
        .const_int(u64::from(matches!(operator, BinaryOperator::Or)), false);
    let phi = state
        .builder
        .build_phi(left.get_type(), "short_circuit")
        .map_err(builder_error)?;
    phi.add_incoming(&[(&skipped, left_block), (&right, rhs_end)]);
    Ok(EmitValue::Basic(phi.as_basic_value()))
}

fn emit_binary_values<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    operator: &BinaryOperator,
    left: EmitValue<'ctx>,
    right: EmitValue<'ctx>,
    unsigned: bool,
) -> Result<EmitValue<'ctx>, String> {
    let left = take_basic(left)?;
    let right = take_basic(right)?;
    if let (BasicValueEnum::FloatValue(left), BasicValueEnum::FloatValue(right)) = (left, right) {
        if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
            return Err("short-circuit operators require expression lowering".to_owned());
        }
        return match operator {
            BinaryOperator::Add => state
                .builder
                .build_float_add(left, right, "add")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Subtract => state
                .builder
                .build_float_sub(left, right, "sub")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Multiply => state
                .builder
                .build_float_mul(left, right, "mul")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Divide => state
                .builder
                .build_float_div(left, right, "div")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Remainder => state
                .builder
                .build_float_rem(left, right, "rem")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Equal => state
                .builder
                .build_float_compare(FloatPredicate::OEQ, left, right, "eq")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::NotEqual => state
                .builder
                .build_float_compare(FloatPredicate::ONE, left, right, "ne")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Less => state
                .builder
                .build_float_compare(FloatPredicate::OLT, left, right, "lt")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::LessEqual => state
                .builder
                .build_float_compare(FloatPredicate::OLE, left, right, "le")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::Greater => state
                .builder
                .build_float_compare(FloatPredicate::OGT, left, right, "gt")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::GreaterEqual => state
                .builder
                .build_float_compare(FloatPredicate::OGE, left, right, "ge")
                .map(|value| EmitValue::Basic(value.into())),
            BinaryOperator::And
            | BinaryOperator::Or
            | BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight => {
                return Err("integer operator used with floating-point values".to_owned())
            }
        }
        .map_err(builder_error);
    }
    let left = left.into_int_value();
    let right = right.into_int_value();
    let value = match operator {
        BinaryOperator::Add => state
            .builder
            .build_int_add(left, right, "add")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Subtract => state
            .builder
            .build_int_sub(left, right, "sub")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Multiply => state
            .builder
            .build_int_mul(left, right, "mul")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Divide => if unsigned {
            state.builder.build_int_unsigned_div(left, right, "div")
        } else {
            state.builder.build_int_signed_div(left, right, "div")
        }
        .map_err(builder_error)?
        .into(),
        BinaryOperator::Remainder => if unsigned {
            state.builder.build_int_unsigned_rem(left, right, "rem")
        } else {
            state.builder.build_int_signed_rem(left, right, "rem")
        }
        .map_err(builder_error)?
        .into(),
        BinaryOperator::And | BinaryOperator::Or => {
            return Err("short-circuit operators require expression lowering".to_owned())
        }
        BinaryOperator::Equal => state
            .builder
            .build_int_compare(IntPredicate::EQ, left, right, "eq")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::NotEqual => state
            .builder
            .build_int_compare(IntPredicate::NE, left, right, "ne")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Less => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::ULT
                } else {
                    IntPredicate::SLT
                },
                left,
                right,
                "lt",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::LessEqual => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::ULE
                } else {
                    IntPredicate::SLE
                },
                left,
                right,
                "le",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Greater => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::UGT
                } else {
                    IntPredicate::SGT
                },
                left,
                right,
                "gt",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::GreaterEqual => state
            .builder
            .build_int_compare(
                if unsigned {
                    IntPredicate::UGE
                } else {
                    IntPredicate::SGE
                },
                left,
                right,
                "ge",
            )
            .map_err(builder_error)?
            .into(),
        BinaryOperator::BitAnd => state
            .builder
            .build_and(left, right, "and")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::BitOr => state
            .builder
            .build_or(left, right, "or")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::BitXor => state
            .builder
            .build_xor(left, right, "xor")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::ShiftLeft => state
            .builder
            .build_left_shift(left, right, "shl")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::ShiftRight => state
            .builder
            .build_right_shift(left, right, !unsigned, "shr")
            .map_err(builder_error)?
            .into(),
    };
    Ok(EmitValue::Basic(value))
}

fn emit_unary_value<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    operator: &crate::scalar::UnaryOperator,
    value: inkwell::values::IntValue<'ctx>,
) -> Result<EmitValue<'ctx>, String> {
    match operator {
        crate::scalar::UnaryOperator::LogicalNot | crate::scalar::UnaryOperator::BitwiseNot => {
            state
                .builder
                .build_not(value, "not")
                .map(|value| EmitValue::Basic(value.into()))
                .map_err(builder_error)
        }
        crate::scalar::UnaryOperator::Negate => state
            .builder
            .build_int_sub(value.get_type().const_zero(), value, "neg")
            .map(|value| EmitValue::Basic(value.into()))
            .map_err(builder_error),
    }
}

fn emit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: &ScalarExpression,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(emit_expression(context, state, condition)?)?.into_int_value();
    emit_if_value(
        context,
        state,
        condition,
        then_branch,
        else_branch,
        |state, block| emit_block(context, state, block),
    )
}
fn emit_project_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: EmitValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(condition)?.into_int_value();
    emit_if_value(
        context,
        state,
        condition,
        then_branch,
        else_branch,
        |state, block| emit_project_block(context, state, block, module, modules),
    )
}

fn emit_unit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: &ScalarExpression,
    then_branch: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(emit_expression(context, state, condition)?)?.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    emit_block(context, state, then_branch)?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge);
    Ok(EmitValue::Unit)
}

fn emit_project_unit_if<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: EmitValue<'ctx>,
    then_branch: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let condition = take_basic(condition)?.into_int_value();
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    emit_project_block(context, state, then_branch, module, modules)?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    state.builder.position_at_end(merge);
    Ok(EmitValue::Unit)
}

fn emit_if_value<'ctx, 'module, F>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    condition: inkwell::values::IntValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    emit: F,
) -> Result<EmitValue<'ctx>, String>
where
    F: Fn(&mut EmitState<'ctx, 'module>, &ScalarBlock) -> Result<EmitValue<'ctx>, String>,
{
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let then_block = context.append_basic_block(function, "if.then");
    let else_block = context.append_basic_block(function, "if.else");
    let merge = context.append_basic_block(function, "if.merge");
    state
        .builder
        .build_conditional_branch(condition, then_block, else_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(then_block);
    let incoming_owners = state.return_owner_flags.clone();
    state.return_owner_flags = incoming_owners.clone();
    let then_value = emit(state, then_branch)?;
    let then_owners = state.return_owner_flags.clone();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let then_end = state.builder.get_insert_block().expect("then block");
    state.builder.position_at_end(else_block);
    state.return_owner_flags = incoming_owners.clone();
    let else_value = emit(state, else_branch)?;
    let else_owners = state.return_owner_flags.clone();
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let else_end = state.builder.get_insert_block().expect("else block");
    state.builder.position_at_end(merge);
    state.return_owner_flags = incoming_owners;
    for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
        match (then_owners.get(&output), else_owners.get(&output)) {
            (Some(then_owner), Some(else_owner)) => {
                let phi = state
                    .builder
                    .build_phi(context.bool_type(), &format!("return_owner_{output}"))
                    .map_err(builder_error)?;
                phi.add_incoming(&[(then_owner, then_end), (else_owner, else_end)]);
                state
                    .return_owner_flags
                    .insert(output, phi.as_basic_value().into_int_value());
            }
            (None, None) => {}
            _ => {
                return Err(format!(
                    "conditional return lacks owner for output {output}"
                ))
            }
        }
    }
    match (then_value, else_value) {
        (EmitValue::Unit, EmitValue::Unit) => Ok(EmitValue::Unit),
        (EmitValue::Basic(then_value), EmitValue::Basic(else_value)) => {
            let phi = state
                .builder
                .build_phi(then_value.get_type(), "if")
                .map_err(builder_error)?;
            phi.add_incoming(&[(&then_value, then_end), (&else_value, else_end)]);
            Ok(EmitValue::Basic(phi.as_basic_value()))
        }
        (
            EmitValue::Aggregate {
                value: then_value,
                outputs,
            },
            EmitValue::Aggregate {
                value: else_value,
                outputs: else_outputs,
            },
        ) if outputs == else_outputs => {
            let phi = state
                .builder
                .build_phi(then_value.get_type(), "if")
                .map_err(builder_error)?;
            phi.add_incoming(&[(&then_value, then_end), (&else_value, else_end)]);
            Ok(EmitValue::Aggregate {
                value: phi.as_basic_value().into_struct_value(),
                outputs,
            })
        }
        _ => Err("conditional branches have different LLVM values".into()),
    }
}

fn transfer_automatic_return_result(state: &mut EmitState<'_, '_>, expression: &ScalarExpression) {
    if let ScalarExpression::Name { name, .. } = expression {
        if let Some(result) = state.automatic_return_result_owners.get_mut(name) {
            result.state = ScalarAutomaticReturnResultState::Transferred;
        }
        state.automatic_return_result_owners.remove(name);
    }
}

fn record_forwarded_return_owner<'ctx>(
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

fn emit_automatic_return_result<'ctx, 'module>(
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

fn emit_return<'ctx, 'module>(
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

fn materialize_fresh_return_outputs<'ctx, 'module>(
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

fn materialize_return_branch<'ctx, 'module>(
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

fn generic_specializations<'a>(
    items: &[ScalarItem],
    function: impl Fn(&str) -> Option<&'a ScalarFunction>,
    symbol: impl Fn(&str) -> String,
) -> Result<Vec<(String, ScalarFunction, String)>, String> {
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
            collect_selected_overloads(&assignment.value, selections)
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
