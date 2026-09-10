use std::collections::BTreeMap;
use std::fmt::Write;
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;

use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, GlobalValue, PointerValue, ValueKind,
};
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use serde::{Deserialize, Serialize};

use crate::scalar::ScalarFieldReference;
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
    Utf8View {
        pointer: BasicValueEnum<'ctx>,
        length: BasicValueEnum<'ctx>,
    },
}

struct EmitState<'ctx, 'module> {
    builder: &'ctx Builder<'ctx>,
    module: &'module Module<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    all_globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    structs: &'module [ScalarStruct],
    utf8_lengths: BTreeMap<String, u64>,
    next_literal: usize,
    next_block: usize,
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
            if binding.declared_type == ScalarType::Unit {
                continue;
            }
            let ty = storage_type(
                &context,
                &binding.declared_type,
                &validation.program.structs,
            )?;
            let global = module.add_global(ty, None, &binding.name);
            global.set_linkage(Linkage::Internal);
            global.set_initializer(&ty.const_zero());
            globals.insert(
                binding.name.clone(),
                (global, binding.declared_type.clone()),
            );
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
        }
    }
    for item in &validation.program.items {
        if let ScalarItem::Extern(extern_decl) = item {
            for function in &extern_decl.functions {
                let key = format!("{}.{}", extern_decl.binding, function.name);
                let value = module.add_function(
                    &function.name,
                    function_type(&context, &function.signature)?,
                    None,
                );
                functions.insert(key.clone(), value);
                signatures.insert(key, function.signature.clone());
                if let crate::scalar::ScalarExternModule::Valid(module_name) =
                    &extern_decl.actual_module
                {
                    externs.push((
                        format!("{}.{}", extern_decl.binding, function.name),
                        module_name.clone(),
                        function.signature.clone(),
                    ));
                }
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
    let mut definitions = Vec::new();
    for source_module in &modules {
        for item in &source_module.items {
            if let ScalarItem::Binding(binding) = item {
                if binding.declared_type == ScalarType::Unit {
                    continue;
                }
                let name = project_global_name(&source_module.source, &binding.name);
                let ty = storage_type(&context, &binding.declared_type, &all_structs)?;
                let global = module.add_global(ty, None, &name);
                global.set_linkage(Linkage::Internal);
                global.set_initializer(&ty.const_zero());
                globals.insert(name, (global, binding.declared_type.clone()));
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
                signatures.insert(name.clone(), function.signature.clone());
                definitions.push((source_module, function, name));
            }
            if let ScalarItem::Extern(extern_decl) = item {
                let module_name = match &extern_decl.actual_module {
                    crate::scalar::ScalarExternModule::Valid(name) => name,
                    crate::scalar::ScalarExternModule::Invalid { .. } => continue,
                };
                for function in &extern_decl.functions {
                    let name = project_function_name(&source_module.source, &function.name);
                    let value = module.add_function(
                        &function.name,
                        function_type(&context, &function.signature)?,
                        None,
                    );
                    functions.insert(name.clone(), value);
                    signatures.insert(name, function.signature.clone());
                    externs.push((
                        project_function_name(&source_module.source, &function.name),
                        module_name.clone(),
                        function.signature.clone(),
                    ));
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
                writeln!(
                    serialized,
                    "declare {} @{}({}) #{}",
                    llvm_text_type(declaration.result),
                    declaration.name,
                    declaration
                        .parameters
                        .iter()
                        .map(|(_, ty)| llvm_text_type(*ty))
                        .collect::<Vec<_>>()
                        .join(", "),
                    declaration.attributes[0].group,
                )
                .expect("writing to a String cannot fail");
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

fn llvm_text_type(ty: LlvmValueType) -> &'static str {
    match ty {
        LlvmValueType::Void => "void",
        LlvmValueType::I1 => "i1",
        LlvmValueType::I16 => "i16",
        LlvmValueType::I8 => "i8",
        LlvmValueType::I32 => "i32",
        LlvmValueType::I64 => "i64",
        LlvmValueType::I128 => "i128",
        LlvmValueType::Pointer => "ptr",
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
    externs: &[(String, String, ScalarType)],
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
            let result = callable_result(&outputs).ok()?;
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
                result: value_type(result).ok()?,
                parameters,
                attributes: Vec::new(),
                body: String::new(),
            })
        })
        .collect();
    let declarations = externs
        .iter()
        .filter_map(|(key, module, signature)| {
            let function = functions.get(key)?;
            let ScalarType::Callable {
                outputs,
                parameters,
            } = signature
            else {
                return None;
            };
            let result = callable_result(outputs).ok()?;
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
                name: function.get_name().to_string_lossy().to_string(),
                result: value_type(result).ok()?,
                parameters,
                attributes: vec![LlvmFunctionAttributes {
                    group: 0,
                    wasm_import_module: module.clone(),
                    wasm_import_name: function.get_name().to_string_lossy().to_string(),
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
    let result = callable_result(outputs)?;
    let parameters = parameters
        .iter()
        .map(|ty| basic_type(context, ty).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    Ok(match result {
        ScalarType::Unit => context.void_type().fn_type(&parameters, false),
        ScalarType::Bool => context.bool_type().fn_type(&parameters, false),
        ScalarType::Utf8 | ScalarType::RawPointer(_) => {
            context.i32_type().fn_type(&parameters, false)
        }
        ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::U128 => integer_type(context, result)?.fn_type(&parameters, false),
        _ => return Err("unsupported LLVM scalar type".into()),
    })
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
        ScalarType::Utf8 => Ok(context.i32_type().into()),
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

fn callable_result(outputs: &crate::ScalarOutputSequence) -> Result<&ScalarType, String> {
    match outputs.outputs.as_slice() {
        [output] => Ok(&output.ty),
        _ => Err(format!(
            "structured scalar expression lowering is pending at {}..{}",
            outputs.span.start, outputs.span.end
        )),
    }
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
        (ScalarExpression::StructLiteral { fields, .. }, ScalarType::Struct(_)) => {
            emit_struct_literal(context, state, expected, fields)
        }
        (ScalarExpression::Integer { value, .. }, ty) if integer_width(ty).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, ty, value)?.into()),
        ),
        _ => emit_expression(context, state, expression),
    }
}

fn emit_utf8_literal<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: &[u8],
) -> Result<EmitValue<'ctx>, String> {
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
    Ok(EmitValue::Basic(
        state
            .builder
            .build_ptr_to_int(
                literal.as_pointer_value(),
                context.i32_type(),
                "utf8_address",
            )
            .map_err(builder_error)?
            .into(),
    ))
}

fn utf8_length<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
) -> Result<u64, String> {
    match expression {
        ScalarExpression::Utf8 { value, .. } => {
            u64::try_from(value.len()).map_err(|_| "utf8 literal length is not u64".to_owned())
        }
        ScalarExpression::Name { name, .. } => state
            .utf8_lengths
            .get(name)
            .copied()
            .ok_or_else(|| format!("unknown UTF-8 length {name}")),
        _ => Err("core.utf8_view requires a UTF-8 value with known storage".to_owned()),
    }
}

fn project_utf8_length(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<u64, String> {
    match expression {
        ScalarExpression::Member { receiver, name, .. } => {
            let target = project_namespace_target(module, modules, receiver)?;
            let binding = target.items.iter().find_map(|item| match item {
                ScalarItem::Binding(binding) if binding.name == *name => Some(binding),
                _ => None,
            });
            let binding = binding.ok_or_else(|| format!("unknown UTF-8 member {name}"))?;
            match &binding.value {
                ScalarExpression::Utf8 { value, .. } => u64::try_from(value.len())
                    .map_err(|_| "utf8 literal length is not u64".to_owned()),
                _ => Err(format!("unknown UTF-8 length {receiver}.{name}")),
            }
        }
        _ => utf8_length(state, expression),
    }
}

fn emit_utf8_view<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    argument: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    let pointer = emit_expression(context, state, argument)?;
    let pointer = take_basic(pointer)?;
    let length = context
        .i64_type()
        .const_int(utf8_length(state, argument)?, false);
    Ok(EmitValue::Utf8View {
        pointer,
        length: length.into(),
    })
}

fn insert_utf8_lengths(
    lengths: &mut BTreeMap<String, u64>,
    items: &[ScalarItem],
) -> Result<(), String> {
    for item in items {
        if let ScalarItem::Binding(binding) = item {
            if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                lengths.insert(
                    binding.name.clone(),
                    u64::try_from(value.len()).map_err(|_| "utf8 literal length is not u64")?,
                );
            }
        }
    }
    Ok(())
}

fn materialize_utf8_view<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    receivers: &[crate::ScalarOutputReceiver],
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    let EmitValue::Utf8View { pointer, length } = value else {
        return Err("core.utf8_view did not produce its canonical outputs".to_owned());
    };
    let first = state
        .builder
        .build_alloca(basic_type(context, &receivers[0].ty)?, &receivers[0].name)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(first, pointer)
        .map_err(builder_error)?;
    let second = state
        .builder
        .build_alloca(basic_type(context, &receivers[1].ty)?, &receivers[1].name)
        .map_err(builder_error)?;
    state
        .builder
        .build_store(second, length)
        .map_err(builder_error)?;
    state
        .storage
        .insert(receivers[0].name.clone(), (first, receivers[0].ty.clone()));
    state
        .storage
        .insert(receivers[1].name.clone(), (second, receivers[1].ty.clone()));
    Ok(())
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
        EmitValue::Utf8View { .. } => {
            Err("structured UTF-8 view used where a scalar value is required".into())
        }
        EmitValue::Unit => Err("unit value used where a scalar value is required".into()),
    }
}

