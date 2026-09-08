use std::collections::BTreeMap;

use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, GlobalValue, PointerValue, ValueKind,
};
use inkwell::IntPredicate;
use serde::{Deserialize, Serialize};

use crate::{
    BinaryOperator, ScalarAssignment, ScalarBlock, ScalarBlockItem, ScalarExpression,
    ScalarFunction, ScalarItem, ScalarModule, ScalarProjectValidation, ScalarType,
    ScalarValidation, ScalarWhile,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LlvmValueType {
    Void,
    I1,
    I32,
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
    pub body: String,
}

#[derive(Clone)]
enum EmitValue<'ctx> {
    Unit,
    Basic(BasicValueEnum<'ctx>),
}

struct EmitState<'ctx> {
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    globals: BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    next_block: usize,
}

pub fn emit_scalar_llvm(validation: &ScalarValidation) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".into());
    }
    let context = Context::create();
    let module = context.create_module(&validation.program.source.path);
    let builder = context.create_builder();
    let signatures = validation
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
            ScalarItem::Namespace(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
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
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    for item in &validation.program.items {
        if let ScalarItem::Function(function) = item {
            emit_function(&context, &builder, &functions, function, &function.name)?;
        }
    }
    emit_main(&context, &builder, &functions, &validation.program.items)?;
    finish_partition(
        module,
        validation.program.source.path.clone(),
        functions,
        &signatures,
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
    let module_name = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .source
        .project
        .clone();
    let context = Context::create();
    let module = context.create_module(&module_name);
    let builder = context.create_builder();
    let mut globals = BTreeMap::new();
    let mut functions = BTreeMap::new();
    let mut signatures = BTreeMap::new();
    let mut definitions = Vec::new();
    for source_module in &modules {
        for item in &source_module.items {
            if let ScalarItem::Binding(binding) = item {
                let name = project_global_name(&source_module.source, &binding.name);
                let global =
                    module.add_global(basic_type(&context, &binding.declared_type)?, None, &name);
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
        )?;
    }
    emit_project_main(&context, &builder, &functions, &modules, &globals)?;
    finish_partition(module, module_name, functions, &signatures)
}

pub fn emit_scalar_llvm_text(validation: &ScalarValidation) -> Result<String, String> {
    Ok(emit_scalar_llvm(validation)?.to_text())
}

impl LlvmPartition {
    pub fn to_text(&self) -> String {
        self.text.clone()
    }
}

fn finish_partition<'ctx>(
    module: inkwell::module::Module<'ctx>,
    module_name: String,
    functions: BTreeMap<String, FunctionValue<'ctx>>,
    signatures: &BTreeMap<String, ScalarType>,
) -> Result<LlvmPartition, String> {
    module.verify().map_err(|error| error.to_string())?;
    let text = module.print_to_string().to_string();
    let metadata = functions
        .iter()
        .filter_map(|(name, function)| {
            let signature = if name == "main" {
                ScalarType::Callable {
                    result: Box::new(ScalarType::I32),
                    parameters: Vec::new(),
                }
            } else {
                signatures.get(name)?.clone()
            };
            let ScalarType::Callable { result, parameters } = signature else {
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
                result: value_type(&result).ok()?,
                parameters,
                body: String::new(),
            })
        })
        .collect();
    Ok(LlvmPartition {
        module_name,
        declarations: Vec::new(),
        functions: metadata,
        text,
    })
}

fn function_type<'ctx>(
    context: &'ctx Context,
    signature: &ScalarType,
) -> Result<FunctionType<'ctx>, String> {
    let ScalarType::Callable { result, parameters } = signature else {
        return Err("function has no callable signature".into());
    };
    let parameters = parameters
        .iter()
        .map(|ty| basic_type(context, ty).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    Ok(match result.as_ref() {
        ScalarType::Unit => context.void_type().fn_type(&parameters, false),
        ScalarType::Bool => context.bool_type().fn_type(&parameters, false),
        ScalarType::I32 => context.i32_type().fn_type(&parameters, false),
        _ => return Err("unsupported LLVM scalar type".into()),
    })
}

