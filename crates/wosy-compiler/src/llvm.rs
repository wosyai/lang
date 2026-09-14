use std::collections::BTreeMap;
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

use crate::scalar::{ScalarBindingOutputOrigin, ScalarFieldReference};
use crate::{
    BinaryOperator, ScalarAssignment, ScalarBlock, ScalarBlockItem, ScalarExpression,
    ScalarFunction, ScalarItem, ScalarModule, ScalarProjectValidation, ScalarStruct, ScalarType,
    ScalarValidation, ScalarWhile,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LlvmValueType {
    Void,
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
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    all_globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    structs: &'module [ScalarStruct],
    next_literal: usize,
    next_block: usize,
}

struct LlvmExternIdentity {
    internal_name: String,
    import_module: String,
    import_name: String,
    signature: ScalarType,
}

pub fn emit_scalar_llvm(validation: &ScalarValidation) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".into());
    }
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&validation.program.source.path));
    let builder = context.create_builder();
    let mut globals = BTreeMap::new();
    for item in &validation.program.items {
        if let ScalarItem::Binding(binding) = item {
            for (name, ty) in binding_receivers(binding).iter() {
                if *ty == ScalarType::Unit {
                    continue;
                }
                let llvm_ty = storage_type(&context, ty, &validation.program.structs)?;
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
    for item in &validation.program.items {
        if let ScalarItem::Function(function) = item {
            let value = module.add_function(
                &function.name,
                function_type(&context, &function.signature)?,
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
                    function_type(&context, &function.signature)?,
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
    for item in &validation.program.items {
        if let ScalarItem::Function(function) = item {
            emit_function(
                &context,
                &builder,
                &functions,
                &call_targets,
                &signatures,
                &globals,
                function,
                &function.name,
                &validation.program.items,
                &module,
                &validation.program.structs,
            )?;
        }
    }
    emit_main(
        &context,
        &builder,
        &functions,
        &call_targets,
        &signatures,
        &globals,
        &validation.program.items,
        &module,
        &validation.program.structs,
    )?;
    finish_partition(
        ManuallyDrop::into_inner(module),
        validation.program.source.path.clone(),
        &functions,
        &signatures,
        &externs,
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
                    let llvm_ty = storage_type(&context, ty, &all_structs)?;
                    let global = module.add_global(llvm_ty, None, &name);
                    global.set_linkage(Linkage::Internal);
                    global.set_initializer(&llvm_ty.const_zero());
                    globals.insert(name, (global, ty.clone()));
                }
            }
            if let ScalarItem::Function(function) = item {
                let name = project_function_name(&source_module.source, &function.name);
                let value =
                    module.add_function(&name, function_type(&context, &function.signature)?, None);
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
                        function_type(&context, &function.signature)?,
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
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    for (source_module, function, name) in definitions {
        emit_project_function(
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            function,
            source_module,
            &modules,
            &globals,
            &name,
            &module,
            &all_structs,
        )?;
    }
    emit_project_main(
        &context,
        &builder,
        &functions,
        &call_targets,
        &signatures,
        &modules,
        &globals,
        &module,
        &all_structs,
    )?;
    finish_partition(
        ManuallyDrop::into_inner(module),
        module_name,
        &functions,
        &signatures,
        &externs,
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
                .filter(|line| !line.starts_with("declare "))
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
    functions: &BTreeMap<String, FunctionValue<'ctx>>,
    signatures: &BTreeMap<String, ScalarType>,
    externs: &[LlvmExternIdentity],
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
            let ScalarType::Callable {
                outputs,
                parameters,
            } = signature
            else {
                return None;
            };
            let result = callable_result(&outputs);
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
                result: value_type(&result).ok()?,
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
                outputs,
                parameters,
            } = &extern_identity.signature
            else {
                return None;
            };
            let result = callable_result(outputs);
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
                result: value_type(&result).ok()?,
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
) -> Result<FunctionType<'ctx>, String> {
    let ScalarType::Callable {
        outputs,
        parameters,
    } = signature
    else {
        return Err("function has no callable signature".into());
    };
    let parameters = parameters
        .iter()
        .map(|ty| basic_type(context, ty).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    match outputs.outputs.as_slice() {
        [] => Ok(context.void_type().fn_type(&parameters, false)),
        [output] if output.ty == ScalarType::Unit => {
            Ok(context.void_type().fn_type(&parameters, false))
        }
        [output] => Ok(basic_type(context, &output.ty)?.fn_type(&parameters, false)),
        _ => Ok(aggregate_type(context, outputs)?.fn_type(&parameters, false)),
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

fn basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Bool => Ok(context.bool_type().into()),
        ty if integer_width(ty).is_some() => Ok(integer_type(context, ty)?.into()),
        ScalarType::Char => Ok(context.i32_type().into()),
        ScalarType::F32 => Ok(context.f32_type().into()),
        ScalarType::F64 => Ok(context.f64_type().into()),
        ScalarType::ArtifactId => Ok(context.ptr_type(AddressSpace::default()).into()),
        ScalarType::RawPointer(_) => Ok(context.i32_type().into()),
        ScalarType::Struct(_) => Ok(context.ptr_type(AddressSpace::default()).into()),
        _ => Err("unit is only valid as a function result".into()),
    }
}

fn storage_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
    structs: &[ScalarStruct],
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Struct(id) => {
            let structure = structs
                .iter()
                .find(|structure| structure.id == *id)
                .ok_or_else(|| format!("unknown LLVM struct {}", id.index))?;
            let size = u32::try_from(structure.layout.size)
                .map_err(|_| "struct layout exceeds LLVM array size")?;
            Ok(context.i8_type().array_type(size).into())
        }
        _ => basic_type(context, ty),
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
        ScalarType::RawPointer(_) => Ok(LlvmValueType::Pointer),
        ScalarType::Struct(_) => Ok(LlvmValueType::Pointer),
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
) -> Result<StructType<'ctx>, String> {
    let fields = outputs
        .outputs
        .iter()
        .map(|output| output_basic_type(context, &output.ty))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(context.struct_type(&fields, false))
}

fn output_basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<BasicTypeEnum<'ctx>, String> {
    if *ty == ScalarType::Unit {
        Ok(context.i8_type().into())
    } else {
        basic_type(context, ty)
    }
}

fn callable_result(outputs: &crate::ScalarOutputSequence) -> ScalarType {
    outputs
        .outputs
        .first()
        .map_or(ScalarType::Unit, |output| output.ty.clone())
}

fn builder_error(error: BuilderError) -> String {
    error.to_string()
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
) -> Result<PointerValue<'ctx>, String> {
    match place {
        crate::ScalarPlace::Name { name, .. } => {
            if let Some((slot, _)) = state.storage.get(name).cloned() {
                return Ok(slot);
            }
            state
                .globals
                .get(name)
                .map(|(global, _)| global.as_pointer_value())
                .ok_or_else(|| format!("unknown LLVM place {name}"))
        }
        crate::ScalarPlace::Dereference { pointer, .. } => {
            let pointer = take_basic(emit_expression(context, state, pointer)?)?.into_int_value();
            state
                .builder
                .build_int_to_ptr(pointer, context.ptr_type(AddressSpace::default()), "deref")
                .map_err(builder_error)
        }
        crate::ScalarPlace::Field { base, field, .. } => {
            let base = place_pointer(context, state, base)?;
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
            let offset = byte_type.const_int(resolved.offset, false);
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
    }
}

fn emit_place_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
) -> Result<EmitValue<'ctx>, String> {
    let pointer = place_pointer(context, state, place)?;
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
        crate::ScalarPlace::Dereference {
            pointer: expression,
            ..
        } => {
            let value = take_basic(emit_expression(context, state, expression)?)?.into_int_value();
            let pointer = state
                .builder
                .build_int_to_ptr(value, context.ptr_type(AddressSpace::default()), "deref")
                .map_err(builder_error)?;
            return Ok(EmitValue::Basic(
                state
                    .builder
                    .build_load(context.i32_type(), pointer, "deref")
                    .map_err(builder_error)?,
            ));
        }
    };
    if matches!(ty, ScalarType::Struct(_)) {
        return Ok(EmitValue::Basic(pointer.into()));
    }
    Ok(EmitValue::Basic(
        state
            .builder
            .build_load(basic_type(context, &ty)?, pointer, "place")
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
    let storage = context.i8_type().array_type(
        u32::try_from(structure.layout.size)
            .map_err(|_| "struct layout exceeds LLVM array size")?,
    );
    let destination = state
        .builder
        .build_alloca(storage, "struct_literal")
        .map_err(builder_error)?;
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
                &[context.i8_type().const_int(field.offset, false)],
                field.name.as_str(),
            )
        }
        .map_err(builder_error)?;
        store_value(context, state, pointer, &field.ty, value)?;
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
    let destination = state
        .builder
        .build_alloca(
            storage_type(context, expected, state.structs)?,
            "utf8_literal",
        )
        .map_err(builder_error)?;
    let data = unsafe {
        state.builder.build_in_bounds_gep(
            byte_type,
            destination,
            &[byte_type.const_int(structure.fields[0].offset, false)],
            "data",
        )
    }
    .map_err(builder_error)?;
    let address = state
        .builder
        .build_ptr_to_int(
            literal.as_pointer_value(),
            context.i32_type(),
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
            &[byte_type.const_int(structure.fields[1].offset, false)],
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
    if let ScalarType::Struct(id) = ty {
        let structure = structure(state.structs, id.clone())?;
        let source = take_basic(value)?.into_pointer_value();
        let aggregate = context.i8_type().array_type(
            u32::try_from(structure.layout.size)
                .map_err(|_| "struct layout exceeds LLVM array size")?,
        );
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
    let ty = aggregate_type(context, outputs)?;
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
    let receivers = binding_receivers(binding);
    for (position, (name, ty)) in receivers.iter().enumerate() {
        let value = extract_output(state, value.clone(), position)?;
        if *ty == ScalarType::Unit {
            state.values.insert(name.clone(), EmitValue::Unit);
            continue;
        }
        let slot = state
            .builder
            .build_alloca(storage_type(context, ty, state.structs)?, name)
            .map_err(builder_error)?;
        store_value(context, state, slot, ty, value)?;
        state.storage.insert(name.clone(), (slot, ty.clone()));
    }
    Ok(())
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
    globals: &'ctx BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    function: &ScalarFunction,
    name: &str,
    items: &[ScalarItem],
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
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
                    storage_type(context, parameter, structs)?,
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
                emit_expression(context, &mut state, expression)?
            }
        }
        _ => emit_block(context, &mut state, &function.body)?,
    };
    emit_return(&mut state, result, outputs)
}

