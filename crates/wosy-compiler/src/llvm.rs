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
    ScalarAllocationIdentity, ScalarAutomaticReturnResult, ScalarAutomaticReturnResultState,
    ScalarBindingOutputOrigin, ScalarFieldReference, ScalarOutputValue, ScalarOverloadSelection,
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
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    return_bindings: BTreeMap<String, ScalarExpression>,
    runtime_array_owners: BTreeMap<String, bool>,
    runtime_array_allocations: BTreeMap<String, bool>,
    automatic_return_result_owners: BTreeMap<String, ScalarAutomaticReturnResult>,
    automatic_return_result_results: &'ctx BTreeSet<String>,
    globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    all_globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    structs: &'module [ScalarStruct],
    target_layout: ScalarTargetLayout,
    next_literal: usize,
    next_block: usize,
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
            function_type(&context, &function.signature, target_layout)?.0,
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
            function_type(&context, &function.signature, target_layout)?.0,
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
            function_type(&context, &function.signature, target_layout)?.0,
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
        }
    }
    for (_, function, name) in specializations.iter().chain(overloads.iter()) {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
    }
    let automatic_return_result_results = automatic_return_result_functions(
        validation
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Function(function) => Some((function.name.as_str(), function)),
                _ => None,
            }),
        &call_targets,
    );
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
            &automatic_return_result_results,
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
            &automatic_return_result_results,
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
            &automatic_return_result_results,
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
        &automatic_return_result_results,
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
        .project
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
                    function_type(&context, &function.signature, target_layout)?.0,
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
                function_type(&context, &function.signature, target_layout)?.0,
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
                function_type(&context, &function.signature, target_layout)?.0,
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
    for (_, function, name) in &definitions {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
    }
    for (_, function, name) in &project_specializations {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
    }
    for (_, function, name) in &project_overloads {
        insert_runtime_array_result_provenance(&mut runtime_array_results, name, function);
    }
    let automatic_return_result_results = automatic_return_result_functions(
        definitions
            .iter()
            .map(|(_, function, name)| (name.as_str(), *function))
            .chain(
                project_specializations
                    .iter()
                    .map(|(_, function, name)| (name.as_str(), function)),
            )
            .chain(
                project_overloads
                    .iter()
                    .map(|(_, function, name)| (name.as_str(), function)),
            ),
        &call_targets,
    );
    for (source_module, function, name) in definitions {
        emit_project_function(
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &automatic_return_result_results,
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
            &automatic_return_result_results,
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
            &automatic_return_result_results,
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
        &automatic_return_result_results,
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

fn integer_width(ty: &ScalarType) -> Option<u32> {
    match ty {
        ScalarType::I8 | ScalarType::U8 => Some(8),
        ScalarType::I16 | ScalarType::U16 => Some(16),
        ScalarType::I32 | ScalarType::U32 => Some(32),
        ScalarType::I64 | ScalarType::U64 => Some(64),
        ScalarType::I128 | ScalarType::U128 => Some(128),
        _ => None,
    }
}

fn integer_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<inkwell::types::IntType<'ctx>, String> {
    let width = integer_width(ty).ok_or_else(|| "expected integer type".to_owned())?;
    context
        .custom_width_int_type(NonZeroU32::new(width).expect("integer width is nonzero"))
        .map_err(str::to_owned)
}

fn integer_scalar_type(width: u32) -> Result<ScalarType, String> {
    match width {
        8 => Ok(ScalarType::U8),
        16 => Ok(ScalarType::U16),
        32 => Ok(ScalarType::U32),
        64 => Ok(ScalarType::U64),
        128 => Ok(ScalarType::U128),
        _ => Err(format!("unsupported LLVM integer width {width}")),
    }
}

fn pointer_integer_type<'ctx>(
    context: &'ctx Context,
    target_layout: ScalarTargetLayout,
) -> inkwell::types::IntType<'ctx> {
    context
        .custom_width_int_type(
            NonZeroU32::new((target_layout.pointer_size * 8) as u32)
                .expect("pointer width is nonzero"),
        )
        .expect("supported pointer width")
}

fn basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Bool => Ok(context.bool_type().into()),
        ty if integer_width(ty).is_some() => Ok(integer_type(context, ty)?.into()),
        ScalarType::Char => Ok(context.i32_type().into()),
        ScalarType::Enum(_) => Ok(context.i32_type().into()),
        ScalarType::F32 => Ok(context.f32_type().into()),
        ScalarType::F64 => Ok(context.f64_type().into()),
        ScalarType::ArtifactId => Ok(context.ptr_type(AddressSpace::default()).into()),
        ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. } => {
            Ok(pointer_integer_type(context, target_layout).into())
        }
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. } => {
            Ok(context.ptr_type(AddressSpace::default()).into())
        }
        _ => Err("unit is only valid as a function result".into()),
    }
}

fn storage_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Struct(id) => {
            let structure = structs
                .iter()
                .find(|structure| structure.id == *id)
                .ok_or_else(|| format!("unknown LLVM struct {}", id.index))?;
            let layout = structure
                .layout
                .as_ref()
                .ok_or_else(|| format!("LLVM struct {} has no valid layout", id.index))?;
            let size =
                u32::try_from(layout.size).map_err(|_| "struct layout exceeds LLVM array size")?;
            Ok(context.i8_type().array_type(size).into())
        }
        ScalarType::Array {
            element, length, ..
        } => Ok(storage_type(context, element, structs, target_layout)?
            .array_type(
                u32::try_from(*length).map_err(|_| "fixed array length exceeds LLVM array size")?,
            )
            .into()),
        _ => basic_type(context, ty, target_layout),
    }
}

fn allocation_layout(
    ty: &ScalarType,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<(u64, u64), String> {
    match ty {
        ScalarType::Bool | ScalarType::I8 | ScalarType::U8 => Ok((1, 1)),
        ScalarType::I16 | ScalarType::U16 => Ok((2, 2)),
        ScalarType::I32 | ScalarType::U32 | ScalarType::Char | ScalarType::F32 => Ok((4, 4)),
        ScalarType::I64 | ScalarType::U64 | ScalarType::F64 => Ok((8, 8)),
        ScalarType::I128 | ScalarType::U128 => Ok((16, 16)),
        ScalarType::RawPointer(_)
        | ScalarType::CheckedReference { .. }
        | ScalarType::ArtifactId => {
            Ok((target_layout.pointer_size, target_layout.pointer_alignment))
        }
        ScalarType::Enum(_) => Ok((4, 4)),
        ScalarType::Struct(id) => structs
            .iter()
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.layout.as_ref())
            .map(|layout| (layout.size, layout.alignment))
            .ok_or_else(|| format!("runtime allocation has no layout for struct {}", id.index)),
        ScalarType::Array {
            element, length, ..
        } => {
            let (size, alignment) = allocation_layout(element, structs, target_layout)?;
            let size = size
                .checked_mul(*length)
                .ok_or_else(|| "runtime allocation size overflows u64".to_owned())?;
            Ok((size, alignment))
        }
        ScalarType::Named { .. }
        | ScalarType::Qualified { .. }
        | ScalarType::RuntimeArray { .. }
        | ScalarType::Unit
        | ScalarType::Error
        | ScalarType::Callable { .. } => {
            Err("runtime allocation requires a sized element type".to_owned())
        }
    }
}
fn value_type(ty: &ScalarType) -> Result<LlvmValueType, String> {
    match ty {
        ScalarType::Unit => Ok(LlvmValueType::Void),
        ScalarType::Bool => Ok(LlvmValueType::I1),
        ScalarType::I8 | ScalarType::U8 => Ok(LlvmValueType::I8),
        ScalarType::I16 | ScalarType::U16 => Ok(LlvmValueType::I16),
        ScalarType::I32 | ScalarType::U32 => Ok(LlvmValueType::I32),
        ScalarType::I64 | ScalarType::U64 => Ok(LlvmValueType::I64),
        ScalarType::I128 | ScalarType::U128 => Ok(LlvmValueType::I128),
        ScalarType::F32 => Ok(LlvmValueType::F32),
        ScalarType::F64 => Ok(LlvmValueType::F64),
        ScalarType::Char => Ok(LlvmValueType::Char),
        ScalarType::ArtifactId => Ok(LlvmValueType::ArtifactId),
        ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. } => {
            Ok(LlvmValueType::Pointer)
        }
        ScalarType::Enum(_) => Ok(LlvmValueType::I32),
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. } => {
            Ok(LlvmValueType::Pointer)
        }
        _ => Err("unsupported LLVM scalar type".into()),
    }
}

fn integer_constant<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    value: &num_bigint::BigInt,
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let integer = integer_type(context, ty)?;
    let (sign, words) = value.to_u64_digits();
    if sign == num_bigint::Sign::Minus {
        return Err("negative integer literal is invalid".to_owned());
    }
    if integer.get_bit_width() <= 64 {
        return Ok(match words.first() {
            Some(word) => integer.const_int(*word, false),
            None => integer.const_zero(),
        });
    }
    Ok(integer.const_int_arbitrary_precision(&words))
}

fn float_constant<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    value: f64,
) -> Result<inkwell::values::FloatValue<'ctx>, String> {
    match ty {
        ScalarType::F32 => Ok(context.f32_type().const_float(value)),
        ScalarType::F64 => Ok(context.f64_type().const_float(value)),
        _ => Err("expected floating-point type".to_owned()),
    }
}

fn aggregate_type<'ctx>(
    context: &'ctx Context,
    outputs: &crate::ScalarOutputSequence,
    target_layout: ScalarTargetLayout,
) -> Result<StructType<'ctx>, String> {
    let fields = outputs
        .outputs
        .iter()
        .map(|output| output_basic_type(context, &output.ty, target_layout))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(context.struct_type(&fields, false))
}

fn output_basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<BasicTypeEnum<'ctx>, String> {
    if *ty == ScalarType::Unit {
        Ok(context.i8_type().into())
    } else {
        basic_type(context, ty, target_layout)
    }
}

fn builder_error(error: BuilderError) -> String {
    error.to_string()
}

fn entry_alloca<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    ty: BasicTypeEnum<'ctx>,
    name: &str,
) -> Result<PointerValue<'ctx>, String> {
    let insert = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing insertion block".to_owned())?;
    let function = insert
        .get_parent()
        .ok_or_else(|| "missing function".to_owned())?;
    let entry = function
        .get_first_basic_block()
        .ok_or_else(|| "missing entry block".to_owned())?;
    if let Some(first) = entry.get_first_instruction() {
        state.builder.position_before(&first);
    } else {
        state.builder.position_at_end(entry);
    }
    let slot = state
        .builder
        .build_alloca(ty, name)
        .map_err(builder_error)?;
    state.builder.position_at_end(insert);
    Ok(slot)
}

fn structure<'a>(
    structs: &'a [ScalarStruct],
    id: crate::ScalarStructId,
) -> Result<&'a ScalarStruct, String> {
    structs
        .iter()
        .find(|structure| structure.id == id)
        .ok_or_else(|| format!("unknown LLVM struct {}", id.index))
}

fn place_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<PointerValue<'ctx>, String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => {
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                if matches!(ty, ScalarType::RuntimeArray { .. }) {
                    if state
                        .runtime_array_allocations
                        .get(name)
                        .copied()
                        .unwrap_or(false)
                    {
                        return Ok(slot);
                    }
                    return state
                        .builder
                        .build_load(basic_type(context, &ty, state.target_layout)?, slot, name)
                        .map_err(builder_error)
                        .map(BasicValueEnum::into_pointer_value);
                }
                return Ok(slot);
            }
            let (global, ty) = state
                .globals
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown LLVM place {name}"))?;
            if matches!(ty, ScalarType::RuntimeArray { .. }) {
                return state
                    .builder
                    .build_load(
                        basic_type(context, &ty, state.target_layout)?,
                        global.as_pointer_value(),
                        name,
                    )
                    .map_err(builder_error)
                    .map(BasicValueEnum::into_pointer_value);
            }
            Ok(global.as_pointer_value())
        }
        crate::ScalarPlace::Dereference { pointer, .. } => {
            let pointer_value = take_basic(match project {
                Some(project) => emit_project_expression(
                    context,
                    state,
                    pointer,
                    project.module,
                    project.modules,
                )?,
                None => emit_expression(context, state, pointer)?,
            })?
            .into_int_value();
            pointer_target_type(state, pointer, project)
                .ok_or_else(|| "unknown LLVM dereference target type".to_owned())?;
            if dereference_requires_null_guard(state, pointer, project) {
                emit_checked_dereference_guard(context, state, pointer_value)?;
            }
            state
                .builder
                .build_int_to_ptr(
                    pointer_value,
                    context.ptr_type(AddressSpace::default()),
                    "deref",
                )
                .map_err(builder_error)
        }
        crate::ScalarPlace::Field { base, field, .. } => {
            let base = place_pointer(context, state, base, project)?;
            let ScalarFieldReference::Resolved(field) = field else {
                return Err("unresolved LLVM struct field".to_owned());
            };
            let structure = structure(state.structs, field.structure.clone())?;
            let resolved = structure
                .fields
                .get(field.index)
                .filter(|candidate| candidate.id == *field)
                .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?;
            let byte_type = context.i8_type();
            let offset = byte_type.const_int(
                resolved.offset.ok_or_else(|| {
                    format!("LLVM struct field {} has no valid offset", field.index)
                })?,
                false,
            );
            unsafe {
                state.builder.build_in_bounds_gep(
                    byte_type,
                    base,
                    &[offset],
                    resolved.name.as_str(),
                )
            }
            .map_err(builder_error)
        }
        crate::ScalarPlace::Index { base, index, .. } => {
            let base_pointer = place_pointer(context, state, base, project)?;
            let base_type = assignment_place_type(state, base)
                .ok_or_else(|| "unknown LLVM indexed place type".to_owned())?;
            if !matches!(
                base_type,
                ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
            ) {
                return Err("LLVM indexed place requires an array".to_owned());
            }
            let index = take_basic(match project {
                Some(project) => emit_project_typed_expression(
                    context,
                    state,
                    index,
                    &ScalarType::U64,
                    project.module,
                    project.modules,
                )?,
                None => emit_typed_expression(context, state, index, &ScalarType::U64)?,
            })?
            .into_int_value();
            if let ScalarType::RuntimeArray { element, .. } = base_type {
                let element = storage_type(context, &element, state.structs, state.target_layout)?;
                return unsafe {
                    state.builder.build_in_bounds_gep(
                        element,
                        base_pointer,
                        &[index],
                        "array_element",
                    )
                }
                .map_err(builder_error);
            }
            let array = storage_type(context, &base_type, state.structs, state.target_layout)?;
            unsafe {
                state.builder.build_in_bounds_gep(
                    array,
                    base_pointer,
                    &[context.i32_type().const_zero(), index],
                    "array_element",
                )
            }
            .map_err(builder_error)
        }
    }
}