fn emit_function<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        utf8_lengths: BTreeMap::new(),
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
    let result_type = match &function.signature {
        ScalarType::Callable { outputs, .. } => callable_result(outputs)?,
        _ => return Err("function has no callable signature".into()),
    };
    let result = match function.body.items.as_slice() {
        [ScalarBlockItem::Expression(expression)] => {
            emit_typed_expression(context, &mut state, expression, result_type)?
        }
        _ => emit_block(context, &mut state, &function.body)?,
    };
    emit_return(&mut state, result, result_type)
}

fn emit_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: globals.clone(),
        all_globals: globals.clone(),
        structs,
        utf8_lengths: BTreeMap::new(),
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, items);
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                let value = emit_typed_expression(
                    context,
                    &mut state,
                    &binding.value,
                    &binding.declared_type,
                )?;
                if binding.receivers.len() == 2 {
                    materialize_utf8_view(context, &mut state, &binding.receivers, value)?;
                    continue;
                }
                if binding.declared_type != ScalarType::Unit {
                    let (global, _) = state
                        .globals
                        .get(&binding.name)
                        .cloned()
                        .ok_or_else(|| format!("unknown LLVM global {}", binding.name))?;
                    store_value(
                        context,
                        &mut state,
                        global.as_pointer_value(),
                        &binding.declared_type,
                        value,
                    )?;
                }
                if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                    state.utf8_lengths.insert(
                        binding.name.clone(),
                        u64::try_from(value.len()).map_err(|_| "utf8 literal length is not u64")?,
                    );
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: module_globals(source_module, globals),
        all_globals: globals.clone(),
        structs,
        utf8_lengths: BTreeMap::new(),
        next_literal: 0,
        next_block: 0,
    };
    insert_unit_values(&mut state.values, &source_module.items);
    insert_utf8_lengths(&mut state.utf8_lengths, &source_module.items)?;
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
    let result_type = match &function.signature {
        ScalarType::Callable { outputs, .. } => callable_result(outputs)?,
        _ => return Err("function has no callable signature".into()),
    };
    let result = match function.body.items.as_slice() {
        [ScalarBlockItem::Expression(expression)] => emit_project_typed_expression(
            context,
            &mut state,
            expression,
            result_type,
            source_module,
            modules,
        )?,
        _ => emit_project_block(context, &mut state, &function.body, source_module, modules)?,
    };
    emit_return(&mut state, result, result_type)
}