fn emit_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    globals: &'ctx BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    items: &[ScalarItem],
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, items);
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                let value = emit_binding_value(context, &mut state, binding)?;
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
                }
            }
            ScalarItem::Executable(item) => {
                emit_block_item(context, &mut state, item)?;
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Function(_) => {}
        }
    }
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
    function: &ScalarFunction,
    source_module: &ScalarModule,
    modules: &[&'module ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    name: &str,
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: module_globals(source_module, globals),
        all_globals: globals.clone(),
        structs,
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
                    storage_type(context, parameter, structs)?,
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
                emit_project_expression(context, &mut state, expression, source_module, modules)?
            }
        }
        _ => emit_project_block(context, &mut state, &function.body, source_module, modules)?,
    };
    emit_return(&mut state, result, outputs)
}

fn emit_project_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    modules: &[&ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    module: &'module Module<'ctx>,
    structs: &'module [ScalarStruct],
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: BTreeMap::new(),
        all_globals: globals.clone(),
        structs,
        next_literal: 0,
        next_block: 0,
    };
    let root = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?;
    initialize_project_module(context, &mut state, root, modules, globals, &mut Vec::new())?;
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
    let current_module_globals = module_globals(module, globals);
    let current_globals = std::mem::replace(&mut state.globals, current_module_globals);
    state.values.clear();
    state.storage.clear();
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
    state.storage = storage;
    Ok(())
}