fn emit_place_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
) -> Result<EmitValue<'ctx>, String> {
    emit_place_value_in_project(context, state, place, None)
}

fn emit_place_value_in_project<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
    project: Option<ProjectCallScope<'_>>,
) -> Result<EmitValue<'ctx>, String> {
    let pointer = place_pointer(context, state, place, project)?;
    let ty = match place {
        crate::ScalarPlace::Field { field, .. } => {
            let ScalarFieldReference::Resolved(field) = field else {
                return Err("unresolved LLVM struct field".to_owned());
            };
            structure(state.structs, field.structure.clone())?
                .fields
                .get(field.index)
                .filter(|candidate| candidate.id == *field)
                .ok_or_else(|| format!("unknown LLVM struct field {}", field.index))?
                .ty
                .clone()
        }
        crate::ScalarPlace::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty.clone()))
            .ok_or_else(|| format!("unknown LLVM place {name}"))?,
        crate::ScalarPlace::Dereference { pointer, .. } => {
            pointer_target_type(state, pointer, project)
                .ok_or_else(|| "unknown LLVM dereference target type".to_owned())?
        }
        crate::ScalarPlace::Index { .. } => assignment_place_type(state, place)
            .ok_or_else(|| "unknown LLVM indexed place type".to_owned())?,
    };
    if matches!(
        ty,
        ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
    ) {
        return Ok(EmitValue::Basic(pointer.into()));
    }
    Ok(EmitValue::Basic(
        state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                pointer,
                "place",
            )
            .map_err(builder_error)?,
    ))
}

fn struct_member_place<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    receiver: &str,
    name: &str,
) -> Result<Option<crate::ScalarPlace>, String> {
    let ty = state
        .storage
        .get(receiver)
        .map(|(_, ty)| ty)
        .or_else(|| state.globals.get(receiver).map(|(_, ty)| ty));
    let Some(ScalarType::Struct(id)) = ty else {
        return Ok(None);
    };
    let field = structure(state.structs, id.clone())?
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format!("unknown LLVM struct field {name}"))?;
    Ok(Some(crate::ScalarPlace::Field {
        base: Box::new(crate::ScalarPlace::Name {
            name: receiver.to_owned(),
            span: wosy_syntax::ByteSpan::new(0, 0),
        }),
        field: ScalarFieldReference::Resolved(field.id.clone()),
        span: wosy_syntax::ByteSpan::new(0, 0),
    }))
}

fn emit_struct_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expected: &crate::ScalarType,
    fields: &[crate::ScalarStructLiteralField],
) -> Result<EmitValue<'ctx>, String> {
    let crate::ScalarType::Struct(id) = expected else {
        return Err("struct literal requires a resolved struct type".to_owned());
    };
    let structure = structure(state.structs, id.clone())?;
    let layout = structure
        .layout
        .as_ref()
        .ok_or_else(|| format!("LLVM struct {} has no valid layout", id.index))?;
    let storage = context.i8_type().array_type(
        u32::try_from(layout.size).map_err(|_| "struct layout exceeds LLVM array size")?,
    );
    let destination = entry_alloca(state, storage.into(), "struct_literal")?;
    let mut values = BTreeMap::new();
    for field in fields {
        values.insert(
            field.name.clone(),
            emit_typed_expression(
                context,
                state,
                &field.value,
                &structure
                    .fields
                    .iter()
                    .find(|candidate| candidate.name == field.name)
                    .expect("validated struct field")
                    .ty,
            )?,
        );
    }
    for field in &structure.fields {
        let value = values
            .remove(&field.name)
            .ok_or_else(|| format!("missing LLVM struct literal field {}", field.name))?;
        let pointer = unsafe {
            state.builder.build_in_bounds_gep(
                context.i8_type(),
                destination,
                &[context.i8_type().const_int(
                    field.offset.ok_or_else(|| {
                        format!("LLVM struct field {} has no valid offset", field.id.index)
                    })?,
                    false,
                )],
                field.name.as_str(),
            )
        }
        .map_err(builder_error)?;
        store_value(context, state, pointer, &field.ty, value)?;
    }
    Ok(EmitValue::Basic(destination.into()))
}

fn emit_array_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expected: &ScalarType,
    elements: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let ScalarType::Array {
        element, length, ..
    } = expected
    else {
        return Err("array literal requires a resolved fixed array type".to_owned());
    };
    if elements.len() as u64 != *length {
        return Err("array literal element count does not match fixed array length".to_owned());
    }
    let storage = storage_type(context, expected, state.structs, state.target_layout)?;
    let destination = entry_alloca(state, storage, "array_literal")?;
    let element_storage = storage_type(context, element, state.structs, state.target_layout)?;
    for (index, expression) in elements.iter().enumerate() {
        let pointer = unsafe {
            state.builder.build_in_bounds_gep(
                storage,
                destination,
                &[
                    context.i32_type().const_zero(),
                    context.i32_type().const_int(index as u64, false),
                ],
                "array_element",
            )
        }
        .map_err(builder_error)?;
        let value = emit_typed_expression(context, state, expression, element)?;
        let _ = element_storage;
        store_value(context, state, pointer, element, value)?;
    }
    Ok(EmitValue::Basic(destination.into()))
}

fn emit_typed_expression<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    expected: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    match (expression, expected) {
        (ScalarExpression::Utf8 { value, .. }, ScalarType::Struct(_)) => {
            emit_utf8_literal(context, state, value, expected)
        }
        (ScalarExpression::StructLiteral { fields, .. }, ScalarType::Struct(_)) => {
            emit_struct_literal(context, state, expected, fields)
        }
        (ScalarExpression::ArrayLiteral { elements, .. }, ScalarType::Array { .. }) => {
            emit_array_literal(context, state, expected, elements)
        }
        (
            ScalarExpression::Unary {
                operator, operand, ..
            },
            ty,
        ) if integer_width(ty).is_some() || *ty == ScalarType::Bool => {
            let value =
                take_basic(emit_typed_expression(context, state, operand, ty)?)?.into_int_value();
            emit_unary_value(state, operator, value)
        }
        (ScalarExpression::Integer { value, .. }, ty) if integer_width(ty).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, ty, value)?.into()),
        ),
        (ScalarExpression::Float { value, .. }, ty) => Ok(EmitValue::Basic(
            float_constant(context, ty, *value)?.into(),
        )),
        _ => emit_expression(context, state, expression),
    }
}

fn emit_utf8_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: &[u8],
    expected: &ScalarType,
) -> Result<EmitValue<'ctx>, String> {
    let ScalarType::Struct(id) = expected else {
        return Err("string literal requires a resolved std.utf8 struct".to_owned());
    };
    let structure = structure(state.structs, id.clone())?;
    if structure.fields.len() != 2
        || structure.fields[0].name != "data"
        || structure.fields[1].name != "length"
    {
        return Err("std.utf8 has an invalid field layout".to_owned());
    }
    let byte_type = context.i8_type();
    let literal_type =
        byte_type.array_type(u32::try_from(value.len()).map_err(|_| "utf8 literal is too large")?);
    let literal = state.module.add_global(
        literal_type,
        None,
        &format!("wosy_utf8_literal_{}", state.next_literal),
    );
    state.next_literal += 1;
    literal.set_linkage(Linkage::Private);
    literal.set_constant(true);
    let bytes = value
        .iter()
        .map(|byte| byte_type.const_int(u64::from(*byte), false))
        .collect::<Vec<_>>();
    literal.set_initializer(&byte_type.const_array(&bytes));
    let destination = entry_alloca(
        state,
        storage_type(context, expected, state.structs, state.target_layout)?,
        "utf8_literal",
    )?;
    let data = unsafe {
        state.builder.build_in_bounds_gep(
            byte_type,
            destination,
            &[byte_type.const_int(
                structure.fields[0]
                    .offset
                    .ok_or_else(|| "std.utf8 data field has no valid offset".to_owned())?,
                false,
            )],
            "data",
        )
    }
    .map_err(builder_error)?;
    let address = state
        .builder
        .build_ptr_to_int(
            literal.as_pointer_value(),
            pointer_integer_type(context, state.target_layout),
            "utf8_address",
        )
        .map_err(builder_error)?;
    state
        .builder
        .build_store(data, address)
        .map_err(builder_error)?;
    let length = unsafe {
        state.builder.build_in_bounds_gep(
            byte_type,
            destination,
            &[byte_type.const_int(
                structure.fields[1]
                    .offset
                    .ok_or_else(|| "std.utf8 length field has no valid offset".to_owned())?,
                false,
            )],
            "length",
        )
    }
    .map_err(builder_error)?;
    state
        .builder
        .build_store(
            length,
            context.i64_type().const_int(value.len() as u64, false),
        )
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(destination.into()))
}

fn store_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    destination: PointerValue<'ctx>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        let source = take_basic(value)?.into_pointer_value();
        let aggregate = storage_type(context, ty, state.structs, state.target_layout)?;
        let value = state
            .builder
            .build_load(aggregate, source, "struct_value")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(destination, value)
            .map_err(builder_error)
            .map(|_| ())
    } else {
        state
            .builder
            .build_store(destination, take_basic(value)?)
            .map_err(builder_error)
            .map(|_| ())
    }
}

fn take_basic(value: EmitValue<'_>) -> Result<BasicValueEnum<'_>, String> {
    match value {
        EmitValue::Basic(value) => Ok(value),
        EmitValue::Unit => Err("unit value used where a scalar value is required".into()),
        EmitValue::Aggregate { .. } => {
            Err("aggregate value used where a scalar value is required".into())
        }
    }
}

fn build_aggregate<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outputs: &crate::ScalarOutputSequence,
    values: Vec<EmitValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let ty = aggregate_type(context, outputs, state.target_layout)?;
    let mut aggregate = ty.const_zero();
    for (index, (output, value)) in outputs.outputs.iter().zip(values).enumerate() {
        let value = if output.ty == ScalarType::Unit {
            context.i8_type().const_zero().into()
        } else {
            take_basic(value)?
        };
        aggregate = state
            .builder
            .build_insert_value(aggregate, value, index as u32, "output")
            .map_err(builder_error)?
            .into_struct_value();
    }
    Ok(EmitValue::Aggregate {
        value: aggregate,
        outputs: outputs
            .outputs
            .iter()
            .map(|output| output.ty.clone())
            .collect(),
    })
}

fn extract_output<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    value: EmitValue<'ctx>,
    position: usize,
) -> Result<EmitValue<'ctx>, String> {
    let EmitValue::Aggregate { value, outputs } = value else {
        return Ok(value);
    };
    let ty = outputs
        .get(position)
        .ok_or_else(|| "LLVM output position is out of range".to_owned())?;
    if *ty == ScalarType::Unit {
        return Ok(EmitValue::Unit);
    }
    Ok(EmitValue::Basic(
        state
            .builder
            .build_extract_value(value, position as u32, "output")
            .map_err(builder_error)?
            .into(),
    ))
}

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
        transition_automatic_return_result_binding_initialization(state, name, ty, &binding.value);
    }
    Ok(())
}

fn transition_automatic_return_result_binding_initialization(
    state: &mut EmitState<'_, '_>,
    name: &str,
    ty: &ScalarType,
    value: &ScalarExpression,
) {
    let ScalarType::CheckedReference { inner, .. } = ty else {
        return;
    };
    if !automatic_return_result_copy_pointee(inner) {
        return;
    }
    let source = match value {
        ScalarExpression::Call { receiver, name, .. } => {
            let lookup = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            state
                .call_targets
                .get(&lookup)
                .is_some_and(|target| state.automatic_return_result_results.contains(target))
        }
        ScalarExpression::Name { name: source, .. } => {
            state.automatic_return_result_owners.contains_key(source)
        }
        _ => false,
    };
    if source {
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
        }
    }
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
        let pointer = state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                slot,
                "return_result_owner",
            )
            .map_err(builder_error)?
            .into_int_value();
        let pointer = state
            .builder
            .build_int_to_ptr(
                pointer,
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
        state.automatic_return_result_owners.remove(&name);
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

fn automatic_return_result_functions<'a>(
    functions: impl IntoIterator<Item = (&'a str, &'a ScalarFunction)>,
    call_targets: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let functions = functions.into_iter().collect::<BTreeMap<_, _>>();
    let mut results = BTreeSet::new();
    loop {
        let mut changed = false;
        for (name, function) in &functions {
            if automatic_return_result_function(function, &results, call_targets)
                && results.insert((*name).to_owned())
            {
                changed = true;
            }
        }
        if !changed {
            return results;
        }
    }
}

fn automatic_return_result_copy_pointee(inner: &ScalarType) -> bool {
    matches!(
        inner,
        ScalarType::Bool
            | ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128
            | ScalarType::F32
            | ScalarType::F64
            | ScalarType::Char
            | ScalarType::Struct(_)
    )
}