fn emit_project_main<'ctx, 'module>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
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
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: BTreeMap::new(),
        all_globals: globals.clone(),
        structs,
        utf8_lengths: BTreeMap::new(),
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
    let utf8_lengths = state.utf8_lengths.clone();
    let current_module_globals = module_globals(module, globals);
    let current_globals = std::mem::replace(&mut state.globals, current_module_globals);
    state.values.clear();
    state.storage.clear();
    insert_unit_values(&mut state.values, &module.items);
    state.utf8_lengths.clear();
    insert_utf8_lengths(&mut state.utf8_lengths, &module.items)?;

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
                let value = emit_project_typed_expression(
                    context,
                    state,
                    &binding.value,
                    &binding.declared_type,
                    module,
                    modules,
                )?;
                if binding.receivers.len() == 2 {
                    materialize_utf8_view(context, state, &binding.receivers, value)?;
                    continue;
                }
                if binding.declared_type != ScalarType::Unit {
                    let (global, _) = state
                        .globals
                        .get(&binding.name)
                        .cloned()
                        .ok_or_else(|| format!("unknown LLVM global {}", binding.name))?;
                    store_value(
                        context,
                        state,
                        global.as_pointer_value(),
                        &binding.declared_type,
                        value,
                    )?;
                }
                if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                    state.utf8_lengths.insert(
                        binding.name.clone(),
                        u64::try_from(value.len()).map_err(|_| "utf8 literal length is not u64")?,
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
    state.storage = storage;
    state.utf8_lengths = utf8_lengths;
    Ok(())
}