fn emit_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let values = state.values.clone();
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
    state.storage = storage;
    state.values = values;
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
    state.storage = storage;
    state.values = values;
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
            return emit_expression(context, state, &output.value);
        }
        return emit_typed_expression(context, state, &output.value, &output.ty);
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
            return emit_project_expression(context, state, &output.value, module, modules);
        }
        return emit_project_typed_expression(
            context,
            state,
            &output.value,
            &output.ty,
            module,
            modules,
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
    build_aggregate(context, state, &outputs, values)
}

fn emit_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
) -> Result<EmitValue<'ctx>, String> {
    let expected = state
        .storage
        .get(&assignment.target)
        .map(|(_, ty)| ty.clone())
        .or_else(|| {
            state
                .globals
                .get(&assignment.target)
                .map(|(_, ty)| ty.clone())
        });
    let value = match expected {
        Some(ty) => emit_typed_expression(context, state, &assignment.value, &ty)?,
        None => emit_expression(context, state, &assignment.value)?,
    };
    let old = if assignment.receiver.is_some() {
        return Err(format!(
            "unknown LLVM storage {}.{}",
            assignment.receiver.as_deref().expect("assignment receiver"),
            assignment.target
        ));
    } else if let Some((slot, ty)) = state.storage.get(&assignment.target).cloned() {
        let value = take_basic(value)?;
        let old = state
            .builder
            .build_load(storage_type(context, &ty, state.structs)?, slot, "old")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(slot, value)
            .map_err(builder_error)?;
        old
    } else if let Some((global, ty)) = state.globals.get(&assignment.target).cloned() {
        let value = take_basic(value)?;
        let slot = global.as_pointer_value();
        let old = state
            .builder
            .build_load(storage_type(context, &ty, state.structs)?, slot, "old")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(slot, value)
            .map_err(builder_error)?;
        old
    } else {
        match state.values.get(&assignment.target) {
            Some(EmitValue::Unit) => return Ok(EmitValue::Unit),
            Some(EmitValue::Basic(_)) => {
                return Err(format!("unknown LLVM storage {}", assignment.target))
            }
            Some(EmitValue::Aggregate { .. }) => {
                return Err(format!("unknown LLVM storage {}", assignment.target))
            }
            None => return Err(format!("unknown LLVM storage {}", assignment.target)),
        }
    };
    Ok(EmitValue::Basic(old))
}
fn emit_project_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let value = emit_project_expression(context, state, &assignment.value, module, modules)?;
    let old = if let Some(receiver) = &assignment.receiver {
        let target = project_namespace_target(module, modules, receiver)?;
        let (global, ty) = project_member_global(state, target, &assignment.target)?;
        if ty == ScalarType::Unit {
            return Ok(EmitValue::Unit);
        }
        let value = take_basic(value)?;
        let global =
            global.ok_or_else(|| format!("unknown LLVM member storage {}", assignment.target))?;
        let slot = global.as_pointer_value();
        let old = state
            .builder
            .build_load(storage_type(context, &ty, state.structs)?, slot, "old")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(slot, value)
            .map_err(builder_error)?;
        old
    } else if let Some((slot, ty)) = state.storage.get(&assignment.target).cloned() {
        let value = take_basic(value)?;
        let old = state
            .builder
            .build_load(storage_type(context, &ty, state.structs)?, slot, "old")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(slot, value)
            .map_err(builder_error)?;
        old
    } else if let Some((global, ty)) = state.globals.get(&assignment.target).cloned() {
        let value = take_basic(value)?;
        let slot = global.as_pointer_value();
        let old = state
            .builder
            .build_load(storage_type(context, &ty, state.structs)?, slot, "old")
            .map_err(builder_error)?;
        state
            .builder
            .build_store(slot, value)
            .map_err(builder_error)?;
        old
    } else {
        match state.values.get(&assignment.target) {
            Some(EmitValue::Unit) => return Ok(EmitValue::Unit),
            Some(EmitValue::Basic(_)) => {
                return Err(format!("unknown LLVM storage {}", assignment.target))
            }
            Some(EmitValue::Aggregate { .. }) => {
                return Err(format!("unknown LLVM storage {}", assignment.target))
            }
            None => return Err(format!("unknown LLVM storage {}", assignment.target)),
        }
    };
    Ok(EmitValue::Basic(old))
}