fn automatic_return_result_function(
    function: &ScalarFunction,
    results: &BTreeSet<String>,
    call_targets: &BTreeMap<String, String>,
) -> bool {
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return false;
    };
    let Some(output) = outputs.outputs.first() else {
        return false;
    };
    let ScalarType::CheckedReference { inner, .. } = &output.ty else {
        return false;
    };
    if !automatic_return_result_copy_pointee(inner) {
        return false;
    }
    let bindings = return_bindings(&function.body);
    automatic_return_result_expression(
        &function.body.final_output_values[0].value,
        &bindings,
        results,
        call_targets,
        &mut BTreeSet::new(),
    )
}

fn automatic_return_result_expression(
    expression: &ScalarExpression,
    bindings: &BTreeMap<String, ScalarExpression>,
    results: &BTreeSet<String>,
    call_targets: &BTreeMap<String, String>,
    visited: &mut BTreeSet<String>,
) -> bool {
    match expression {
        ScalarExpression::CheckedAddress { place, .. } => {
            matches!(place, crate::ScalarPlace::Name { name, .. } if bindings.contains_key(name))
        }
        ScalarExpression::Name { name, .. } => {
            visited.insert(name.clone())
                && bindings.get(name).is_some_and(|value| {
                    automatic_return_result_expression(
                        value,
                        bindings,
                        results,
                        call_targets,
                        visited,
                    )
                })
        }
        ScalarExpression::Call { receiver, name, .. } => {
            let lookup = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            call_targets
                .get(&lookup)
                .is_some_and(|target| results.contains(target))
        }
        _ => false,
    }
}

fn function_runtime_array_result_provenance(
    function: &ScalarFunction,
) -> Option<RuntimeArrayResultProvenance> {
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return None;
    };
    if !matches!(outputs.outputs.first()?.ty, ScalarType::RuntimeArray { .. }) {
        return None;
    }
    let expression = function.body.final_output_values.first()?.value.clone();
    match expression {
        ScalarExpression::Call { .. } => Some(RuntimeArrayResultProvenance::FreshCall),
        ScalarExpression::Name { name, .. } => {
            if function.parameters.contains(&name) {
                return Some(RuntimeArrayResultProvenance::ParameterAlias);
            }
            function.body.items.iter().find_map(|item| match item {
                ScalarBlockItem::LocalBinding(binding)
                    if binding_receivers(binding)
                        .iter()
                        .any(|(receiver, _)| receiver == &name) =>
                {
                    if binding.is_allocation {
                        Some(RuntimeArrayResultProvenance::LocalOwner)
                    } else if matches!(binding.declared_type, ScalarType::Array { .. }) {
                        Some(RuntimeArrayResultProvenance::FixedWidening)
                    } else if matches!(binding.value, ScalarExpression::Call { .. }) {
                        Some(RuntimeArrayResultProvenance::FreshCall)
                    } else {
                        Some(RuntimeArrayResultProvenance::ParameterAlias)
                    }
                }
                _ => None,
            })
        }
        _ => Some(RuntimeArrayResultProvenance::FixedWidening),
    }
}

fn release_runtime_array_owner<'ctx, 'module>(
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
        .ok_or_else(|| format!("missing runtime-array storage for {name}"))?;
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Err(format!("runtime-array owner {name} has an invalid type"));
    }
    declare_core_runtime(context, state.module);
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
    state
        .builder
        .build_call(release, &[pointer.into()], "runtime_array_release")
        .map_err(builder_error)?;
    state.runtime_array_owners.insert(name.to_owned(), false);
    Ok(())
}

fn release_scope_runtime_array_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outer_owners: &BTreeMap<String, bool>,
) -> Result<(), String> {
    let owners = state
        .runtime_array_owners
        .keys()
        .filter(|name| !outer_owners.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    for name in owners {
        release_runtime_array_owner(context, state, &name)?;
    }
    Ok(())
}

fn transfer_runtime_array_return(
    state: &mut EmitState<'_, '_>,
    block: &ScalarBlock,
    expression: &ScalarExpression,
    ty: &ScalarType,
) {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        let mut bindings = state.return_bindings.clone();
        bindings.extend(return_bindings(block));
        let mut visited = BTreeSet::new();
        for name in escaped_runtime_array_owners(expression, &bindings, &mut visited) {
            if state.runtime_array_owners.contains_key(&name) {
                state.runtime_array_owners.insert(name, false);
            }
        }
        return;
    }
    if let ScalarExpression::Name { name, .. } = expression {
        state.runtime_array_owners.insert(name.clone(), false);
    }
}

fn return_bindings(block: &ScalarBlock) -> BTreeMap<String, ScalarExpression> {
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
                if assignment.targets.len() == 1 {
                    bindings.insert(assignment.target.clone(), assignment.value.clone());
                }
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
        | ScalarExpression::CheckedAddress { .. }
        | ScalarExpression::Dereference { .. }
        | ScalarExpression::IndexedRead { .. }
        | ScalarExpression::StructLiteral { .. }
        | ScalarExpression::ArrayLiteral { .. }
        | ScalarExpression::Binary { .. }
        | ScalarExpression::Unary { .. }
        | ScalarExpression::Call { .. } => {}
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
        ScalarExpression::Member { .. }
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

fn emit_runtime_array_allocation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
) -> Result<PointerValue<'ctx>, String> {
    declare_core_runtime(context, state.module);
    let (element_size, alignment) = allocation_layout(element, state.structs, state.target_layout)?;
    let size = state
        .builder
        .build_int_mul(
            length,
            context.i64_type().const_int(element_size, false),
            "allocation_size",
        )
        .map_err(builder_error)?;
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
                context.i64_type().const_int(alignment, false).into(),
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
    Ok(allocation)
}