fn basic_type<'ctx>(
    context: &'ctx Context,
    ty: &ScalarType,
) -> Result<BasicTypeEnum<'ctx>, String> {
    match ty {
        ScalarType::Bool => Ok(context.bool_type().into()),
        ScalarType::I32 => Ok(context.i32_type().into()),
        _ => Err("unit is only valid as a function result".into()),
    }
}
fn value_type(ty: &ScalarType) -> Result<LlvmValueType, String> {
    match ty {
        ScalarType::Unit => Ok(LlvmValueType::Void),
        ScalarType::Bool => Ok(LlvmValueType::I1),
        ScalarType::I32 => Ok(LlvmValueType::I32),
        _ => Err("unsupported LLVM scalar type".into()),
    }
}
fn builder_error(error: BuilderError) -> String {
    error.to_string()
}
fn take_basic(value: EmitValue<'_>) -> Result<BasicValueEnum<'_>, String> {
    match value {
        EmitValue::Basic(value) => Ok(value),
        EmitValue::Unit => Err("unit value used where a scalar value is required".into()),
    }
}

fn emit_function<'ctx>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    function: &ScalarFunction,
    name: &str,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        builder,
        functions,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: BTreeMap::new(),
        next_block: 0,
    };
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(index as u32)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            let slot = builder
                .build_alloca(basic_type(context, parameter)?, &function.parameters[index])
                .map_err(builder_error)?;
            builder.build_store(slot, argument).map_err(builder_error)?;
            state.storage.insert(
                function.parameters[index].clone(),
                (slot, parameter.clone()),
            );
        }
    }
    let result = emit_block(context, &mut state, &function.body)?;
    let result_type = match &function.signature {
        ScalarType::Callable { result, .. } => result.as_ref(),
        _ => return Err("function has no callable signature".into()),
    };
    emit_return(&mut state, result, result_type)
}

fn emit_main<'ctx>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    items: &[ScalarItem],
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        builder,
        functions,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: BTreeMap::new(),
        next_block: 0,
    };
    for item in items {
        if let ScalarItem::Binding(binding) = item {
            let value = emit_expression(context, &mut state, &binding.value)?;
            state.values.insert(binding.name.clone(), value);
        }
    }
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn emit_project_function<'ctx>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    function: &ScalarFunction,
    source_module: &ScalarModule,
    modules: &[&ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
    name: &str,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        builder,
        functions,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: module_globals(source_module, globals),
        next_block: 0,
    };
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(index as u32)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            let slot = builder
                .build_alloca(basic_type(context, parameter)?, &function.parameters[index])
                .map_err(builder_error)?;
            builder.build_store(slot, argument).map_err(builder_error)?;
            state.storage.insert(
                function.parameters[index].clone(),
                (slot, parameter.clone()),
            );
        }
    }
    let result = emit_project_block(context, &mut state, &function.body, source_module, modules)?;
    let result_type = match &function.signature {
        ScalarType::Callable { result, .. } => result.as_ref(),
        _ => return Err("function has no callable signature".into()),
    };
    emit_return(&mut state, result, result_type)
}