fn emit_block_item<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    item: &ScalarBlockItem,
) -> Result<EmitValue<'ctx>, String> {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let value =
                emit_typed_expression(context, state, &binding.value, &binding.declared_type)?;
            if binding.declared_type == ScalarType::Unit {
                state.values.insert(binding.name.clone(), EmitValue::Unit);
                return Ok(EmitValue::Unit);
            }
            let slot = state
                .builder
                .build_alloca(
                    storage_type(context, &binding.declared_type, state.structs)?,
                    &binding.name,
                )
                .map_err(builder_error)?;
            store_value(context, state, slot, &binding.declared_type, value)?;
            state
                .storage
                .insert(binding.name.clone(), (slot, binding.declared_type.clone()));
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
            let value = emit_project_typed_expression(
                context,
                state,
                &binding.value,
                &binding.declared_type,
                module,
                modules,
            )?;
            if binding.declared_type == ScalarType::Unit {
                state.values.insert(binding.name.clone(), EmitValue::Unit);
                return Ok(EmitValue::Unit);
            }
            let slot = state
                .builder
                .build_alloca(
                    storage_type(context, &binding.declared_type, state.structs)?,
                    &binding.name,
                )
                .map_err(builder_error)?;
            store_value(context, state, slot, &binding.declared_type, value)?;
            state
                .storage
                .insert(binding.name.clone(), (slot, binding.declared_type.clone()));
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
                return Ok(EmitValue::Basic(context.i32_type().const_zero().into()));
            }
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                if matches!(ty, ScalarType::Struct(_)) {
                    return Ok(EmitValue::Basic(slot.into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(storage_type(context, &ty, state.structs)?, slot, name)
                        .map_err(builder_error)?,
                ));
            }
            if let Some((global, ty)) = state.globals.get(name).cloned() {
                if matches!(ty, ScalarType::Struct(_)) {
                    return Ok(EmitValue::Basic(global.as_pointer_value().into()));
                }
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(
                            storage_type(context, &ty, state.structs)?,
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
        ScalarExpression::Member { receiver, name, .. } => {
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            Err(format!("unknown LLVM member {receiver}.{name}"))
        }
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
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "cast" {
                return emit_cast(context, state, type_arguments, arguments);
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return emit_int_conversion(context, state, name, type_arguments, arguments);
            }
            emit_call(context, state, receiver.as_deref(), name, arguments)
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
        ScalarExpression::RawAddress { place, .. } => Ok(EmitValue::Basic(
            state
                .builder
                .build_ptr_to_int(
                    place_pointer(context, state, place)?,
                    context.i32_type(),
                    "address",
                )
                .map_err(builder_error)?
                .into(),
        )),
        ScalarExpression::StructLiteral { .. } => {
            return Err("struct literal requires an expected struct type".to_owned())
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
        ScalarExpression::Member { receiver, name, .. } => {
            if let Some(place) = struct_member_place(state, receiver, name)? {
                return emit_place_value(context, state, &place);
            }
            let target = project_namespace_target(module, modules, receiver)?;
            let (global, ty) = project_member_global(state, target, name)?;
            if ty == ScalarType::Unit {
                return Ok(EmitValue::Unit);
            }
            let global = global.ok_or_else(|| format!("unknown LLVM member storage {name}"))?;
            if matches!(ty, ScalarType::Struct(_)) {
                return Ok(EmitValue::Basic(global.as_pointer_value().into()));
            }
            Ok(EmitValue::Basic(
                state
                    .builder
                    .build_load(
                        storage_type(context, &ty, state.structs)?,
                        global.as_pointer_value(),
                        name,
                    )
                    .map_err(builder_error)?,
            ))
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
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
            let target = state
                .call_targets
                .get(&qualified)
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
        ScalarExpression::RawAddress { place, .. } => Ok(EmitValue::Basic(
            state
                .builder
                .build_ptr_to_int(
                    place_pointer(context, state, place)?,
                    context.i32_type(),
                    "address",
                )
                .map_err(builder_error)?
                .into(),
        )),
        ScalarExpression::StructLiteral { .. } => {
            return Err("struct literal requires an expected struct type".to_owned())
        }
    }
}

fn emit_call<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    receiver: Option<&str>,
    name: &str,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let qualified =
        receiver.map_or_else(|| name.to_owned(), |receiver| format!("{receiver}.{name}"));
    let target = state
        .call_targets
        .get(&qualified)
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
    let (source, destination) = match operation {
        "int_trunc" if type_argument.ty == ScalarType::U32 => (ScalarType::U64, ScalarType::U32),
        "int_extend" if type_argument.ty == ScalarType::U64 => (ScalarType::U32, ScalarType::U64),
        _ => {
            return Err(format!(
                "core.{operation} has an invalid integer conversion"
            ))
        }
    };
    let value =
        take_basic(emit_typed_expression(context, state, value, &source)?)?.into_int_value();
    let converted = if operation == "int_trunc" {
        state
            .builder
            .build_int_truncate(value, integer_type(context, &destination)?, "int_trunc")
    } else {
        state
            .builder
            .build_int_z_extend(value, integer_type(context, &destination)?, "int_extend")
    }
    .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted.into()))
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
    let (source, destination) = match operation {
        "int_trunc" if type_argument.ty == ScalarType::U32 => (ScalarType::U64, ScalarType::U32),
        "int_extend" if type_argument.ty == ScalarType::U64 => (ScalarType::U32, ScalarType::U64),
        _ => {
            return Err(format!(
                "core.{operation} has an invalid integer conversion"
            ))
        }
    };
    let value = take_basic(emit_project_typed_expression(
        context, state, value, &source, module, modules,
    )?)?
    .into_int_value();
    let converted = if operation == "int_trunc" {
        state
            .builder
            .build_int_truncate(value, integer_type(context, &destination)?, "int_trunc")
    } else {
        state
            .builder
            .build_int_z_extend(value, integer_type(context, &destination)?, "int_extend")
    }
    .map_err(builder_error)?;
    Ok(EmitValue::Basic(converted.into()))
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
        _ => Err("conditional branches have different LLVM values".into()),
    }
}