fn binding_receivers(binding: &crate::ScalarBinding) -> Vec<(String, ScalarType)> {
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
    automatic_return_result_results: &'ctx BTreeSet<String>,
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
    let mut state = EmitState {
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: return_bindings(&function.body),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        automatic_return_result_results,
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, items);
    if let ScalarType::Callable { parameters, .. } = &function.signature {
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
    emit_return(
        context,
        &mut state,
        result,
        outputs,
        function
            .body
            .final_output_values
            .first()
            .map(|output| &output.value),
    )
}

fn emit_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    automatic_return_result_results: &'ctx BTreeSet<String>,
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        automatic_return_result_results,
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
    automatic_return_result_results: &'ctx BTreeSet<String>,
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
    let mut state = EmitState {
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: return_bindings(&function.body),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        automatic_return_result_results,
        globals: module_globals(source_module, globals),
        all_globals: globals.clone(),
        structs,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, &source_module.items);
    if let ScalarType::Callable { parameters, .. } = &function.signature {
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
    emit_return(
        context,
        &mut state,
        result,
        outputs,
        function
            .body
            .final_output_values
            .first()
            .map(|output| &output.value),
    )
}

fn emit_project_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    automatic_return_result_results: &'ctx BTreeSet<String>,
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        return_bindings: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        automatic_return_result_results,
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
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
    let current_module_globals = module_globals(module, globals);
    let current_globals = std::mem::replace(&mut state.globals, current_module_globals);
    state.values.clear();
    state.storage.clear();
    state.runtime_array_owners.clear();
    state.runtime_array_allocations.clear();
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
    let mut retained_return_results = automatic_return_result_owners;
    retained_return_results.extend(std::mem::take(&mut state.automatic_return_result_owners));
    state.automatic_return_result_owners = retained_return_results;
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
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
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
    release_scope_runtime_array_owners(context, state, &runtime_array_owners)?;
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    state.storage = storage;
    state.values = values;
    state.runtime_array_owners = runtime_array_owners;
    state.runtime_array_allocations = runtime_array_allocations;
    state.automatic_return_result_owners = automatic_return_result_owners;
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
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
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
    release_scope_runtime_array_owners(context, state, &runtime_array_owners)?;
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    state.storage = storage;
    state.values = values;
    state.runtime_array_owners = runtime_array_owners;
    state.runtime_array_allocations = runtime_array_allocations;
    state.automatic_return_result_owners = automatic_return_result_owners;
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
            return emit_expression(context, state, &output.value);
        }
        let value = emit_typed_expression(context, state, &output.value, &output.ty)?;
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
            return emit_project_expression(context, state, &output.value, module, modules);
        }
        let value = emit_project_typed_expression(
            context,
            state,
            &output.value,
            &output.ty,
            module,
            modules,
        )?;
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
    for (target, (value, ty, provenance)) in assignment.targets.iter().zip(values) {
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
    for (target, (value, ty, provenance)) in assignment.targets.iter().zip(values) {
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
        transition_runtime_array_assignment(context, state, target, &ty, &provenance)?;
        store_value(context, state, destination, &ty, value)?;
    }
    Ok(EmitValue::Unit)
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
                values.push((EmitValue::Unit, ty, provenance.clone()));
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
            values.push((value, ty, provenance.clone()));
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
            type_arguments,
            arguments,
            overload_selection,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "cast" {
                return emit_cast(context, state, type_arguments, arguments);
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
            type_arguments,
            arguments,
            overload_selection,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "cast" {
                let values = emit_cast_project_values(
                    context,
                    state,
                    type_arguments,
                    arguments,
                    module,
                    modules,
                )?;
                return Ok(values);
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
            emit_call_values(state, &target, values)
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
    emit_call_values(state, &target, values)
}

fn emit_cast<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    if type_arguments.len() != 1 || type_arguments[0].ty != ScalarType::U32 {
        return Err("core.cast has an invalid destination type argument".to_owned());
    }
    let [value, mode] = arguments else {
        return Err("core.cast has invalid argument arity".to_owned());
    };
    if !matches!(mode, ScalarExpression::Utf8 { value, .. } if value == b"exact") {
        return Err("core.cast requires the static mode \"exact\"".to_owned());
    }
    let value = take_basic(emit_expression(context, state, value)?)?.into_int_value();
    Ok(EmitValue::Basic(
        state
            .builder
            .build_int_truncate(value, context.i32_type(), "cast")
            .map_err(builder_error)?
            .into(),
    ))
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
fn emit_cast_project_values<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    type_arguments: &[crate::scalar::ScalarTypeArgument],
    arguments: &[ScalarExpression],
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    if type_arguments.len() != 1 || type_arguments[0].ty != ScalarType::U32 {
        return Err("core.cast has an invalid destination type argument".to_owned());
    }
    let [value, mode] = arguments else {
        return Err("core.cast has invalid argument arity".to_owned());
    };
    if !matches!(mode, ScalarExpression::Utf8 { value, .. } if value == b"exact") {
        return Err("core.cast requires the static mode \"exact\"".to_owned());
    }
    let value = take_basic(emit_project_expression(
        context, state, value, module, modules,
    )?)?
    .into_int_value();
    Ok(EmitValue::Basic(
        state
            .builder
            .build_int_truncate(value, context.i32_type(), "cast")
            .map_err(builder_error)?
            .into(),
    ))
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
    state: &mut EmitState<'ctx, 'module>,
    name: &str,
    values: Vec<EmitValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let function = *state
        .functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM callable {name}"))?;
    let arguments = values
        .into_iter()
        .map(|value| match value {
            EmitValue::Basic(value) => Ok(BasicMetadataValueEnum::from(value)),
            EmitValue::Unit => Err("unit argument is invalid".into()),
            EmitValue::Aggregate { .. } => Err("aggregate argument is invalid".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let call = state
        .builder
        .build_call(function, &arguments, "call")
        .map_err(builder_error)?;
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
    let then_value = emit(state, then_branch)?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let then_end = state.builder.get_insert_block().expect("then block");
    state.builder.position_at_end(else_block);
    let else_value = emit(state, else_branch)?;
    state
        .builder
        .build_unconditional_branch(merge)
        .map_err(builder_error)?;
    let else_end = state.builder.get_insert_block().expect("else block");
    state.builder.position_at_end(merge);
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

fn automatic_return_result_materialization_type(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    output: &ScalarType,
    visited: &mut BTreeSet<String>,
) -> Option<ScalarType> {
    let ScalarType::CheckedReference { inner, .. } = output else {
        return None;
    };
    if !automatic_return_result_copy_pointee(inner) {
        return None;
    }
    match expression {
        ScalarExpression::CheckedAddress { .. } => Some(*inner.clone()),
        ScalarExpression::Name { name, .. } if visited.insert(name.clone()) => {
            state.return_bindings.get(name).and_then(|value| {
                automatic_return_result_materialization_type(state, value, output, visited)
            })
        }
        _ => None,
    }
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
    expression: Option<&ScalarExpression>,
) -> Result<(), String> {
    match outputs.outputs.as_slice() {
        [] => {
            state.builder.build_return(None).map_err(builder_error)?;
        }
        [output] if output.ty != ScalarType::Unit => {
            let mut value = take_basic(value)?;
            if let Some(expression) = expression {
                if let Some(aggregate_type) = automatic_return_result_materialization_type(
                    state,
                    expression,
                    &output.ty,
                    &mut BTreeSet::new(),
                ) {
                    value = emit_automatic_return_result(context, state, value, &aggregate_type)?;
                }
            }
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

fn project_function_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    let mut symbol = String::from("wosy_fn");
    for component in [
        source.project.as_str(),
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
mod tests {
    use std::collections::BTreeMap;

    use inkwell::context::Context;

    use super::emit_scalar_llvm;
    use super::emit_scalar_project_llvm;
    use super::function_type;
    use super::project_function_name;
    use super::project_global_name;
    use super::storage_type;
    use super::{LlvmFunction, LlvmFunctionAttributes, LlvmPartition, LlvmValueType};
    use crate::scalar::ScalarOverloadSelection;
    use crate::{
        derive_scalar_program, derive_scalar_program_with_layout, parse_source,
        validate_scalar_project, ScalarModule, ScalarOutput, ScalarOutputSequence, ScalarProject,
        ScalarTargetLayout, ScalarType,
    };
    use wosy_syntax::{ByteSpan, SourceIdentity};

    #[test]
    fn rejects_fixed_array_lengths_that_exceed_llvm_array_size() {
        let context = Context::create();
        let array = ScalarType::Array {
            element: Box::new(ScalarType::U8),
            length: u64::from(u32::MAX) + 1,
            length_span: ByteSpan::new(0, 1),
            span: ByteSpan::new(0, 1),
        };
        assert_eq!(
            storage_type(&context, &array, &[], ScalarTargetLayout::NATIVE64).unwrap_err(),
            "fixed array length exceeds LLVM array size"
        );
    }

    #[test]
    fn accepts_fixed_array_lengths_at_llvm_array_size_limit() {
        let context = Context::create();
        let array = ScalarType::Array {
            element: Box::new(ScalarType::U8),
            length: u64::from(u32::MAX),
            length_span: ByteSpan::new(0, 1),
            span: ByteSpan::new(0, 1),
        };
        assert!(storage_type(&context, &array, &[], ScalarTargetLayout::NATIVE64).is_ok());
    }

    #[test]
    fn lowers_nested_project_while_loops_with_runtime_array_operations_and_conditionals() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let scalar = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ni32(u64) count = fn(length) { u8[length] bytes; i32 outer = 0; while (outer < 1) { i32 index = 0; while (index < 1) { if (index == 0) { bytes[0] = 1; }; index = index + 1; } outer = outer + 1; } outer };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
        let program = scalar.program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let llvm = emit_scalar_project_llvm(&validation)
            .expect("project loop LLVM")
            .to_text();
        assert!(llvm.contains("while.cond.0:"), "{llvm}");
        assert!(llvm.contains("while.cond.3:"), "{llvm}");
        assert!(llvm.contains("getelementptr inbounds i8"), "{llvm}");
    }

    #[test]
    fn lowers_integer_literals_in_binary_expressions_to_the_left_operand_width() {
        let validation = derive_scalar_program(
            &parse_source(
                SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "src/main.w".into(),
                    "r1".into(),
                ),
                "%%start\nbool() equal = fn { u8 value = 10; value == 10 };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let llvm = emit_scalar_llvm(&validation)
            .expect("integer comparison LLVM")
            .to_text();
        assert!(llvm.contains("icmp eq i8"), "{llvm}");
    }

    #[test]
    fn emits_multiple_unit_conditionals_in_a_while_body() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { while (running) { if (running) { running = false; } else { running = false; }; if (running) { running = running; } else { running = running; }; } running = false; 1, true } else { 2, false } };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let llvm = emit_scalar_llvm(&validation)
            .expect("multiple loop unit conditional LLVM")
            .to_text();
        assert_eq!(
            llvm.lines()
                .filter(|line| line.starts_with("while.cond.") && line.contains(':'))
                .count(),
            1,
            "{llvm}"
        );
    }

    #[test]
    fn emits_project_multiple_unit_conditionals_in_a_while_body_once() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let scalar = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { while (running) { if (running) { running = false; } else { running = false; }; if (running) { running = running; } else { running = running; }; } running = false; 1, true } else { 2, false } };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
        let program = scalar.program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let llvm = emit_scalar_project_llvm(&validation)
            .expect("project multiple loop unit conditional LLVM")
            .to_text();
        assert_eq!(
            llvm.lines()
                .filter(|line| line.starts_with("while.cond.") && line.contains(':'))
                .count(),
            1,
            "{llvm}"
        );
    }

    #[test]
    fn emits_recursive_fixed_array_index_reads_and_addresses_for_both_targets() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = "%%start\nu8() read = fn { u8[2][2] matrix = [[1, 2], [3, 4]]; u64 row = 1; u64 column = 0; u8 value = matrix[row][column]; *u8 shared = &matrix[row][column]; shared; unsafe { *?u8 raw = &?matrix[row][column]; raw; }; value };\n%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("indexed LLVM")
                .to_text();
            assert!(
                llvm.contains("getelementptr inbounds [2 x [2 x i8]]"),
                "{llvm}"
            );
            assert!(llvm.contains("getelementptr inbounds [2 x i8]"), "{llvm}");
            assert!(llvm.contains("ptrtoint"), "{llvm}");
            assert!(!llvm.contains("icmp ult"), "{llvm}");
        }
    }

    #[test]
    fn emits_runtime_array_allocation_and_indexing_for_both_targets() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = "%%start\nu8(u64) read = fn(length) { u8[length] bytes; bytes[0] = 7; bytes[0] };\n%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("runtime array LLVM")
                .to_text();
            assert!(
                llvm.contains("declare ptr @__wosy_core_alloc(i64, i64)"),
                "{llvm}"
            );
            assert!(
                llvm.contains("call ptr @__wosy_core_alloc(i64 %allocation_size, i64 1)"),
                "{llvm}"
            );
            assert!(llvm.contains("icmp eq ptr %allocation, null"), "{llvm}");
            assert!(
                llvm.contains("call void @__wosy_core_system_panic()\n  unreachable"),
                "{llvm}"
            );
            assert!(llvm.contains("getelementptr inbounds i8"), "{llvm}");
            assert!(!llvm.contains("icmp ult"), "{llvm}");
        }
    }

    #[test]
    fn emits_one_release_for_a_moved_top_level_runtime_array_for_both_targets() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = "%%start\nu64 length = 1;\nu8[length] first;\nu8[] second = first;\nsecond[0] = 7;\nu8 value = second[0];\n%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("moved runtime array LLVM")
                .to_text();
            assert!(llvm.contains("call ptr @__wosy_core_alloc"), "{llvm}");
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
        }
    }

    #[test]
    fn transfers_runtime_array_ownership_through_a_returned_aggregate_pointer() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() make_packet = fn { u64 length = 1; u8[length] bytes; bytes[0] = 4; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result };\nunit() release = fn { u64 length = 1; u8[length] temporary; temporary[0] = 9; };\n%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("returned aggregate pointer LLVM")
                .to_text();
            let packet = &llvm[llvm.find("@make_packet(").expect("packet function")..];
            let packet = &packet[..packet.find("\n}").expect("packet function end")];
            assert!(!packet.contains("call void @__wosy_core_free"), "{llvm}");
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
        }
    }

    #[test]
    fn emits_shared_core_allocation_release_and_panic_abi_for_both_targets() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = "%%start\nunit() release = fn { unsafe { *?u8 storage = core.alloc(16, 8); core.free(storage); }; };\nunit() panic = fn { core.system_panic(); };\ni64(u64) allocate = fn(length) { i64[length] values; values[0] = 7; values[0] };\n%%end";
        for (layout, pointer_width) in [
            (ScalarTargetLayout::WASM32, 32),
            (ScalarTargetLayout::NATIVE64, 64),
        ] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("core allocation LLVM")
                .to_text();
            assert!(
                llvm.contains("declare ptr @__wosy_core_alloc(i64, i64)"),
                "{llvm}"
            );
            assert!(
                llvm.contains("declare void @__wosy_core_free(ptr)"),
                "{llvm}"
            );
            assert!(
                llvm.contains("declare void @__wosy_core_system_panic()"),
                "{llvm}"
            );
            assert!(
                llvm.contains("call ptr @__wosy_core_alloc(i64 16, i64 8)"),
                "{llvm}"
            );
            assert!(
                llvm.contains(&format!("ptrtoint ptr %allocation to i{pointer_width}")),
                "{llvm}"
            );
            assert!(
                llvm.contains(&format!("inttoptr i{pointer_width}")),
                "{llvm}"
            );
            assert!(
                llvm.contains("call void @__wosy_core_free(ptr %free_pointer)"),
                "{llvm}"
            );
            assert!(
                llvm.contains("call ptr @__wosy_core_alloc(i64 %allocation_size, i64 8)"),
                "{llvm}"
            );
            assert!(llvm.contains("allocation_panic:"), "{llvm}");
            assert!(llvm.contains("unreachable"), "{llvm}");
            assert!(llvm.contains("define void @panic()"), "{llvm}");
        }
    }

    #[test]
    fn retains_core_runtime_declarations_when_serializing_wasm_externs() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source,
            "%%start\nenv = extern wasm \"host\" { unit() log; };\nunit() run = fn { u64 length = 1; u8[length] bytes; bytes[0] = 1; env.log(); };\nrun();\n%%end".into(),
            &[],
        );
        let validation =
            crate::derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let llvm = emit_scalar_llvm(&validation)
            .expect("core runtime LLVM with wasm extern")
            .to_text();
        assert!(
            llvm.contains("declare ptr @__wosy_core_alloc(i64, i64)"),
            "{llvm}"
        );
        assert!(
            llvm.contains("declare void @__wosy_core_system_panic()"),
            "{llvm}"
        );
        assert!(llvm.contains("wasm-import-module\"=\"host\""), "{llvm}");
    }

    #[test]
    fn emits_mutable_typed_place_writes_for_wasm32_and_native64() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = "%%start\nstruct Buffer { u8 value; u8[3] bytes; }\nu8() write = fn { u8[3] values = [1, 2, 3]; Buffer holder = { .value = 4; .bytes = [5, 6, 7]; }; *!u8 writer = &!values[1]; values[0] = 8; holder.value = 9; *writer = 10; holder.bytes[2] = 11; values[0], values[2] = 12, 13; values[0] };\n%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("mutable place LLVM")
                .to_text();
            assert!(llvm.contains("getelementptr inbounds [3 x i8]"), "{llvm}");
            assert!(llvm.contains("inttoptr"), "{llvm}");
            assert!(!llvm.contains("icmp ult"), "{llvm}");
        }
    }

    #[test]
    fn serializes_each_extern_with_its_wasm_import_attribute_group() {
        let partition = LlvmPartition {
            module_name: "test".into(),
            declarations: vec![
                LlvmFunction {
                    name: "first".into(),
                    result: LlvmValueType::I32,
                    parameters: Vec::new(),
                    attributes: vec![LlvmFunctionAttributes {
                        group: 3,
                        wasm_import_module: "one".into(),
                        wasm_import_name: "first_import".into(),
                    }],
                    body: String::new(),
                },
                LlvmFunction {
                    name: "second".into(),
                    result: LlvmValueType::Void,
                    parameters: Vec::new(),
                    attributes: vec![LlvmFunctionAttributes {
                        group: 7,
                        wasm_import_module: "two".into(),
                        wasm_import_name: "second_import".into(),
                    }],
                    body: String::new(),
                },
            ],
            functions: Vec::new(),
            text: "; ModuleID = 'test'\nsource_filename = \"test\"\n\ndeclare i32 @first()\ndeclare void @second()\n".into(),
        };

        let text = partition.to_text();

        assert!(text.contains("declare i32 @first() #3"), "{text}");
        assert!(text.contains("declare void @second() #7"), "{text}");
        assert!(
            text.contains(
                "attributes #3 = { \"wasm-import-module\"=\"one\" \"wasm-import-name\"=\"first_import\" }"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "attributes #7 = { \"wasm-import-module\"=\"two\" \"wasm-import-name\"=\"second_import\" }"
            ),
            "{text}"
        );
    }

    #[test]
    fn preserves_callable_return_metadata_for_every_output_form() {
        let context = Context::create();
        let sequence = |types: Vec<ScalarType>| ScalarOutputSequence {
            outputs: types
                .into_iter()
                .map(|ty| ScalarOutput {
                    ty,
                    span: wosy_syntax::ByteSpan::new(0, 0),
                })
                .collect(),
            span: wosy_syntax::ByteSpan::new(0, 0),
        };
        let callable = |outputs| ScalarType::Callable {
            outputs,
            parameters: Vec::new(),
        };

        assert_eq!(
            function_type(
                &context,
                &callable(sequence(Vec::new())),
                ScalarTargetLayout::WASM32,
            )
            .expect("zero-output function type")
            .1,
            LlvmValueType::Void
        );
        assert_eq!(
            function_type(
                &context,
                &callable(sequence(vec![ScalarType::I32])),
                ScalarTargetLayout::WASM32,
            )
            .expect("direct function type")
            .1,
            LlvmValueType::I32
        );
        assert_eq!(
            function_type(
                &context,
                &callable(sequence(vec![ScalarType::I32, ScalarType::Bool])),
                ScalarTargetLayout::WASM32,
            )
            .expect("aggregate function type")
            .1,
            LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1])
        );
        assert_eq!(
            function_type(
                &context,
                &callable(sequence(vec![ScalarType::Unit, ScalarType::I32])),
                ScalarTargetLayout::WASM32,
            )
            .expect("unit aggregate function type")
            .1,
            LlvmValueType::Aggregate(vec![LlvmValueType::I8, LlvmValueType::I32])
        );
    }

    #[test]
    fn collects_function_and_declaration_aggregate_metadata() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"metadata\" { unit() void_result; i32() direct_result; (i32, bool)() aggregate_result; (unit, i32)() unit_aggregate_result; };\nunit() local_void = fn { env.void_result() };\ni32() local_direct = fn { env.direct_result() };\n(i32, bool)() local_aggregate = fn { env.aggregate_result() };\n(unit, i32)() local_unit_aggregate = fn { env.unit_aggregate_result() };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let partition = emit_scalar_llvm(&validation).expect("LLVM metadata");
        let function_results = partition
            .functions
            .iter()
            .filter(|function| function.name.starts_with("local_"))
            .map(|function| function.result.clone())
            .collect::<Vec<_>>();
        let declaration_results = partition
            .declarations
            .iter()
            .map(|declaration| declaration.result.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            function_results,
            vec![
                LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1]),
                LlvmValueType::I32,
                LlvmValueType::Aggregate(vec![LlvmValueType::I8, LlvmValueType::I32]),
                LlvmValueType::Void,
            ]
        );
        assert_eq!(
            declaration_results,
            vec![
                LlvmValueType::Void,
                LlvmValueType::I32,
                LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1]),
                LlvmValueType::Aggregate(vec![LlvmValueType::I8, LlvmValueType::I32]),
            ]
        );
    }

    #[test]
    fn emits_typed_scalar_calls_and_verified_module() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source,
            "%%start\nbool() flag = fn { true };\nunit() touch = fn { i32 marker = 0; marker = marker; };\ni32() count = fn { 42 };\nbool selected = flag();\nunit done = touch();\ni32 result = count();\n%%end"
                .into(),
            &[],
        );
        let partition = emit_scalar_llvm(&derive_scalar_program(&parsed.result)).expect("LLVM");
        let text = partition.to_text();
        assert!(text.contains("call i1 @flag"));
        assert!(text.contains("call void @touch"));
        assert!(text.contains("call i32 @count"));
    }

    #[test]
    fn emits_concrete_ordinary_generic_direct_call() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\ni64 result = identity<i64>(1);\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_llvm(&validation)
            .expect("ordinary generic LLVM")
            .to_text();

        assert!(text.contains("define i64 @wosy_generic__"), "{text}");
        assert!(text.contains("call i64 @wosy_generic__"), "{text}");

        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                validation.program.clone(),
                Vec::new(),
            )],
            vec![source],
        ));
        let project_text = emit_scalar_project_llvm(&project)
            .expect("project ordinary generic LLVM")
            .to_text();
        assert!(
            project_text.contains("call i64 @wosy_generic__"),
            "{project_text}"
        );
    }

    #[test]
    fn emits_concrete_ordinary_generic_multi_output_call() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ngeneric T;\n(T, T)(T) pair = fn(value) { value };\ni64 first, i64 second = pair<i64>(1);\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_llvm(&validation)
            .expect("ordinary generic multi-output LLVM")
            .to_text();

        assert!(
            text.contains("define { i64, i64 } @wosy_generic__"),
            "{text}"
        );
        assert!(text.contains("call { i64, i64 } @wosy_generic__"), "{text}");
        assert!(text.contains("extractvalue { i64, i64 }"), "{text}");
    }

    #[test]
    fn lowers_ordinary_and_generic_overload_arms_with_ordered_outputs() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ncast = overload {\n    i32(i64) => fn(value) { 1 };\n    generic T;\n    T(T) => fn(value) { value };\n};\npair = overload {\n    generic T;\n    (T, T)(T) => fn(value) { value };\n};\ni64 wide = 1;\ni32 narrow = cast(wide);\nbool flag = true;\nbool copied = cast(flag);\ni64 first, i64 second = pair(wide);\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_llvm(&validation)
            .expect("overload LLVM")
            .to_text();

        assert!(text.contains("define i32 @wosy_generic__"), "{text}");
        assert!(text.contains("define i1 @wosy_generic__"), "{text}");
        assert!(
            text.contains("define { i64, i64 } @wosy_generic__"),
            "{text}"
        );
        assert!(text.contains("call i32 @wosy_generic__"), "{text}");
        assert!(text.contains("call i1 @wosy_generic__"), "{text}");
        assert!(text.contains("call { i64, i64 } @wosy_generic__"), "{text}");
        assert!(
            text.contains("extractvalue { i64, i64 } %call3, 0"),
            "{text}"
        );
        assert!(
            text.contains("extractvalue { i64, i64 } %call3, 1"),
            "{text}"
        );

        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                validation.program.clone(),
                Vec::new(),
            )],
            vec![source],
        ));
        let project_text = emit_scalar_project_llvm(&project)
            .expect("project overload LLVM")
            .to_text();
        assert!(
            project_text.contains("call { i64, i64 } @wosy_generic__"),
            "{project_text}"
        );
    }

    #[test]
    fn emits_distinct_overload_arms_for_shared_input_types() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nselect = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n    (bool, i32)(i64) => fn(value) { true, 1 };\n};\ni64 input = 1;\ni32 integer, bool flag = select(input);\nbool boolean, i32 number = select(input);\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_llvm(&validation)
            .expect("selected overload LLVM")
            .to_text();
        assert!(
            text.contains("define { i32, i1 } @wosy_generic__"),
            "{text}"
        );
        assert!(
            text.contains("define { i1, i32 } @wosy_generic__"),
            "{text}"
        );
        assert!(text.contains("call { i32, i1 } @wosy_generic__"), "{text}");
        assert!(text.contains("call { i1, i32 } @wosy_generic__"), "{text}");
    }

    #[test]
    fn lowers_overload_arms_through_project_namespaces() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nidentity = overload {\n    i64(i64) => fn(value) { value };\n};\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\ni64 value = 1;\ni64 copied = child.identity(value);\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_project_llvm(&validation)
            .expect("namespaced overload LLVM")
            .to_text();
        assert!(text.contains("define i64 @wosy_generic__"), "{text}");
        assert!(text.contains("call i64 @wosy_generic__"), "{text}");
    }

    #[test]
    fn lowers_generic_overload_arms_through_project_namespaces() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nidentity = overload {\n    i32(i64) => fn(value) { 1 };\n    generic T;\n    T(T) => fn(value) { value };\n};\nbool initial = true;\nbool copied = identity(initial);\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let mut root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\nchar value = 'a';\nchar copied = child.identity(value);\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let crate::ScalarItem::Binding(binding) = root
            .items
            .last_mut()
            .expect("qualified generic overload binding")
        else {
            panic!("qualified generic overload binding");
        };
        let crate::ScalarExpression::Call {
            overload_selection, ..
        } = &mut binding.value
        else {
            panic!("qualified generic overload call");
        };
        *overload_selection = Some(ScalarOverloadSelection {
            arm_index: 1,
            substitutions: BTreeMap::from([("T".into(), crate::ScalarType::Char)]),
        });
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let validation = crate::ScalarProjectValidation {
            project: ScalarProject::new(
                vec![
                    ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                    ScalarModule::new(child_source.clone(), child.items, Vec::new()),
                ],
                vec![root_source, child_source],
            ),
            diagnostics: Vec::new(),
        };

        let text = emit_scalar_project_llvm(&validation)
            .expect("namespaced generic overload LLVM")
            .to_text();
        assert!(text.contains("define i32 @wosy_generic__"), "{text}");
        assert!(text.contains("call i32 @wosy_generic__"), "{text}");
    }

    #[test]
    fn emits_empty_utf8_call_arguments_with_resolved_parameter_types() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let mut program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start