fn emit_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let values = state.values.clone();
    let utf8_lengths = state.utf8_lengths.clone();
    let mut result = EmitValue::Unit;
    for item in &block.items {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value =
                    emit_typed_expression(context, state, &binding.value, &binding.declared_type)?;
                if binding.declared_type == ScalarType::Unit {
                    state.values.insert(binding.name.clone(), EmitValue::Unit);
                    EmitValue::Unit
                } else if binding.receivers.len() == 2 {
                    materialize_utf8_view(context, state, &binding.receivers, value)?;
                    EmitValue::Unit
                } else {
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
                    if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                        state.utf8_lengths.insert(
                            binding.name.clone(),
                            u64::try_from(value.len())
                                .map_err(|_| "utf8 literal length is not u64")?,
                        );
                    }
                    EmitValue::Unit
                }
            }
            ScalarBlockItem::Expression(expression) => emit_expression(context, state, expression)?,
            ScalarBlockItem::Assignment(assignment) => emit_assignment(context, state, assignment)?,
            ScalarBlockItem::While(expression) => emit_while(context, state, expression)?,
        };
    }
    state.storage = storage;
    state.values = values;
    state.utf8_lengths = utf8_lengths;
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
    let utf8_lengths = state.utf8_lengths.clone();
    let mut result = EmitValue::Unit;
    for item in &block.items {
        result = match item {
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
                    EmitValue::Unit
                } else if binding.receivers.len() == 2 {
                    materialize_utf8_view(context, state, &binding.receivers, value)?;
                    EmitValue::Unit
                } else {
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
                    if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                        state.utf8_lengths.insert(
                            binding.name.clone(),
                            u64::try_from(value.len())
                                .map_err(|_| "utf8 literal length is not u64")?,
                        );
                    }
                    EmitValue::Unit
                }
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
    state.storage = storage;
    state.values = values;
    state.utf8_lengths = utf8_lengths;
    Ok(result)
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
            Some(EmitValue::Utf8View { .. }) => {
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
            Some(EmitValue::Utf8View { .. }) => {
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
            if binding.receivers.len() == 2 {
                materialize_utf8_view(context, state, &binding.receivers, value)?;
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
            if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                state.utf8_lengths.insert(
                    binding.name.clone(),
                    u64::try_from(value.len()).map_err(|_| "utf8 literal length is not u64")?,
                );
            }
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
            if binding.receivers.len() == 2 {
                materialize_utf8_view(context, state, &binding.receivers, value)?;
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
            if let ScalarExpression::Utf8 { value, .. } = &binding.value {
                state.utf8_lengths.insert(
                    binding.name.clone(),
                    u64::try_from(value.len()).map_err(|_| "utf8 literal length is not u64")?,
                );
            }
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
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Utf8 { value, .. } => emit_utf8_literal(context, state, value),
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
            emit_call(context, state, receiver.as_deref(), name, arguments)
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => emit_if(context, state, condition, then_branch, else_branch),
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
        ScalarExpression::Integer { value, .. } if integer_width(expected).is_some() => Ok(
            EmitValue::Basic(integer_constant(context, expected, value)?.into()),
        ),
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
            if receiver.as_deref() == Some("core") && name == "utf8_view" {
                let [argument] = arguments.as_slice() else {
                    return Err("core.utf8_view has invalid argument arity".to_owned());
                };
                return match argument {
                    ScalarExpression::Utf8 { .. }
                    | ScalarExpression::Name { .. }
                    | ScalarExpression::Member { .. } => {
                        let pointer =
                            emit_project_expression(context, state, argument, module, modules)?;
                        let pointer = take_basic(pointer)?;
                        let length = project_utf8_length(state, argument, module, modules)?;
                        Ok(EmitValue::Utf8View {
                            pointer,
                            length: context.i64_type().const_int(length, false).into(),
                        })
                    }
                    _ => Err("core.utf8_view requires a UTF-8 value with known storage".to_owned()),
                };
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
            let qualified = project_function_name(&target.source, name);
            let values = arguments
                .iter()
                .map(|argument| emit_project_expression(context, state, argument, module, modules))
                .collect::<Result<Vec<_>, _>>()?;
            emit_call_values(state, &qualified, values)
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
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Utf8 { value, .. } => emit_utf8_literal(context, state, value),
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            let unsigned = expression_is_unsigned(state, left);
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
    if receiver == Some("core") && name == "utf8_view" {
        let [argument] = arguments else {
            return Err("core.utf8_view has invalid argument arity".to_owned());
        };
        return match argument {
            ScalarExpression::Utf8 { .. } | ScalarExpression::Name { .. } => {
                emit_utf8_view(context, state, argument)
            }
            _ => Err("core.utf8_view requires a UTF-8 value with known storage".to_owned()),
        };
    }
    let values = arguments
        .iter()
        .map(|argument| emit_expression(context, state, argument))
        .collect::<Result<Vec<_>, _>>()?;
    let qualified =
        receiver.map_or_else(|| name.to_owned(), |receiver| format!("{receiver}.{name}"));
    emit_call_values(state, &qualified, values)
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
            EmitValue::Utf8View { .. } => Err("structured UTF-8 view argument is invalid".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let call = state
        .builder
        .build_call(function, &arguments, "call")
        .map_err(builder_error)?;
    match call.try_as_basic_value() {
        ValueKind::Basic(value) => Ok(EmitValue::Basic(value)),
        ValueKind::Instruction(_) => Ok(EmitValue::Unit),
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
        _ => false,
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
            ExpressionPath::Single => emit_expression(context, state, right)?,
            ExpressionPath::Project { module, modules } => {
                emit_project_expression(context, state, right, module, modules)?
            }
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
    let left = take_basic(left)?.into_int_value();
    let right = take_basic(right)?.into_int_value();
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
    };
    Ok(EmitValue::Basic(value))
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
    result: &ScalarType,
) -> Result<(), String> {
    match result {
        ScalarType::Unit => {
            state.builder.build_return(None).map_err(builder_error)?;
        }
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
        | ScalarType::RawPointer(_)
        | ScalarType::Struct(_) => {
            let value = take_basic(value)?;
            state
                .builder
                .build_return(Some(&value))
                .map_err(builder_error)?;
        }
        _ => return Err("unsupported LLVM return type".into()),
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
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => globals
                .get(&project_global_name(&module.source, &binding.name))
                .cloned()
                .map(|global| (binding.name.clone(), global)),
            ScalarItem::Namespace(_)
            | ScalarItem::Extern(_)
            | ScalarItem::Function(_)
            | ScalarItem::Executable(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::emit_scalar_llvm;
    use super::emit_scalar_project_llvm;
    use super::project_function_name;
    use super::project_global_name;
    use crate::{
        derive_scalar_program, parse_source, validate_scalar_project, ScalarModule, ScalarProject,
    };
    use wosy_syntax::SourceIdentity;

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
        assert_eq!(partition.declarations[0].name, "read");
        assert_eq!(
            partition.declarations[0].attributes[0].wasm_import_module,
            "helper"
        );
        assert_eq!(partition.declarations[1].name, "flush");
        assert!(partition.to_text().contains("declare i32 @read(i32) #0"));
        assert!(partition.to_text().contains("declare void @flush() #0"));
        assert!(partition.to_text().contains("call i32 @read(i32 7)"));
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
    fn reports_unsupported_multi_output_lowering_with_output_span() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, u64)() pair = fn { 1 };\n%%end".into(),
                &[],
            )
            .result,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let error = emit_scalar_llvm(&validation).expect_err("multi-output LLVM");
        assert!(error.contains("structured scalar expression lowering is pending"));
        assert!(error.contains("at 8..20"), "{error}");
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

        assert!(text.contains("declare void @touch(ptr) #0"), "{text}");
        assert!(text.contains("declare void @inspect(ptr) #0"), "{text}");
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
        assert!(
            forward.contains("call void @touch(i32 %pointer"),
            "{forward}"
        );
        assert!(forward.contains("load i32, ptr %field"), "{forward}");
        assert!(
            forward.contains("call void @inspect(i32 %field"),
            "{forward}"
        );
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

        assert!(text.contains("declare void @touch(ptr) #0"), "{text}");
        assert!(
            text.contains(&format!("@{value} = internal global [8 x i8]")),
            "{text}"
        );
        assert!(
            main.contains(&format!("ptrtoint (ptr @{value} to i32)")),
            "{main}"
        );
        assert!(main.contains("load i32, ptr %pointer"), "{main}");
        assert!(main.contains("call void @touch(i32 %pointer"), "{main}");
    }

    #[test]
    fn emits_utf8_view_literal_and_bound_outputs_in_order() {
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
utf8 text = \"hé\";
unsafe {
    *?u8 literal_bytes, u64 literal_length = core.utf8_view(\"hé\");
    *?u8 bound_bytes, u64 bound_length = core.utf8_view(text);
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
        let text = emit_scalar_llvm(&validation)
            .expect("UTF-8 view LLVM")
            .to_text();
        let main = text.split("define i32 @main").nth(1).expect("main");
        assert!(
            text.contains("@wosy_utf8_literal_0 = private constant [3 x i8]"),
            "{text}"
        );
        assert!(main.matches("store i64 3").count() == 2, "{main}");
        assert!(
            main.find("store i32").expect("pointer store")
                < main.find("store i64 3").expect("length store")
        );
        assert!(main.contains("alloca i32"));
        assert!(main.contains("alloca i64"));
    }

    #[test]
    fn emits_utf8_view_for_project_module_binding() {
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
utf8 text = \"猫\";
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
unsafe {
    *?u8 bytes, u64 length = core.utf8_view(child.text);
};
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
                ScalarModule::new(root_source, root.items, namespace_bindings),
                ScalarModule::new(child_source, child.items, Vec::new()),
            ],
            vec![
                SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "src/main.w".into(),
                    "r1".into(),
                ),
                SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "src/child.w".into(),
                    "r1".into(),
                ),
            ],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("project UTF-8 view LLVM")
            .to_text();
        assert!(
            text.contains("@wosy_utf8_literal_0 = private constant [3 x i8]"),
            "{text}"
        );
        assert!(text.contains("store i64 3"), "{text}");
    }
}