fn emit_return<'ctx, 'module>(
    state: &mut EmitState<'ctx, 'module>,
    value: EmitValue<'ctx>,
    outputs: &crate::ScalarOutputSequence,
) -> Result<(), String> {
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
        import_name: import_name.to_owned(),
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

fn project_global_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    project_function_name(source, &format!("global_{name}"))
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
    use super::emit_scalar_llvm;
    use super::emit_scalar_project_llvm;
    use super::project_function_name;
    use super::project_global_name;
    use super::{LlvmFunction, LlvmFunctionAttributes, LlvmPartition, LlvmValueType};
    use crate::{
        derive_scalar_program, parse_source, validate_scalar_project, ScalarModule, ScalarProject,
    };
    use wosy_syntax::SourceIdentity;

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
    fn emits_integer_conversions_for_single_file_and_project() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nu64 source = 42;\nu32 narrow = core.int_trunc<u32>(source);\nu64 wide = core.int_extend<u64>(narrow);\n%%end".into(),
                &[],
            )
            .result,
        );
        let single = emit_scalar_llvm(&program)
            .expect("single-file conversion LLVM")
            .to_text();
        assert!(single.contains("trunc i64"), "{single}");
        assert!(single.contains("zext i32"), "{single}");

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                source.clone(),
                program.program.items,
                Vec::new(),
            )],
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
        assert!(project.contains("trunc i64"), "{project}");
        assert!(project.contains("zext i32"), "{project}");
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
        assert!(main.contains(&format!("store i32 %add, ptr @{global}")));
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
    fn emits_imported_struct_layout_and_field_address() {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n%%end".into(),
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
                "%%start\nchild = namespace package \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\nunsafe { *?child.Pair pointer = &?item; *?u32 address = &?(*pointer).second; };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace = match &root.items[0] {
            crate::ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
            _ => panic!("namespace item"),
        };
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
        assert!(text.contains("getelementptr inbounds i8"), "{text}");
        assert!(text.contains("i8 4"), "{text}");
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
}