struct utf8 {
    *?u8 data;
    u64 length;
}
u64(utf8) print = fn(text) { text.length };
utf8 value = { .data = null; .length = 0; };
u64 reported = print(value);
%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        let crate::ScalarItem::Binding(binding) = &mut program.program.items[2] else {
            panic!("call binding");
        };
        let crate::ScalarExpression::Call { arguments, .. } = &mut binding.value else {
            panic!("call expression");
        };
        arguments[0] = crate::ScalarExpression::Utf8 {
            value: Vec::new(),
            span: wosy_syntax::ByteSpan::new(0, 0),
        };
        let text = emit_scalar_llvm(&program)
            .expect("empty UTF-8 call LLVM")
            .to_text();

        assert!(
            text.contains("@wosy_utf8_literal_0 = private constant [0 x i8]"),
            "{text}"
        );
        assert!(text.contains("store i64 0"), "{text}");
        assert!(
            text.contains("call i64 @print(ptr %utf8_literal)"),
            "{text}"
        );
    }

    #[test]
    fn emits_project_public_empty_utf8_output_call() {
        let std_source = SourceIdentity::new(
            "project".into(),
            "stdlib".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let std = derive_scalar_program(
            &parse_source(
                std_source.clone(),
                "%%start
struct utf8 {
    *?u8 data;
    u64 length;
}
(u64, bool)(utf8) print = fn(text) { text.length, true };
%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start
text = namespace stdlib \"src/bootstrap.w\";
u64 reported, bool complete = text.print(\"\");
%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    root,
                    vec![crate::ScalarNamespaceBinding {
                        binding: "text".into(),
                        target: std_source.clone(),
                        span: wosy_syntax::ByteSpan::new(0, 0),
                    }],
                ),
                ScalarModule::from_program(std, Vec::new()),
            ],
            vec![source, std_source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("project public empty UTF-8 output LLVM")
            .to_text();

        assert!(
            text.contains("@wosy_utf8_literal_0 = private constant [0 x i8]"),
            "{text}"
        );
        assert!(text.contains("store i64 0"), "{text}");
        assert!(
            text.contains(&format!(
                "call {{ i64, i1 }} @{}(ptr %utf8_literal)",
                project_function_name(&std_source, "print")
            )),
            "{text}"
        );
    }

    #[test]
    fn emits_wasi_read_into_with_one_iovec_for_wasm32_and_native64() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/read_into.w".into(),
            "r1".into(),
        );
        let text = "%%start
struct _Iovec { *?u8 data; u32 length; }
struct _Nread { u32 value; }
_wasi = extern wasm \"wasi_snapshot_preview1\" { unsafe i32(i32, *?_Iovec, i32, *?_Nread) _fd_read; };
i32(i32, *?_Iovec, *?_Nread) _fd_read_once = fn(descriptor, iovec_address, count_address) { unsafe { _wasi._fd_read(descriptor, iovec_address, 1, count_address) } };
(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) {
	_Iovec entry = { .data = destination; .length = core.int_trunc<u32>(capacity); };
	_Nread count_cell = { .value = 0; };
	i32 result = 0;
	unsafe { result = _fd_read_once(0, &?entry, &?count_cell); };
	core.int_extend<u64>(count_cell.value), result == 0
};
*?u8 buffer = null;
u64 requested_capacity = 4;
u64 count = 0;
bool complete = false;
count, complete = read_into(buffer, requested_capacity);
%%end";
        for (layout, iovec_size) in [
            (ScalarTargetLayout::WASM32, 8),
            (ScalarTargetLayout::NATIVE64, 16),
        ] {
            let validation = derive_scalar_program_with_layout(
                &parse_source(source.clone(), text.into(), &[]).result,
                layout,
            );
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let llvm = emit_scalar_llvm(&validation)
                .expect("read_into LLVM")
                .to_text();
            let pointer_width = layout.pointer_size * 8;
            assert!(llvm.contains("wasm-import-name\"=\"fd_read"), "{llvm}");
            assert!(
                llvm.contains(&format!("i32 0, i{pointer_width} %address")),
                "{llvm}"
            );
            assert!(
                llvm.contains(&format!("i32 1, i{pointer_width} %count_address")),
                "{llvm}"
            );
            assert!(
                llvm.contains(&format!("alloca [{iovec_size} x i8]")),
                "{llvm}"
            );
            assert!(llvm.contains("trunc i64"), "{llvm}");
            assert!(llvm.contains("zext i32"), "{llvm}");
            assert!(!llvm.contains("icmp eq i32 %destination"), "{llvm}");
        }
    }

    #[test]
    fn emits_generic_integer_conversions_for_single_file_and_project() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ni8() signed_result = fn { 1 };\ni16 signed_from_call = core.int_extend<i16>(signed_result());\ni64 signed_source = 42;\ni128 signed_wide = core.int_extend<i128>(signed_source);\nu64 unsigned_source = 42;\nu128 unsigned_wide = core.int_extend<u128>(unsigned_source);\ni8 signed_narrow = core.int_trunc<i8>(signed_wide);\nu8 unsigned_narrow = core.int_trunc<u8>(unsigned_wide);\n%%end".into(),
                &[],
            )
            .result,
        );
        let single = emit_scalar_llvm(&program)
            .expect("single-file conversion LLVM")
            .to_text();
        assert!(single.contains("sext i64"), "{single}");
        assert!(single.contains("sext i8"), "{single}");
        assert!(single.contains("zext i64"), "{single}");
        assert!(single.contains("trunc i128"), "{single}");

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = emit_scalar_project_llvm(&validation)
            .expect("project conversion LLVM")
            .to_text();
        assert!(project.contains("sext i64"), "{project}");
        assert!(project.contains("zext i64"), "{project}");
        assert!(project.contains("trunc i128"), "{project}");
    }

    #[test]
    fn emits_float_conversions_for_single_file_and_project() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nu64 uint_source = 42;\ni64 sint_source = 42;\nf64 wide_source = 1.5;\nf32 narrow_source = 1.5;\nf64 from_uint = core.uint_to_float<f64>(uint_source);\nf32 from_sint = core.sint_to_float<f32>(sint_source);\nf64 from_uint_literal = core.uint_to_float<f64>(7);\ni32 to_sint = core.float_to_sint_trunc<i32>(wide_source);\nu64 to_uint = core.float_to_uint_trunc<u64>(narrow_source);\nf32 narrowed = core.float_trunc<f32>(wide_source);\nf64 widened = core.float_extend<f64>(narrow_source);\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        let single = emit_scalar_llvm(&program)
            .expect("single-file float conversion LLVM")
            .to_text();
        assert!(single.contains("uitofp"), "{single}");
        assert!(single.contains("sitofp"), "{single}");
        assert!(single.contains("fptosi"), "{single}");
        assert!(single.contains("fptoui"), "{single}");
        assert!(single.contains("fptrunc"), "{single}");
        assert!(single.contains("fpext"), "{single}");

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = emit_scalar_project_llvm(&validation)
            .expect("project float conversion LLVM")
            .to_text();
        assert!(project.contains("uitofp"), "{project}");
        assert!(project.contains("sitofp"), "{project}");
        assert!(project.contains("fptosi"), "{project}");
        assert!(project.contains("fptoui"), "{project}");
        assert!(project.contains("fptrunc"), "{project}");
        assert!(project.contains("fpext"), "{project}");
    }

    #[test]
    fn emits_bitcast_for_single_file_and_project() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nchar letter = 'A';\nu32 code = core.bitcast<u32>(letter);\nf32 bits = core.bitcast<f32>(code);\nu32 roundtrip = core.bitcast<u32>(bits);\ni64 wider = 42;\nf64 dwbits = core.bitcast<f64>(wider);\ni64 back = core.bitcast<i64>(dwbits);\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        let single = emit_scalar_llvm(&program)
            .expect("single-file bitcast LLVM")
            .to_text();
        assert!(single.contains("bitcast"), "{single}");

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = emit_scalar_project_llvm(&validation)
            .expect("project bitcast LLVM")
            .to_text();
        assert!(project.contains("bitcast"), "{project}");
    }

    #[test]
    fn emits_raw_offset_and_load_for_single_file_and_project() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index)) } };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        let single = emit_scalar_llvm(&program)
            .expect("single-file offset and load LLVM")
            .to_text();
        assert!(single.contains("inttoptr"), "{single}");
        assert!(single.contains("getelementptr inbounds i8"), "{single}");
        assert!(single.contains("ptrtoint"), "{single}");
        assert!(single.contains("load i8"), "{single}");

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = emit_scalar_project_llvm(&validation)
            .expect("project offset and load LLVM")
            .to_text();
        assert!(project.contains("inttoptr"), "{project}");
        assert!(project.contains("getelementptr inbounds i8"), "{project}");
        assert!(project.contains("ptrtoint"), "{project}");
        assert!(project.contains("load i8"), "{project}");
    }

    #[test]
    fn emits_generic_integer_conversion_for_struct_member_source() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct utf8 {\n\tu64 length;\n}\nu32(utf8) truncate = fn(text) { core.int_trunc<u32>(text.length) };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        let single = emit_scalar_llvm(&program)
            .expect("single-file member conversion LLVM")
            .to_text();
        assert!(single.contains("trunc i64"), "{single}");

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = emit_scalar_project_llvm(&validation)
            .expect("project member conversion LLVM")
            .to_text();
        assert!(project.contains("trunc i64"), "{project}");
    }

    #[test]
    fn emits_unary_bitwise_and_shift_instructions() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/operators.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nbool(bool) logical_not = fn(value) { !value };\ni32(i32) bitwise_not = fn(value) { ~value };\ni32(i32, i32) bitwise = fn(left, right) { (left & right) | (left ^ right) };\ni32(i32, i32) signed_shift = fn(value, count) { (value << count) >> count };\nu32(u32, u32) unsigned_shift = fn(value, count) { value >> count };\nbool inverted = logical_not(true);\ni32 complemented = bitwise_not(1);\ni32 combined = bitwise(7, 3);\ni32 shifted = signed_shift(8, 1);\nu32 logical_shifted = unsigned_shift(8, 1);\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("operator LLVM")
            .to_text();
        assert!(text.contains("xor i1"), "{text}");
        assert!(text.contains("xor i32"), "{text}");
        assert!(text.contains("and i32"), "{text}");
        assert!(text.contains("or i32"), "{text}");
        assert!(text.contains("shl i32"), "{text}");
        assert!(text.contains("ashr i32"), "{text}");
        assert!(text.contains("lshr i32"), "{text}");
    }

    #[test]
    fn emits_typed_integer_negation_and_signed_division_remainder() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/negate.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ni32(i32) negate = fn(value) { -value };\ni8(i8) negate_narrow = fn(value) { -value };\ni32 quotient = -7 / 3;\ni32 remainder = -7 % 3;\ni32 computed_quotient = negate(7) / 3;\ni32 computed_remainder = negate(7) % 3;\ni8 seed = 7;\ni8 signed = negate_narrow(seed);\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("negation LLVM")
            .to_text();
        assert!(text.contains("store i32 -2"), "{text}");
        assert!(text.contains("store i32 -1"), "{text}");
        assert!(text.contains("sub i32 0, %value"), "{text}");
        assert!(text.contains("sdiv i32 %call, 3"), "{text}");
        assert!(text.contains("srem i32 %call1, 3"), "{text}");
        assert!(text.contains("sub i8 0, %value"), "{text}");
        assert!(text.contains("store i8 %call2"), "{text}");
    }

    #[test]
    fn emits_unit_if_with_then_and_merge_blocks_without_phi() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/unit_if.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (true) { touch(); }; };\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("unit conditional LLVM")
            .to_text();
        let run = text.split("define void @run").nth(1).expect("run function");
        assert!(
            run.contains("br i1 true, label %if.then, label %if.merge"),
            "{run}"
        );
        assert!(run.contains("if.then:"), "{run}");
        assert!(run.contains("if.merge:"), "{run}");
        assert!(!run.contains(" phi "), "{run}");
    }

    #[test]
    fn emits_float_types_literals_operations_and_comparisons() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/floats.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nf32(f32) identity = fn(value) { value };\nf32 a = 1.25;\nf64 b = 2e1;\nf32 sum = a + 2.5;\nf32 echoed = identity(4e-1);\nb < 3e1;\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation).expect("float LLVM").to_text();
        assert!(text.contains("global float"), "{text}");
        assert!(text.contains("global double"), "{text}");
        assert!(text.contains("fadd float"), "{text}");
        assert!(text.contains("fcmp olt double"), "{text}");
        assert!(text.contains("call float @identity(float"), "{text}");
    }

    #[test]
    fn emits_distinct_char_and_artifact_id_primitive_signatures() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"primitive\" { char() read_char; artifact_id() read_artifact; };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let partition = emit_scalar_llvm(&validation).expect("primitive LLVM");
        let text = partition.to_text();
        assert!(text.contains("declare i32 @wosy_extern__"));
        assert!(text.contains("declare ptr @wosy_extern__"));
        assert!(text.contains("wasm-import-name\"=\"read_char"));
        assert!(text.contains("wasm-import-name\"=\"read_artifact"));
    }

    #[test]
    fn emits_generic_extern_module_symbol_and_signature() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"helper\" { i32(i32) read; unit() flush; };\ni32 value = env.read(7);\nenv.flush();\n%%end".into(),
                &[],
            )
            .result,
        );
        let partition = emit_scalar_llvm(&validation).expect("generic extern LLVM");
        assert_eq!(partition.declarations.len(), 2);
        assert!(partition.declarations[0].name.starts_with("wosy_extern__"));
        assert_eq!(
            partition.declarations[0].attributes[0].wasm_import_module,
            "helper"
        );
        assert!(partition.declarations[1].name.starts_with("wosy_extern__"));
        assert!(partition.to_text().contains("declare i32 @wosy_extern__"));
        assert!(partition.to_text().contains("declare void @wosy_extern__"));
        assert!(partition.to_text().contains("call i32 @wosy_extern__"));
    }

    #[test]
    fn emits_injective_extern_identity_for_same_symbols() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nleft = extern wasm \"shared\" { i32() same; };\nright = extern wasm \"shared\" { i32() same; };\nother = extern wasm \"other\" { i32() same; };\ni32 first = left.same();\ni32 second = right.same();\ni32 third = other.same();\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let partition = emit_scalar_llvm(&validation).expect("colliding extern LLVM");
        assert_eq!(partition.declarations.len(), 3, "{}", partition.to_text());
        assert_eq!(
            partition
                .declarations
                .iter()
                .map(|declaration| declaration.name.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            partition
                .declarations
                .iter()
                .map(|declaration| declaration.attributes[0].wasm_import_name.as_str())
                .collect::<Vec<_>>(),
            vec!["same", "same", "same"]
        );
        assert_eq!(
            partition
                .declarations
                .iter()
                .map(|declaration| declaration.attributes[0].wasm_import_module.as_str())
                .collect::<Vec<_>>(),
            vec!["shared", "shared", "other"]
        );
        let text = partition.to_text();
        for declaration in &partition.declarations {
            assert!(text.contains(&format!("call i32 @{}()", declaration.name)));
            let attributes = &declaration.attributes[0];
            assert!(
                text.contains(&format!(
                    "declare i32 @{}() #{}",
                    declaration.name, attributes.group
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "attributes #{} = {{ \"wasm-import-module\"=\"{}\" \"wasm-import-name\"=\"{}\" }}",
                    attributes.group, attributes.wasm_import_module, attributes.wasm_import_name
                )),
                "{text}"
            );
        }
    }

    #[test]
    fn emits_generic_exact_u64_to_u32_cast_without_runtime_guard() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nu64 source = 42;\nu32 result = core.cast<u32>(source, \"exact\");\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation).expect("cast LLVM").to_text();
        assert!(text.contains("trunc i64"), "{text}");
        assert!(text.contains(" to i32"), "{text}");
        assert!(!text.contains("icmp"), "{text}");
    }

    #[test]
    fn emits_ordered_multi_output_aggregate_returns_calls_and_receivers() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"helper\" { (i32, u64)() read; };\n(i32, u64)() pair = fn { env.read() };\ni32 first, u64 second = pair();\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let partition = emit_scalar_llvm(&validation).expect("multi-output LLVM");
        let text = partition.to_text();
        assert!(text.contains("define { i32, i64 } @pair"), "{text}");
        assert!(
            text.contains("declare { i32, i64 } @wosy_extern__"),
            "{text}"
        );
        assert!(text.contains("call { i32, i64 } @pair"), "{text}");
        assert!(text.contains("extractvalue { i32, i64 }"), "{text}");
        assert!(
            text.contains("extractvalue { i32, i64 } %call, 0"),
            "{text}"
        );
        assert!(
            text.contains("extractvalue { i32, i64 } %call, 1"),
            "{text}"
        );
    }

    #[test]
    fn emits_nested_conditional_multi_outputs_in_aggregate_order() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_llvm(&validation)
            .expect("nested conditional LLVM")
            .to_text();
        let pair = text
            .split("define { i32, i1 } @pair")
            .nth(1)
            .expect("pair function");
        assert_eq!(pair.matches("phi { i32, i1 }").count(), 2, "{pair}");
        assert!(pair.contains("{ i32 1, i1 true }"), "{pair}");
        assert!(pair.contains("{ i32 2, i1 false }"), "{pair}");
        assert!(pair.contains("{ i32 3, i1 true }"), "{pair}");
        assert!(pair.contains("ret { i32, i1 }"), "{pair}");
    }

    #[test]
    fn materializes_multi_output_assignments_before_ordered_writes() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, i32)(i32, i32) pair = fn(first, second) { first, second };\ni32() swap = fn { i32 left = 1; i32 right = 2; right, left = pair(left, right); left };\ni32() inferred = fn { first, second = pair(3, 4); first + second };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("ordered assignment LLVM")
            .to_text();
        let swap = text.split("define i32 @swap").nth(1).expect("swap");
        let call = swap.find("call { i32, i32 } @pair").expect("pair call");
        let first_output = swap
            .find("extractvalue { i32, i32 } %call, 0")
            .expect("first output");
        let second_output = swap
            .find("extractvalue { i32, i32 } %call, 1")
            .expect("second output");
        let right_store = swap.rfind("ptr %right").expect("right store");
        let left_store = swap.rfind("ptr %left").expect("left store");
        assert!(call < first_output, "{swap}");
        assert!(first_output < second_output, "{swap}");
        assert!(second_output < right_store, "{swap}");
        assert!(right_store < left_store, "{swap}");

        let inferred = text.split("define i32 @inferred").nth(1).expect("inferred");
        assert!(inferred.contains("%first = alloca i32"), "{inferred}");
        assert!(inferred.contains("%second = alloca i32"), "{inferred}");
    }

    #[test]
    fn emits_distinct_independent_output_values_in_source_order() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ni32 first, u64 second = 1, 2;\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("independent output LLVM")
            .to_text();
        assert!(text.contains("store i32 1"), "{text}");
        assert!(text.contains("store i64 2"), "{text}");
    }

    #[test]
    fn emits_function_final_outputs_from_locals_in_order() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, u64)() pair = fn { i32 local = 1; local, 2 };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("final output LLVM")
            .to_text();
        let pair = text
            .split("define { i32, i64 } @pair")
            .nth(1)
            .expect("pair");
        assert!(pair.contains("load i32"), "{pair}");
        assert!(pair.contains("insertvalue { i32, i64 }"), "{pair}");
        assert!(
            pair.contains("insertvalue { i32, i64 } %output, i64 2, 1"),
            "{pair}"
        );
    }

    #[test]
    fn emits_project_final_outputs_with_single_file_parity() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\n(i32, u64)() pair = fn { i32 local = 1; local, 2 };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("project final output LLVM")
            .to_text();
        let pair = text
            .split(&format!(
                "define {{ i32, i64 }} @{}",
                project_function_name(&source, "pair")
            ))
            .nth(1)
            .expect("project pair");
        assert!(pair.contains("load i32"), "{pair}");
        assert!(
            pair.contains("insertvalue { i32, i64 } %output, i64 2, 1"),
            "{pair}"
        );
    }

    #[test]
    fn preserves_direct_one_output_and_void_zero_output_function_abis() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ni32() one = fn { i32 local = 1; local };\nunit() zero = fn { i32 local = 1; local; };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation).expect("arity LLVM").to_text();
        assert!(text.contains("define i32 @one"), "{text}");
        assert!(text.contains("define void @zero"), "{text}");
    }

    #[test]
    fn emits_equal_independent_output_values_with_typed_receivers() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let mut validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ni32 first, u64 second = 1, 1;\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let crate::ScalarItem::Binding(binding) = &mut validation.program.items[0] else {
            panic!("output binding");
        };
        binding.output_values[1].value = binding.output_values[0].value.clone();
        let text = emit_scalar_llvm(&validation)
            .expect("equal independent output LLVM")
            .to_text();
        assert!(text.contains("store i32 1"), "{text}");
        assert!(text.contains("store i64 1"), "{text}");
    }

    #[test]
    fn emits_zero_initialized_unit_output_fields_without_truncation() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"helper\" { (i32, unit, u64)() read; };\ni32 first, unit done, u64 last = env.read();\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("unit aggregate LLVM")
            .to_text();
        assert!(
            text.contains("declare { i32, i8, i64 } @wosy_extern__"),
            "{text}"
        );
        assert!(
            text.contains("extractvalue { i32, i8, i64 } %call, 0"),
            "{text}"
        );
        assert!(
            text.contains("extractvalue { i32, i8, i64 } %call, 2"),
            "{text}"
        );
        assert!(
            !text.contains("extractvalue { i32, i8, i64 } %call, 3"),
            "{text}"
        );
    }

    #[test]
    fn emits_ordered_multi_output_project_namespace_calls() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nenv = extern wasm \"helper\" { (i32, u64)() read; };\n(i32, u64)() pair = fn { env.read() };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\ni32 first, u64 second = child.pair();\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("project LLVM")
            .to_text();
        assert!(text.contains("define { i32, i64 } @wosy_fn__"), "{text}");
        assert!(text.contains("call { i32, i64 } @wosy_fn__"), "{text}");
    }

    #[test]
    fn emits_project_binding_store_and_function_load() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source.clone(),
            "%%start\ni32 count = 41;\ni32() read = fn { count + 1 };\n%%end".into(),
            &[],
        );
        let program = derive_scalar_program(&parsed.result).program;
        let project = ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        );
        let validation = validate_scalar_project(project);
        let text = emit_scalar_project_llvm(&validation)
            .expect("LLVM")
            .to_text();
        let main = text.split("define i32 @main").nth(1).expect("main");
        let read = text
            .split("define i32 @wosy_fn__70726f6a656374__7061636b616765__7372632f6d61696e2e77__7231__72656164")
            .nth(1)
            .expect("read");
        assert!(main.contains("store i32 41"));
        assert!(read.contains("load i32, ptr"));
    }

    #[test]
    fn initializes_namespaces_recursively_in_source_order_once() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\ni32 value = 7;\ni32() read = fn { value };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nfirst = namespace package \"src/child.w\";\ni32 observed = first.read();\nagain = namespace package \"src/child.w\";\ni32 after = 9;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source.clone()],
        );
        let text = emit_scalar_project_llvm(&validate_scalar_project(project))
            .expect("verified project LLVM")
            .to_text();
        let main = text.split("define i32 @main").nth(1).expect("main");
        let child_global = project_global_name(&child_source, "value");
        let child_store = main.find(&format!("store i32 7, ptr @{child_global}"));
        let read_call = main.find(&format!(
            "call i32 @{}",
            project_function_name(&child_source, "read")
        ));
        let observed_store = main.find("store i32 %call, ptr");
        assert_eq!(
            main.matches(&format!("store i32 7, ptr @{child_global}"))
                .count(),
            1
        );
        assert!(child_store.expect("child initialization") < read_call.expect("read call"));
        assert!(read_call.expect("read call") < observed_store.expect("observed store"));
    }

    #[test]
    fn releases_same_named_top_level_runtime_arrays_per_project_module() {
        let first_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/first.w".into(),
            "r1".into(),
        );
        let second_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/second.w".into(),
            "r1".into(),
        );
        let first_module_text = "%%start\nu64 length = 1;\nu8[length] bytes;\nbytes[0] = 7;\nu8() read = fn { bytes[0] };\n%%end";
        let second_module_text = "%%start\nu64 length = 1;\nu8[length] bytes;\nbytes[0] = 11;\nu8() read = fn { bytes[0] };\n%%end";
        let first = derive_scalar_program(
            &parse_source(first_source.clone(), first_module_text.into(), &[]).result,
        )
        .program;
        let second = derive_scalar_program(
            &parse_source(second_source.clone(), second_module_text.into(), &[]).result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nfirst = namespace package \"src/first.w\";\nsecond = namespace package \"src/second.w\";\nu8 first_value = first.read();\nu8 second_value = second.read();\nu8 first_expected = 7;\nu8 second_expected = 11;\nif (first_value != first_expected || second_value != second_expected) { core.system_panic(); };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: match namespace.binding.as_str() {
                        "first" => first_source.clone(),
                        "second" => second_source.clone(),
                        binding => panic!("unexpected namespace binding {binding}"),
                    },
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(first_source.clone(), first.items, Vec::new()),
                ScalarModule::new(second_source.clone(), second.items, Vec::new()),
            ],
            vec![root_source, first_source.clone(), second_source.clone()],
        );
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let mut project = project.clone();
            for module in &mut project.modules {
                module.target_layout = layout;
            }
            let validation = validate_scalar_project(project);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
            let text = emit_scalar_project_llvm(&validation)
                .expect("project runtime-array LLVM")
                .to_text();
            assert_eq!(
                text.matches("call ptr @__wosy_core_alloc").count(),
                2,
                "{text}"
            );
            assert_eq!(
                text.matches("call void @__wosy_core_free").count(),
                2,
                "{text}"
            );
            for (source, value) in [(&first_source, 7), (&second_source, 11)] {
                let global = project_global_name(source, "bytes");
                let read = project_function_name(source, "read");
                assert!(
                    text.lines().any(|line| {
                        line.contains("store ptr %") && line.contains(&format!("ptr @{global}"))
                    }),
                    "{text}"
                );
                let body = text
                    .split(&format!("define i8 @{read}"))
                    .nth(1)
                    .expect("read function")
                    .split("}")
                    .next()
                    .expect("read function body");
                let load = body
                    .find(&format!("load ptr, ptr @{global}"))
                    .expect("global pointer load");
                let gep = body.find("getelementptr inbounds i8").expect("indexed GEP");
                assert!(load < gep, "{body}");
                assert!(text.contains(&format!("store i8 {value}")), "{text}");
            }
        }
    }

    #[test]
    fn emits_ordered_top_level_assignment_and_global_function_load() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source.clone(),
            "%%start\ni32 value = 0;\nvalue = 1;\ni32() read = fn { value };\nvalue;\n%%end".into(),
            &[],
        );
        let validation = derive_scalar_program(&parsed.result);
        let text = emit_scalar_llvm(&validation)
            .expect("verified LLVM")
            .to_text();
        let main = text.split("define i32 @main").nth(1).expect("main");
        assert!(
            main.find("store i32 0").expect("initial store")
                < main.find("store i32 1").expect("assignment store")
        );
        assert!(text.contains("@value = internal global i32 0"));
        assert!(text.contains("define i32 @read"));
        assert!(text.contains("load i32, ptr @value"));

        let program = validation.program;
        let project = ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        );
        let project_text = emit_scalar_project_llvm(&validate_scalar_project(project))
            .expect("verified project LLVM")
            .to_text();
        let project_main = project_text
            .split("define i32 @main")
            .nth(1)
            .expect("project main");
        assert!(
            project_main
                .find("store i32 0")
                .expect("project initial store")
                < project_main
                    .find("store i32 1")
                    .expect("project assignment store")
        );
        assert!(project_text.contains("load i32, ptr @wosy_fn__70726f6a656374__7061636b616765__7372632f6d61696e2e77__7231__676c6f62616c5f76616c7565"));
    }

    #[test]
    fn encodes_colliding_source_paths_as_distinct_verified_symbols() {
        let first_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/a-b.w".into(),
            "r1".into(),
        );
        let second_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/a/b.w".into(),
            "r1".into(),
        );
        let first_program = derive_scalar_program(
            &parse_source(
                first_source.clone(),
                "%%start\ni32() same = fn { 1 };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let second_program = derive_scalar_program(
            &parse_source(
                second_source.clone(),
                "%%start\ni32() same = fn { 2 };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(first_source.clone(), first_program.items, Vec::new()),
                ScalarModule::new(second_source.clone(), second_program.items, Vec::new()),
            ],
            vec![first_source.clone(), second_source.clone()],
        );
        let text = emit_scalar_project_llvm(&validate_scalar_project(project))
            .expect("verified LLVM")
            .to_text();
        let first_name = project_function_name(&first_source, "same");
        let second_name = project_function_name(&second_source, "same");

        assert_ne!(first_name, second_name);
        assert!(text.contains(&format!("define i32 @{first_name}")));
        assert!(text.contains(&format!("define i32 @{second_name}")));
    }

    #[test]
    fn emits_qualified_member_load_and_store() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\ni32 value = 7;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nmath = namespace package \"src/child.w\";\ni32() read = fn { math.value };\ni32 result = read();\nmath.value = result + 1;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(root_source, root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![
                SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "src/main.w".into(),
                    "r1".into(),
                ),
                child_source.clone(),
            ],
        );
        let text = emit_scalar_project_llvm(&validate_scalar_project(project))
            .expect("qualified member LLVM")
            .to_text();
        let global = project_global_name(&child_source, "value");
        let main = text.split("define i32 @main").nth(1).expect("main");
        let temporary_store = main.find("store i32 %add, ptr %assignment_value");
        let member_store = main.rfind(&format!("ptr @{global}"));
        assert!(temporary_store.is_some(), "{main}");
        assert!(member_store.is_some(), "{main}");
        assert!(temporary_store < member_store, "{main}");
        let read = text
            .split(&format!(
                "define i32 @{}",
                project_function_name(
                    &SourceIdentity::new(
                        "project".into(),
                        "package".into(),
                        "src/main.w".into(),
                        "r1".into(),
                    ),
                    "read"
                )
            ))
            .nth(1)
            .expect("read");
        assert!(read.contains(&format!("load i32, ptr @{global}")));
    }

    #[test]
    fn verifies_qualified_member_project_module() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\ni32 value = 7;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nmath = namespace package \"src/child.w\";\ni32 result = math.value;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source],
        ));
        assert!(emit_scalar_project_llvm(&validation).is_ok());
    }

    #[test]
    fn emits_single_file_unit_bindings_as_effect_only() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start