fn emit_project_main<'ctx>(
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    modules: &[&ScalarModule],
    globals: &BTreeMap<String, (GlobalValue<'ctx>, ScalarType)>,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        builder,
        functions,
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        globals: BTreeMap::new(),
        next_block: 0,
    };
    for module in modules {
        state.values.clear();
        state.globals = module_globals(module, globals);
        for node in &module.initialization_nodes {
            if let ScalarItem::Binding(binding) = &module.items[node.item_index] {
                let value =
                    emit_project_expression(context, &mut state, &binding.value, module, modules)?;
                let value = take_basic(value)?;
                let (global, _) = state
                    .globals
                    .get(&binding.name)
                    .cloned()
                    .ok_or_else(|| format!("unknown LLVM global {}", binding.name))?;
                state
                    .builder
                    .build_store(global.as_pointer_value(), value)
                    .map_err(builder_error)?;
            }
        }
    }
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn emit_block<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let mut result = EmitValue::Unit;
    for item in &block.items {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value = take_basic(emit_expression(context, state, &binding.value)?)?;
                let slot = state
                    .builder
                    .build_alloca(basic_type(context, &binding.declared_type)?, &binding.name)
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_store(slot, value)
                    .map_err(builder_error)?;
                state
                    .storage
                    .insert(binding.name.clone(), (slot, binding.declared_type.clone()));
                EmitValue::Unit
            }
            ScalarBlockItem::Expression(expression) => emit_expression(context, state, expression)?,
            ScalarBlockItem::Assignment(assignment) => emit_assignment(context, state, assignment)?,
            ScalarBlockItem::While(expression) => emit_while(context, state, expression)?,
        };
    }
    state.storage = storage;
    Ok(result)
}

fn emit_project_block<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let storage = state.storage.clone();
    let mut result = EmitValue::Unit;
    for item in &block.items {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value = take_basic(emit_project_expression(
                    context,
                    state,
                    &binding.value,
                    module,
                    modules,
                )?)?;
                let slot = state
                    .builder
                    .build_alloca(basic_type(context, &binding.declared_type)?, &binding.name)
                    .map_err(builder_error)?;
                state
                    .builder
                    .build_store(slot, value)
                    .map_err(builder_error)?;
                state
                    .storage
                    .insert(binding.name.clone(), (slot, binding.declared_type.clone()));
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
    state.storage = storage;
    Ok(result)
}

fn emit_assignment<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    assignment: &ScalarAssignment,
) -> Result<EmitValue<'ctx>, String> {
    let value = take_basic(emit_expression(context, state, &assignment.value)?)?;
    let (slot, ty) = state
        .storage
        .get(&assignment.target)
        .cloned()
        .ok_or_else(|| format!("unknown LLVM storage {}", assignment.target))?;
    let old = state
        .builder
        .build_load(basic_type(context, &ty)?, slot, "old")
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slot, value)
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(old))
}
fn emit_project_assignment<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    let value = take_basic(emit_project_expression(
        context,
        state,
        &assignment.value,
        module,
        modules,
    )?)?;
    let (slot, ty) = state
        .storage
        .get(&assignment.target)
        .cloned()
        .ok_or_else(|| format!("unknown LLVM storage {}", assignment.target))?;
    let old = state
        .builder
        .build_load(basic_type(context, &ty)?, slot, "old")
        .map_err(builder_error)?;
    state
        .builder
        .build_store(slot, value)
        .map_err(builder_error)?;
    Ok(EmitValue::Basic(old))
}

fn emit_while<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
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

fn emit_project_while<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
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

fn emit_expression<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    expression: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            if let Some((slot, ty)) = state.storage.get(name).cloned() {
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(basic_type(context, &ty)?, slot, name)
                        .map_err(builder_error)?,
                ));
            }
            if let Some((global, ty)) = state.globals.get(name).cloned() {
                return Ok(EmitValue::Basic(
                    state
                        .builder
                        .build_load(basic_type(context, &ty)?, global.as_pointer_value(), name)
                        .map_err(builder_error)?,
                ));
            }
            state
                .values
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown LLVM value {name}"))
        }
        ScalarExpression::Integer { value, .. } => Ok(EmitValue::Basic(
            context.i32_type().const_int(*value as u64, true).into(),
        )),
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => emit_binary(context, state, operator, left, right),
        ScalarExpression::Call {
            name, arguments, ..
        } => emit_call(context, state, name, arguments),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => emit_if(context, state, condition, then_branch, else_branch),
        ScalarExpression::Block(block) => emit_block(context, state, block),
    }
}