unit() touch = fn { touch(); };
unit done = touch();
unit() use = fn {
    unit local = touch();
    local;
    local = touch();
    done;
    done = touch();
};
%%end"
                    .into(),
                &[],
            )
            .result,
        );
        let text = emit_scalar_llvm(&validation).expect("unit LLVM").to_text();

        assert_eq!(text.matches("call void @touch()").count(), 5);
        assert!(!text.contains("declare i32 @fd_write("));
        assert!(
            text.contains("source_filename = \"src/main.w\"\n\ndefine void @touch()"),
            "{text}"
        );
        assert!(!text.contains("global"), "{text}");
        assert!(!text.contains("alloca"));
        assert!(!text.contains("load"));
        assert!(!text.contains("store"));
    }

    #[test]
    fn emits_all_integer_widths_in_textual_llvm() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/integers.w".into(),
            "r1".into(),
        );
        let validation = crate::derive_scalar_program(
            &crate::parse_source(
                source,
                "%%start\ni8 a = 1;\ni16 b = 2;\ni32 c = 3;\ni64 d = 4;\ni128 e = 5;\nu8 f = 6;\nu16 g = 7;\nu32 h = 8;\nu64 i = 9;\nu128 j = 10;\n%%end".into(),
                &[],
            )
            .result,
        );
        let text = emit_scalar_llvm(&validation)
            .expect("integer LLVM")
            .to_text();
        for ty in ["i8", "i16", "i32", "i64", "i128"] {
            assert!(text.contains(&format!("global {ty}")), "{ty}: {text}");
        }
    }

    #[test]
    fn emits_single_file_top_level_unit_reads_and_assignments_as_effect_only() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start
unit() touch = fn { touch(); };
unit marker = touch();
marker;
marker = touch();
%%end"
                    .into(),
                &[],
            )
            .result,
        );
        let text = emit_scalar_llvm(&validation)
            .expect("unit top-level LLVM")
            .to_text();

        assert_eq!(text.matches("call void @touch()").count(), 3);
        assert!(!text.contains("@marker"));
        assert!(!text.contains("alloca"));
        assert!(!text.contains("load"));
        assert!(!text.contains("store"));
    }

    #[test]
    fn emits_project_unit_export_reads_and_assignments_as_effect_only() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start
unit() touch = fn { touch(); };
unit marker = touch();
%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start
child = namespace package \"src/child.w\";
unit() use = fn {
    unit local = child.marker;
    local;
    child.marker = child.touch();
};
unit observed = child.marker;
child.marker = child.touch();
%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("unit project LLVM")
            .to_text();

        let child_global = project_global_name(&child_source, "marker");
        assert_eq!(text.matches("call void @wosy_fn").count(), 4);
        assert!(!text.contains(&child_global));
        assert!(!text.contains("alloca"));
        assert!(!text.contains("load"));
        assert!(!text.contains("store"));
    }

    #[test]
    fn emits_short_circuit_cfg_for_all_boolean_paths() {
        let cases = [
            ("false_and", "false && right()", "false"),
            ("true_and", "true && right()", "true"),
            ("true_or", "true || right()", "true"),
            ("false_or", "false || right()", "false"),
        ];

        for (name, expression, left) in cases {
            let source = SourceIdentity::new(
                "project".into(),
                "package".into(),
                format!("src/{name}.w"),
                "r1".into(),
            );
            let validation = derive_scalar_program(
                &parse_source(
                    source,
                    format!(
                        "%%start\nbool() right = fn {{ true }};\nbool result = {expression};\n%%end"
                    ),
                    &[],
                )
                .result,
            );
            let text = emit_scalar_llvm(&validation)
                .expect("short-circuit LLVM")
                .to_text();
            let main = text.split("define i32 @main").nth(1).expect("main");
            assert_eq!(main.matches("call i1 @right()").count(), 1);
            assert!(main.contains("phi i1"));
            assert!(main.contains(&format!("br i1 {left}")));
            assert!(main.contains("short_circuit.rhs"));
            assert!(main.contains("short_circuit.merge"));
            assert!(
                main.find("br i1").expect("conditional branch")
                    < main.find("call i1 @right()").expect("RHS call")
            );
        }
    }

    #[test]
    fn emits_project_short_circuit_cfg_with_qualified_rhs() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nbool() right = fn { true };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\nbool result = false && child.right();\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_bindings = root
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source.clone()],
        ));
        let text = emit_scalar_project_llvm(&validation)
            .expect("project short-circuit LLVM")
            .to_text();
        assert!(text.contains("phi i1"));
        assert!(text.contains(&format!(
            "call i1 @{}",
            project_function_name(&child_source, "right")
        )));
    }

    #[test]
    fn emits_wasm32_struct_layout_literals_and_field_addresses() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\nPair item = { .second = 2; .first = 1; };\nunsafe { *?u8 address = &?item.first; };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("struct LLVM")
            .to_text();
        assert!(
            text.contains("@item = internal global [8 x i8] zeroinitializer"),
            "{text}"
        );
        assert!(text.contains("store i8 1"), "{text}");
        assert!(text.contains("store i32 2"), "{text}");
        assert!(
            text.contains("getelementptr inbounds i8, ptr %struct_literal, i8 4"),
            "{text}"
        );
        assert!(text.contains("ptrtoint"), "{text}");
    }

    #[test]
    fn emits_wasm32_pointer_struct_layout_for_wasi_iovec() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/wasi.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nstruct WasiIovec {\n\t*?u8 buf;\n\tu32 len;\n}\nWasiIovec iovec = { .buf = null; .len = 2; };\nunsafe { *?u32 len_address = &?iovec.len; };\n%%end".into(),
                &[],
            )
            .result,
        );

        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("WASI iovec LLVM")
            .to_text();
        assert!(
            text.contains("@iovec = internal global [8 x i8] zeroinitializer"),
            "{text}"
        );
        assert!(text.contains("store i32 0"), "{text}");
        assert!(
            text.contains("getelementptr inbounds i8, ptr %struct_literal, i8 4"),
            "{text}"
        );
    }

    #[test]
    fn emits_native64_pointer_struct_layout_literals_and_field_addresses() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source,
            "%%start\nstruct Pair {\n\t*?u8 address;\n\tu32 count;\n}\nPair item = { .address = null; .count = 2; };\nunsafe { *?u32 count_address = &?item.count; };\n%%end".into(),
            &[],
        );
        let validation =
            crate::derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);

        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("native struct LLVM")
            .to_text();
        assert!(
            text.contains("@item = internal global [16 x i8] zeroinitializer"),
            "{text}"
        );
        assert!(text.contains("store i64 0"), "{text}");
        assert!(
            text.contains("getelementptr inbounds i8, ptr %struct_literal, i8 8"),
            "{text}"
        );
        assert!(
            text.contains("ptrtoint (ptr getelementptr inbounds (i8, ptr @item, i8 8) to i64)"),
            "{text}"
        );
    }

    #[test]
    fn lowers_checked_reference_field_addresses_and_callable_transport_at_target_width() {
        for (layout, pointer_type, field_offset) in [
            (ScalarTargetLayout::WASM32, "i32", 4),
            (ScalarTargetLayout::NATIVE64, "i64", 8),
        ] {
            let source = SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/checked_references.w".into(),
                "r1".into(),
            );
            let parsed = parse_source(
                source,
                "%%start
struct Record {
    *?u8 bytes;
    u32 count;
}
*u32(*u32) transport = fn(value) { value };
*!u32(*!u32) transport_mutable = fn(value) { value };
Record item = { .bytes = null; .count = 2; };
*u32 shared = &item.count;
*!u32 mutable = &!item.count;
*Record record_shared = &item;
u32 shared_value = *shared;
u32 mutable_value = *mutable;
u32 field_value = (*record_shared).count;
*u32 forwarded = transport(shared);
*!u32 mutable_forwarded = transport_mutable(mutable);
u32 forwarded_value = *transport(shared);
u32 mutable_forwarded_value = *transport_mutable(mutable);
forwarded;
mutable_forwarded;
%%end"
                    .into(),
                &[],
            );
            let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );

            let text = emit_scalar_llvm(&validation)
                .expect("checked-reference LLVM")
                .to_text();

            assert!(
                text.contains(&format!(
                    "define {pointer_type} @transport({pointer_type} %value)"
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "define {pointer_type} @transport_mutable({pointer_type} %value)"
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!("call {pointer_type} @transport({pointer_type}")),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "call {pointer_type} @transport_mutable({pointer_type}"
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "getelementptr inbounds i8, ptr %struct_literal, i8 {field_offset}"
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "ptrtoint (ptr getelementptr inbounds (i8, ptr @item, i8 {field_offset}) to {pointer_type})"
                )),
                "{text}"
            );
            assert!(text.contains(&format!("inttoptr {pointer_type}")), "{text}");
            assert!(text.contains("load i32"), "{text}");
        }
    }

    #[test]
    fn emits_imported_callable_result_field_read_for_target_layouts() {
        for (layout, pointer_type, field_offset) in [
            (ScalarTargetLayout::WASM32, "i32", 4),
            (ScalarTargetLayout::NATIVE64, "i64", 4),
        ] {
            let child_source = SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/child.w".into(),
                "r1".into(),
            );
            let child = crate::derive_scalar_program_with_layout(
                &parse_source(
                    child_source.clone(),
                    "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n*Pair(*Pair) forward = fn(value) { value };\n%%end".into(),
                    &[],
                )
                .result,
                layout,
            )
            .program;
            let root_source = SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/main.w".into(),
                "r1".into(),
            );
            let root = crate::derive_scalar_program_with_layout(
                &parse_source(
                    root_source.clone(),
                    "%%start\nchild = namespace package \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\n*child.Pair pair = &item;\nu32 forwarded_read = (*child.forward(pair)).second;\n%%end".into(),
                    &[],
                )
                .result,
                layout,
            )
            .program;
            let namespace = match &root.items[0] {
                crate::ScalarItem::Namespace(namespace) => {
                    (namespace.binding.clone(), namespace.span)
                }
                _ => panic!("namespace item"),
            };
            let forward = project_function_name(&child_source, "forward");
            let validation = validate_scalar_project(ScalarProject::new(
                vec![
                    ScalarModule::from_program(
                        root,
                        vec![crate::ScalarNamespaceBinding {
                            binding: namespace.0,
                            target: child_source.clone(),
                            span: namespace.1,
                        }],
                    ),
                    ScalarModule::from_program(child, Vec::new()),
                ],
                vec![root_source, child_source],
            ));
            let text = emit_scalar_project_llvm(&validation)
                .expect("imported struct LLVM")
                .to_text();
            assert!(text.contains("[8 x i8]"), "{text}");
            assert!(
                text.contains(&format!(
                    "getelementptr inbounds i8, ptr %deref, i8 {field_offset}"
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!("call {pointer_type} @{forward}({pointer_type}")),
                "{text}"
            );
            assert!(text.contains(&format!("inttoptr {pointer_type}")), "{text}");
        }
    }

    #[test]
    fn emits_direct_local_struct_addresses_and_loaded_pointer_bindings_for_externs() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct Outer {\n\tPair pair;\n}\nstruct Pair {\n\tu32 value;\n\tu8 tag;\n}\nraw = extern wasm \"env\" { unit(*?Pair) touch; unit(*?u8) inspect; };\nunit(Outer) forward = fn(value) {\n\tOuter local = { .pair = { .value = 2; .tag = 1; }; };\n\tlocal;\n\tunsafe {\n\t\t*?Outer outer = &?value;\n\t\t*?Pair pointer = &?(*outer).pair;\n\t\traw.touch(pointer);\n\t\t*?u8 field = &?(*pointer).tag;\n\t\traw.inspect(field);\n\t};\n};\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("raw struct pointer LLVM")
            .to_text();
        let forward = text
            .split(&format!(
                "define void @{}",
                project_function_name(&source, "forward")
            ))
            .nth(1)
            .expect("forward");

        assert!(text.contains("declare void @wosy_extern__"), "{text}");
        assert!(text.contains("wasm-import-name\"=\"touch"), "{text}");
        assert!(text.contains("wasm-import-name\"=\"inspect"), "{text}");
        assert!(forward.contains("alloca [8 x i8]"), "{forward}");
        assert!(forward.contains("store [8 x i8]"), "{forward}");
        assert!(forward.contains("%local = alloca [8 x i8]"), "{forward}");
        assert!(
            forward.contains("store [8 x i8] %struct_value") && forward.contains("ptr %local"),
            "{forward}"
        );
        assert!(forward.contains("ptrtoint ptr %pair"), "{forward}");
        assert!(
            forward.contains("getelementptr inbounds i8, ptr %deref") && forward.contains("i8 4"),
            "{forward}"
        );
        assert!(forward.contains("load i32, ptr %pointer"), "{forward}");
        assert!(forward.contains("call void @wosy_extern__"), "{forward}");
        assert!(forward.contains("load i32, ptr %field"), "{forward}");
        assert!(forward.contains("call void @wosy_extern__"), "{forward}");
    }

    #[test]
    fn emits_project_struct_storage_address_for_generic_extern() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct Outer {\n\tPair pair;\n}\nstruct Pair {\n\tu8 tag;\n\tu32 value;\n}\nraw = extern wasm \"env\" { unit(*?Pair) touch; };\nOuter value = { .pair = { .tag = 1; .value = 2; }; };\nunsafe { *?Pair pointer = &?value.pair; raw.touch(pointer); };\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("project raw struct pointer LLVM")
            .to_text();
        let main = text.split("define i32 @main").nth(1).expect("main");
        let value = project_global_name(&source, "value");

        assert!(text.contains("declare void @wosy_extern__"), "{text}");
        assert!(text.contains("wasm-import-name\"=\"touch"), "{text}");
        assert!(
            text.contains(&format!("@{value} = internal global [8 x i8]")),
            "{text}"
        );
        assert!(
            main.contains(&format!("ptrtoint (ptr @{value} to i32)")),
            "{main}"
        );
        assert!(main.contains("load i32, ptr %pointer"), "{main}");
        assert!(main.contains("call void @wosy_extern__"), "{main}");
    }

    #[test]
    fn emits_utf8_literal_as_private_struct_storage() {
        let std_source = SourceIdentity::new(
            "project".into(),
            "stdlib".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let std = derive_scalar_program(
            &parse_source(
                std_source.clone(),
                "%%start
struct utf8 {
    *?u8 data;
    u64 length;
}
%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start
text = namespace stdlib \"src/bootstrap.w\";
text.utf8 value = \"hé\";
%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    root,
                    vec![crate::ScalarNamespaceBinding {
                        binding: "text".into(),
                        target: std_source.clone(),
                        span: wosy_syntax::ByteSpan::new(0, 0),
                    }],
                ),
                ScalarModule::from_program(std, Vec::new()),
            ],
            vec![source, std_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("UTF-8 LLVM")
            .to_text();
        assert!(
            text.contains("@wosy_utf8_literal_0 = private constant [3 x i8]"),
            "{text}"
        );
        assert!(text.contains("store i64 3"), "{text}");
        assert!(text.contains("i8 0"), "{text}");
    }

    #[test]
    fn emits_fixed_array_callable_transport_with_typed_address_storage() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nu8[2](u8[2]) transport = fn(bytes) { bytes };\nunit(u8) observe = fn(value) { value; };\nunit() run = fn { u8[2] bytes = [7, 9]; transport(bytes); observe(1); };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_llvm(&validation)
            .expect("fixed array callable LLVM")
            .to_text();
        let transport = text
            .split("define ptr @transport(ptr %bytes)")
            .nth(1)
            .expect("transport function");
        let run = text
            .split("define void @run()")
            .nth(1)
            .expect("run function");

        assert!(text.contains("[2 x i8]"), "{text}");
        assert!(run.contains("%array_literal = alloca [2 x i8]"), "{run}");
        let first = run
            .find("store i8 7, ptr %array_element")
            .expect("first element");
        let second = run
            .find("store i8 9, ptr %array_element")
            .expect("second element");
        assert!(first < second, "{run}");
        assert!(run.contains("%bytes = alloca [2 x i8]"), "{run}");
        assert!(run.contains("call ptr @transport(ptr %bytes)"), "{run}");
        assert!(transport.contains("ret ptr %bytes"), "{transport}");
    }

    #[test]
    fn emits_valid_nested_struct_array_storage_with_final_layout_offsets() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start
struct Leaf {
    u16 value;
}
struct Grid {
    Leaf[2][2] leaves;
    u8 marker;
}
raw = extern wasm \"env\" { unit(*?u8) inspect; };
unit(Grid) store_grid = fn(value) {
    value;
};
unit(*?Grid) marker_address = fn(pointer) {
    unsafe {
        *?u8 marker = &?(*pointer).marker;
        raw.inspect(marker);
    };
};
%%end"
                    .into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let leaf = validation
            .program
            .structs
            .iter()
            .find(|structure| structure.name == "Leaf")
            .expect("Leaf layout");
        let grid = validation
            .program
            .structs
            .iter()
            .find(|structure| structure.name == "Grid")
            .expect("Grid layout");

        assert_eq!(
            leaf.layout
                .as_ref()
                .map(|layout| (layout.size, layout.alignment)),
            Some((2, 2))
        );
        assert_eq!(leaf.fields[0].offset, Some(0));
        assert_eq!(
            grid.layout
                .as_ref()
                .map(|layout| (layout.size, layout.alignment)),
            Some((10, 2))
        );
        assert_eq!(grid.fields[0].offset, Some(0));
        assert_eq!(grid.fields[1].offset, Some(8));

        let text = emit_scalar_llvm(&validation)
            .expect("nested aggregate LLVM")
            .to_text();

        assert!(text.contains("alloca [10 x i8]"), "{text}");
        assert!(
            text.contains("getelementptr inbounds i8, ptr %deref, i8 8"),
            "{text}"
        );
    }
}