fn emit_project_expression<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    expression: &ScalarExpression,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match expression {
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            ..
        } => {
            let target = receiver.as_ref().map_or(module, |binding| {
                let namespace = module
                    .namespace_bindings
                    .iter()
                    .find(|namespace| namespace.binding == *binding)
                    .expect("validated namespace binding");
                modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)
                    .expect("validated namespace target")
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
            context.i32_type().const_int(*value as u64, true).into(),
        )),
        ScalarExpression::Boolean { value, .. } => Ok(EmitValue::Basic(
            context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
        )),
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            let left = emit_project_expression(context, state, left, module, modules)?;
            let right = emit_project_expression(context, state, right, module, modules)?;
            emit_binary_values(state, operator, left, right)
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
    }
}

fn emit_call<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    name: &str,
    arguments: &[ScalarExpression],
) -> Result<EmitValue<'ctx>, String> {
    let values = arguments
        .iter()
        .map(|argument| emit_expression(context, state, argument))
        .collect::<Result<Vec<_>, _>>()?;
    emit_call_values(state, name, values)
}

fn emit_call_values<'ctx>(
    state: &mut EmitState<'ctx>,
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

fn emit_binary<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    operator: &BinaryOperator,
    left: &ScalarExpression,
    right: &ScalarExpression,
) -> Result<EmitValue<'ctx>, String> {
    let left = emit_expression(context, state, left)?;
    let right = emit_expression(context, state, right)?;
    emit_binary_values(state, operator, left, right)
}
fn emit_binary_values<'ctx>(
    state: &mut EmitState<'ctx>,
    operator: &BinaryOperator,
    left: EmitValue<'ctx>,
    right: EmitValue<'ctx>,
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
        BinaryOperator::Divide => state
            .builder
            .build_int_signed_div(left, right, "div")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Remainder => state
            .builder
            .build_int_signed_rem(left, right, "rem")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::And => state
            .builder
            .build_and(left, right, "and")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Or => state
            .builder
            .build_or(left, right, "or")
            .map_err(builder_error)?
            .into(),
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
            .build_int_compare(IntPredicate::SLT, left, right, "lt")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::LessEqual => state
            .builder
            .build_int_compare(IntPredicate::SLE, left, right, "le")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::Greater => state
            .builder
            .build_int_compare(IntPredicate::SGT, left, right, "gt")
            .map_err(builder_error)?
            .into(),
        BinaryOperator::GreaterEqual => state
            .builder
            .build_int_compare(IntPredicate::SGE, left, right, "ge")
            .map_err(builder_error)?
            .into(),
    };
    Ok(EmitValue::Basic(value))
}

fn emit_if<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
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
fn emit_project_if<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
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
fn emit_if_value<'ctx, F>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx>,
    condition: inkwell::values::IntValue<'ctx>,
    then_branch: &ScalarBlock,
    else_branch: &ScalarBlock,
    emit: F,
) -> Result<EmitValue<'ctx>, String>
where
    F: Fn(&mut EmitState<'ctx>, &ScalarBlock) -> Result<EmitValue<'ctx>, String>,
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

fn emit_return<'ctx>(
    state: &mut EmitState<'ctx>,
    value: EmitValue<'ctx>,
    result: &ScalarType,
) -> Result<(), String> {
    match result {
        ScalarType::Unit => {
            state.builder.build_return(None).map_err(builder_error)?;
        }
        ScalarType::Bool | ScalarType::I32 => {
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
    format!("{}__{}__{}", source.package, source.path, name)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn project_global_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    project_function_name(source, &format!("global_{name}"))
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
            ScalarItem::Namespace(_) | ScalarItem::Function(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::emit_scalar_llvm;
    use super::emit_scalar_project_llvm;
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
            "%%start\nbool() flag = fn { true };\nunit() touch = fn { i32 marker = 0; };\ni32() count = fn { 42 };\nbool selected = flag();\nunit done = touch();\ni32 result = count();\n%%end"
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
            .split("define i32 @package__src_main_w__read")
            .nth(1)
            .expect("read");
        assert!(main.contains("store i32 41"));
        assert!(read.contains("load i32, ptr"));
    }
}
