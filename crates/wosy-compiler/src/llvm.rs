use std::collections::BTreeMap;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::{
    BinaryOperator, ScalarAssignment, ScalarBlock, ScalarBlockItem, ScalarExpression,
    ScalarFunction, ScalarItem, ScalarModule, ScalarProjectValidation, ScalarType,
    ScalarValidation,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmFunction {
    pub name: String,
    pub result: LlvmValueType,
    pub parameters: Vec<(String, LlvmValueType)>,
    pub body: String,
}

pub fn emit_scalar_llvm(validation: &ScalarValidation) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".to_owned());
    }
    let mut functions = Vec::new();
    for item in &validation.program.items {
        if let ScalarItem::Function(function) = item {
            functions.push(emit_function(function)?);
        }
    }
    functions.push(emit_main(&validation.program.items)?);
    Ok(LlvmPartition {
        module_name: validation.program.source.path.clone(),
        declarations: Vec::new(),
        functions,
    })
}

pub fn emit_scalar_project_llvm(
    validation: &ScalarProjectValidation,
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar project".to_owned());
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
    let mut declarations = Vec::new();
    let mut functions = Vec::new();
    for module in &modules {
        for item in &module.items {
            if let ScalarItem::Function(function) = item {
                let declaration = llvm_function_signature(function, &module.source)?;
                declarations.push(declaration);
            }
        }
    }
    for module in &modules {
        for item in &module.items {
            if let ScalarItem::Function(function) = item {
                functions.push(emit_project_function(function, module, &modules)?);
            }
        }
    }
    functions.push(emit_project_main(&modules)?);
    let module_name = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .source
        .project
        .clone();
    Ok(LlvmPartition {
        module_name,
        declarations,
        functions,
    })
}

pub fn emit_scalar_llvm_text(validation: &ScalarValidation) -> Result<String, String> {
    Ok(emit_scalar_llvm(validation)?.to_text())
}

impl LlvmPartition {
    pub fn to_text(&self) -> String {
        let mut text = format!("; ModuleID = '{}'\n", self.module_name);
        text.push_str("source_filename = \"wosy\"\n\n");
        for function in &self.declarations {
            if self
                .functions
                .iter()
                .any(|definition| definition.name == function.name)
            {
                continue;
            }
            let _ = writeln!(
                text,
                "declare {} @{}({})",
                llvm_type(function.result),
                function.name,
                parameters_text(&function.parameters)
            );
        }
        if !self.declarations.is_empty() {
            text.push('\n');
        }
        for function in &self.functions {
            let _ = writeln!(
                text,
                "define {} @{}({}) {{",
                llvm_type(function.result),
                function.name,
                parameters_text(&function.parameters)
            );
            text.push_str(&function.body);
            text.push_str("}\n\n");
        }
        text
    }
}

fn llvm_function_signature(
    function: &ScalarFunction,
    source: &wosy_syntax::SourceIdentity,
) -> Result<LlvmFunction, String> {
    let ScalarType::Callable { result, parameters } = &function.signature else {
        return Err(format!(
            "function {} has no callable signature",
            function.name
        ));
    };
    let llvm_parameters = parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| Ok((function.parameters[index].clone(), value_type(parameter)?)))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(LlvmFunction {
        name: project_function_name(source, &function.name),
        result: value_type(result)?,
        parameters: llvm_parameters,
        body: String::new(),
    })
}

fn emit_project_function(
    function: &ScalarFunction,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<LlvmFunction, String> {
    let signature = llvm_function_signature(function, &module.source)?;
    let names = signature
        .parameters
        .iter()
        .map(|(name, _)| (name.clone(), format!("%{name}")))
        .collect();
    let mut state = EmitState::new(names, BTreeMap::new());
    state.initialize_parameters(&signature.parameters);
    let value = emit_project_block(&function.body, &mut state, module, modules)?;
    append_return(&mut state, value, signature.result)?;
    Ok(LlvmFunction {
        body: state.body,
        ..signature
    })
}

fn emit_project_main(modules: &[&ScalarModule]) -> Result<LlvmFunction, String> {
    let mut state = EmitState::new(BTreeMap::new(), BTreeMap::new());
    let mut initialized = Vec::new();
    for module in modules {
        if initialized.contains(&module.source) {
            continue;
        }
        initialized.push(module.source.clone());
        state.values.clear();
        for node in &module.initialization_nodes {
            if let ScalarItem::Binding(binding) = &module.items[node.item_index] {
                let value = emit_project_expression(&binding.value, &mut state, module, modules)?;
                state.values.insert(binding.name.clone(), value);
            }
        }
    }
    state.body.push_str("  ret i32 0\n");
    Ok(LlvmFunction {
        name: "main".to_owned(),
        result: LlvmValueType::I32,
        parameters: Vec::new(),
        body: state.body,
    })
}

fn emit_project_block(
    block: &ScalarBlock,
    state: &mut EmitState,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<(String, LlvmValueType), String> {
    let storage = state.storage.clone();
    let mut result = ("0".to_owned(), LlvmValueType::Void);
    for item in &block.items {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value = emit_project_expression(&binding.value, state, module, modules)?;
                let slot = state.allocate(&binding.name, value_type(&binding.declared_type)?);
                state.body.push_str(&format!(
                    "  store {} {}, ptr {}\n",
                    llvm_type(value.1),
                    value.0,
                    slot
                ));
                ("0".to_owned(), LlvmValueType::Void)
            }
            ScalarBlockItem::Expression(expression) => {
                emit_project_expression(expression, state, module, modules)?
            }
            ScalarBlockItem::Assignment(assignment) => {
                emit_project_assignment(assignment, state, module, modules)?
            }
            ScalarBlockItem::While(while_expression) => {
                let condition_label = state.block("while.cond");
                let body_label = state.block("while.body");
                let exit_label = state.block("while.exit");
                state.body.push_str(&format!(
                    "  br label %{condition_label}\n\n{condition_label}:\n"
                ));
                let condition =
                    emit_project_expression(&while_expression.condition, state, module, modules)?;
                state.body.push_str(&format!(
                    "  br i1 {}, label %{body_label}, label %{exit_label}\n\n{body_label}:\n",
                    condition.0
                ));
                let _ = emit_project_block(&while_expression.body, state, module, modules)?;
                state
                    .body
                    .push_str(&format!("  br label %{condition_label}\n\n{exit_label}:\n"));
                ("0".to_owned(), LlvmValueType::Void)
            }
        };
    }
    state.storage = storage;
    Ok(result)
}

fn emit_project_assignment(
    assignment: &ScalarAssignment,
    state: &mut EmitState,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<(String, LlvmValueType), String> {
    let value = emit_project_expression(&assignment.value, state, module, modules)?;
    let (slot, value_type) = state
        .storage
        .get(&assignment.target)
        .cloned()
        .ok_or_else(|| format!("unknown LLVM storage {}", assignment.target))?;
    let old = state.value();
    state.body.push_str(&format!(
        "  {old} = load {} , ptr {}\n",
        llvm_type(value_type),
        slot
    ));
    state.body.push_str(&format!(
        "  store {} {}, ptr {}\n",
        llvm_type(value_type),
        value.0,
        slot
    ));
    Ok((old, value_type))
}

fn emit_project_expression(
    expression: &ScalarExpression,
    state: &mut EmitState,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<(String, LlvmValueType), String> {
    match expression {
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            ..
        } => {
            let arguments = arguments
                .iter()
                .map(|argument| emit_project_expression(argument, state, module, modules))
                .collect::<Result<Vec<_>, _>>()?;
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
            let result = state.value();
            let args = arguments
                .iter()
                .map(|value| format!("{} {}", llvm_type(value.1), value.0))
                .collect::<Vec<_>>()
                .join(", ");
            state.body.push_str(&format!(
                "  {result} = call i32 @{}({args})\n",
                project_function_name(&target.source, name)
            ));
            Ok((result, LlvmValueType::I32))
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            let left = emit_project_expression(left, state, module, modules)?;
            let right = emit_project_expression(right, state, module, modules)?;
            let result = state.value();
            let instruction = match operator {
                BinaryOperator::Add => format!("add i32 {}", operands(&left, &right)),
                BinaryOperator::Subtract => format!("sub i32 {}", operands(&left, &right)),
                BinaryOperator::Multiply => format!("mul i32 {}", operands(&left, &right)),
                BinaryOperator::Divide => format!("sdiv i32 {}", operands(&left, &right)),
                BinaryOperator::Remainder => format!("srem i32 {}", operands(&left, &right)),
                BinaryOperator::Equal => comparison("eq", &left, &right),
                BinaryOperator::NotEqual => comparison("ne", &left, &right),
                BinaryOperator::Less => comparison("slt", &left, &right),
                BinaryOperator::LessEqual => comparison("sle", &left, &right),
                BinaryOperator::Greater => comparison("sgt", &left, &right),
                BinaryOperator::GreaterEqual => comparison("sge", &left, &right),
                BinaryOperator::And => format!("and i1 {}", operands(&left, &right)),
                BinaryOperator::Or => format!("or i1 {}", operands(&left, &right)),
            };
            let result_type = if matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::And
                    | BinaryOperator::Or
            ) {
                LlvmValueType::I1
            } else {
                LlvmValueType::I32
            };
            state
                .body
                .push_str(&format!("  {result} = {instruction}\n"));
            Ok((result, result_type))
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let condition = emit_project_expression(condition, state, module, modules)?;
            let then_label = state.block("if.then");
            let else_label = state.block("if.else");
            let merge_label = state.block("if.merge");
            state.body.push_str(&format!(
                "  br i1 {}, label %{then_label}, label %{else_label}\n\n{then_label}:\n",
                condition.0
            ));
            let then_value = emit_project_block(then_branch, state, module, modules)?;
            state
                .body
                .push_str(&format!("  br label %{merge_label}\n\n{else_label}:\n"));
            let else_value = emit_project_block(else_branch, state, module, modules)?;
            state
                .body
                .push_str(&format!("  br label %{merge_label}\n\n{merge_label}:\n"));
            let result = state.value();
            state.body.push_str(&format!(
                "  {result} = phi {} [ {}, %{} ], [ {}, %{} ]\n",
                llvm_type(then_value.1),
                then_value.0,
                then_label,
                else_value.0,
                else_label
            ));
            Ok((result, then_value.1))
        }
        ScalarExpression::Block(block) => emit_project_block(block, state, module, modules),
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .cloned()
            .map(|(slot, value_type)| {
                let result = state.value();
                state.body.push_str(&format!(
                    "  {result} = load {}, ptr {slot}\n",
                    llvm_type(value_type)
                ));
                (result, value_type)
            })
            .or_else(|| {
                state.values.get(name).cloned().or_else(|| {
                    state
                        .names
                        .get(name)
                        .map(|value| (value.clone(), LlvmValueType::I32))
                })
            })
            .ok_or_else(|| format!("unknown LLVM value {name}")),
        ScalarExpression::Integer { value, .. } => Ok((value.to_string(), LlvmValueType::I32)),
        ScalarExpression::Boolean { value, .. } => {
            Ok((if *value { "1" } else { "0" }.to_owned(), LlvmValueType::I1))
        }
    }
}

fn project_function_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    let identity = format!("{}__{}__{}", source.package, source.path, name);
    identity
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

fn emit_function(function: &ScalarFunction) -> Result<LlvmFunction, String> {
    let ScalarType::Callable { result, parameters } = &function.signature else {
        return Err(format!(
            "function {} has no callable signature",
            function.name
        ));
    };
    let mut names = BTreeMap::new();
    let mut llvm_parameters = Vec::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let name = function.parameters[index].clone();
        let value_type = value_type(parameter)?;
        names.insert(name.clone(), format!("%{name}"));
        llvm_parameters.push((name, value_type));
    }
    let mut state = EmitState::new(names, BTreeMap::new());
    state.initialize_parameters(&llvm_parameters);
    let value = emit_block(&function.body, &mut state)?;
    append_return(&mut state, value, value_type(result)?)?;
    Ok(LlvmFunction {
        name: function.name.clone(),
        result: value_type(result)?,
        parameters: llvm_parameters,
        body: state.body,
    })
}

fn emit_main(items: &[ScalarItem]) -> Result<LlvmFunction, String> {
    let mut state = EmitState::new(BTreeMap::new(), BTreeMap::new());
    for item in items {
        if let ScalarItem::Binding(binding) = item {
            let value = emit_expression(&binding.value, &mut state)?;
            state.values.insert(binding.name.clone(), value);
        }
    }
    state.body.push_str("  ret i32 0\n");
    Ok(LlvmFunction {
        name: "main".to_owned(),
        result: LlvmValueType::I32,
        parameters: Vec::new(),
        body: state.body,
    })
}

struct EmitState {
    names: BTreeMap<String, String>,
    values: BTreeMap<String, (String, LlvmValueType)>,
    storage: BTreeMap<String, (String, LlvmValueType)>,
    body: String,
    next_value: usize,
    next_block: usize,
}

impl EmitState {
    fn new(
        names: BTreeMap<String, String>,
        values: BTreeMap<String, (String, LlvmValueType)>,
    ) -> Self {
        Self {
            names,
            values,
            storage: BTreeMap::new(),
            body: "entry:\n".to_owned(),
            next_value: 0,
            next_block: 0,
        }
    }

    fn value(&mut self) -> String {
        let value = format!("%v{}", self.next_value);
        self.next_value += 1;
        value
    }

    fn allocate(&mut self, name: &str, value_type: LlvmValueType) -> String {
        let slot = self.value();
        self.body
            .push_str(&format!("  {slot} = alloca {}\n", llvm_type(value_type)));
        self.storage
            .insert(name.to_owned(), (slot.clone(), value_type));
        slot
    }

    fn initialize_parameters(&mut self, parameters: &[(String, LlvmValueType)]) {
        for (name, value_type) in parameters {
            let slot = self.allocate(name, *value_type);
            self.body.push_str(&format!(
                "  store {} %{}, ptr {}\n",
                llvm_type(*value_type),
                name,
                slot
            ));
        }
    }

    fn block(&mut self, prefix: &str) -> String {
        let block = format!("{}.{}", prefix, self.next_block);
        self.next_block += 1;
        block
    }
}

fn append_return(
    state: &mut EmitState,
    value: (String, LlvmValueType),
    result: LlvmValueType,
) -> Result<(), String> {
    if result == LlvmValueType::Void {
        state.body.push_str("  ret void\n");
        return Ok(());
    }
    if value.1 != result {
        return Err(format!(
            "LLVM block result {} does not match function result {}",
            llvm_type(value.1),
            llvm_type(result)
        ));
    }
    state
        .body
        .push_str(&format!("  ret {} {}\n", llvm_type(result), value.0));
    Ok(())
}

fn emit_block(
    block: &ScalarBlock,
    state: &mut EmitState,
) -> Result<(String, LlvmValueType), String> {
    let storage = state.storage.clone();
    let mut result = ("0".to_owned(), LlvmValueType::Void);
    for item in &block.items {
        result = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let value = emit_expression(&binding.value, state)?;
                let slot = state.allocate(&binding.name, value_type(&binding.declared_type)?);
                state.body.push_str(&format!(
                    "  store {} {}, ptr {}\n",
                    llvm_type(value.1),
                    value.0,
                    slot
                ));
                ("0".to_owned(), LlvmValueType::Void)
            }
            ScalarBlockItem::Expression(expression) => emit_expression(expression, state)?,
            ScalarBlockItem::Assignment(assignment) => emit_assignment(assignment, state)?,
            ScalarBlockItem::While(while_expression) => {
                let condition_label = state.block("while.cond");
                let body_label = state.block("while.body");
                let exit_label = state.block("while.exit");
                state.body.push_str(&format!(
                    "  br label %{condition_label}\n\n{condition_label}:\n"
                ));
                let condition = emit_expression(&while_expression.condition, state)?;
                state.body.push_str(&format!(
                    "  br i1 {}, label %{body_label}, label %{exit_label}\n\n{body_label}:\n",
                    condition.0
                ));
                let _ = emit_block(&while_expression.body, state)?;
                state
                    .body
                    .push_str(&format!("  br label %{condition_label}\n\n{exit_label}:\n"));
                ("0".to_owned(), LlvmValueType::Void)
            }
        };
    }
    state.storage = storage;
    Ok(result)
}

fn emit_assignment(
    assignment: &ScalarAssignment,
    state: &mut EmitState,
) -> Result<(String, LlvmValueType), String> {
    let value = emit_expression(&assignment.value, state)?;
    let (slot, value_type) = state
        .storage
        .get(&assignment.target)
        .cloned()
        .ok_or_else(|| format!("unknown LLVM storage {}", assignment.target))?;
    let old = state.value();
    state.body.push_str(&format!(
        "  {old} = load {}, ptr {slot}\n",
        llvm_type(value_type)
    ));
    state.body.push_str(&format!(
        "  store {} {}, ptr {}\n",
        llvm_type(value_type),
        value.0,
        slot
    ));
    Ok((old, value_type))
}

fn emit_expression(
    expression: &ScalarExpression,
    state: &mut EmitState,
) -> Result<(String, LlvmValueType), String> {
    match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .cloned()
            .map(|(slot, value_type)| {
                let result = state.value();
                state.body.push_str(&format!(
                    "  {result} = load {}, ptr {slot}\n",
                    llvm_type(value_type)
                ));
                (result, value_type)
            })
            .or_else(|| {
                state.values.get(name).cloned().or_else(|| {
                    state
                        .names
                        .get(name)
                        .map(|value| (value.clone(), LlvmValueType::I32))
                })
            })
            .ok_or_else(|| format!("unknown LLVM value {name}")),
        ScalarExpression::Integer { value, .. } => Ok((value.to_string(), LlvmValueType::I32)),
        ScalarExpression::Boolean { value, .. } => {
            Ok((if *value { "1" } else { "0" }.to_owned(), LlvmValueType::I1))
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => {
            let left = emit_expression(left, state)?;
            let right = emit_expression(right, state)?;
            let result = state.value();
            let instruction = match operator {
                BinaryOperator::Add => format!("add i32 {}", operands(&left, &right)),
                BinaryOperator::Subtract => format!("sub i32 {}", operands(&left, &right)),
                BinaryOperator::Multiply => format!("mul i32 {}", operands(&left, &right)),
                BinaryOperator::Divide => format!("sdiv i32 {}", operands(&left, &right)),
                BinaryOperator::Remainder => format!("srem i32 {}", operands(&left, &right)),
                BinaryOperator::Equal => comparison("eq", &left, &right),
                BinaryOperator::NotEqual => comparison("ne", &left, &right),
                BinaryOperator::Less => comparison("slt", &left, &right),
                BinaryOperator::LessEqual => comparison("sle", &left, &right),
                BinaryOperator::Greater => comparison("sgt", &left, &right),
                BinaryOperator::GreaterEqual => comparison("sge", &left, &right),
                BinaryOperator::And => format!("and i1 {}", operands(&left, &right)),
                BinaryOperator::Or => format!("or i1 {}", operands(&left, &right)),
            };
            let result_type = if matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::And
                    | BinaryOperator::Or
            ) {
                LlvmValueType::I1
            } else {
                LlvmValueType::I32
            };
            state
                .body
                .push_str(&format!("  {result} = {instruction}\n"));
            Ok((result, result_type))
        }
        ScalarExpression::Call {
            name, arguments, ..
        } => {
            let arguments = arguments
                .iter()
                .map(|argument| emit_expression(argument, state))
                .collect::<Result<Vec<_>, _>>()?;
            let result = state.value();
            let args = arguments
                .iter()
                .map(|value| format!("{} {}", llvm_type(value.1), value.0))
                .collect::<Vec<_>>()
                .join(", ");
            state
                .body
                .push_str(&format!("  {result} = call i32 @{name}({args})\n"));
            Ok((result, LlvmValueType::I32))
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let condition = emit_expression(condition, state)?;
            let then_label = state.block("if.then");
            let else_label = state.block("if.else");
            let merge_label = state.block("if.merge");
            state.body.push_str(&format!(
                "  br i1 {}, label %{then_label}, label %{else_label}\n\n{then_label}:\n",
                condition.0
            ));
            let then_value = emit_block(then_branch, state)?;
            state
                .body
                .push_str(&format!("  br label %{merge_label}\n\n{else_label}:\n"));
            let else_value = emit_block(else_branch, state)?;
            state
                .body
                .push_str(&format!("  br label %{merge_label}\n\n{merge_label}:\n"));
            let result = state.value();
            state.body.push_str(&format!(
                "  {result} = phi {} [ {}, %{} ], [ {}, %{} ]\n",
                llvm_type(then_value.1),
                then_value.0,
                then_label,
                else_value.0,
                else_label
            ));
            Ok((result, then_value.1))
        }
        ScalarExpression::Block(block) => emit_block(block, state),
    }
}

fn value_type(value: &ScalarType) -> Result<LlvmValueType, String> {
    match value {
        ScalarType::Unit => Ok(LlvmValueType::Void),
        ScalarType::Bool => Ok(LlvmValueType::I1),
        ScalarType::I32 => Ok(LlvmValueType::I32),
        ScalarType::Callable { .. } | ScalarType::Named(_) => {
            Err("unsupported LLVM scalar type".to_owned())
        }
    }
}

fn llvm_type(value: LlvmValueType) -> &'static str {
    match value {
        LlvmValueType::Void => "void",
        LlvmValueType::I1 => "i1",
        LlvmValueType::I32 => "i32",
    }
}
fn parameters_text(parameters: &[(String, LlvmValueType)]) -> String {
    parameters
        .iter()
        .map(|(name, value)| format!("{} %{}", llvm_type(*value), name))
        .collect::<Vec<_>>()
        .join(", ")
}
fn operands(left: &(String, LlvmValueType), right: &(String, LlvmValueType)) -> String {
    format!("{}, {}", left.0, right.0)
}
fn comparison(
    operator: &str,
    left: &(String, LlvmValueType),
    right: &(String, LlvmValueType),
) -> String {
    format!("icmp {operator} i32 {}", operands(left, right))
}

#[cfg(test)]
mod tests {
    use super::{emit_scalar_llvm, emit_scalar_project_llvm};
    use crate::{
        derive_scalar_program, parse_source, validate_scalar_project, ScalarItem, ScalarModule,
        ScalarNamespaceBinding, ScalarProject,
    };
    use std::fs;
    use std::process::Command;
    use wosy_syntax::SourceIdentity;

    #[test]
    fn emits_clang_buildable_scalar_module() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(source, "%%start\ni32(i32, i32) add = fn(left, right) {\n\tleft + right\n};\ni32 seed = 20;\ni32 increment = 22;\nbool choose_sum = true;\ni32 result = if (choose_sum) {\n\tadd(seed, increment)\n} else {\n\t0\n};\n%%end".to_owned(), &[]);
        let validation = derive_scalar_program(&parsed.result);
        let partition = emit_scalar_llvm(&validation).expect("valid scalar LLVM");
        let directory = std::env::temp_dir().join(format!("wosy-llvm-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("temporary directory");
        let input = directory.join("partition.ll");
        let output = directory.join("main");
        fs::write(&input, partition.to_text()).expect("LLVM text");
        let status = Command::new("/usr/bin/clang")
            .args(["-x", "ir"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .status()
            .expect("clang");
        assert!(status.success());
        assert!(output.is_file());
        let result = Command::new(&output).status().expect("native artifact");
        assert_eq!(result.code(), Some(0));
        fs::remove_dir_all(directory).expect("temporary directory cleanup");
    }

    #[test]
    fn emits_typed_direct_block_while_in_source_order() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source,
            "%%start\ni32(i32) loop = fn(start) {\n\ti32 value = start;\n\twhile (value < 3) {\n\t\tvalue = value + 1;\n\t}\n\tvalue\n};\n\ni32 result = loop(0);\n%%end"
                .to_owned(),
            &[],
        );
        let validation = derive_scalar_program(&parsed.result);
        let partition = emit_scalar_llvm(&validation).expect("valid scalar LLVM");
        let function = partition
            .functions
            .iter()
            .find(|function| function.name == "loop")
            .expect("loop function");
        let body = &function.body;
        let condition = body.find("while.cond.0:").expect("condition block");
        let loop_body = body.find("while.body.1:").expect("body block");
        let exit = body.find("while.exit.2:").expect("exit block");
        assert!(condition < loop_body && loop_body < exit);
        assert!(body.contains("alloca i32"));
        assert!(body.contains("load i32, ptr"));
        assert!(body.contains("store i32"));
        assert!(body.contains("br label %while.cond.0"));
        assert!(body.matches("br label %while.cond.0").count() >= 2);

        let directory =
            std::env::temp_dir().join(format!("wosy-llvm-while-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("temporary directory");
        let input = directory.join("partition.ll");
        let output = directory.join("main");
        fs::write(&input, partition.to_text()).expect("LLVM text");
        let status = Command::new("/usr/bin/clang")
            .args(["-x", "ir"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .status()
            .expect("clang");
        assert!(status.success());
        assert_eq!(
            Command::new(&output)
                .status()
                .expect("native artifact")
                .code(),
            Some(0)
        );
        fs::remove_dir_all(directory).expect("temporary directory cleanup");
    }

    #[test]
    fn emits_one_ordered_project_with_qualified_symbols_and_declarations() {
        let math_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/math.w".into(),
            "r1".into(),
        );
        let main_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let math = derive_scalar_program(
            &parse_source(
                math_source.clone(),
                "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\ni32 seed = 20;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let main = derive_scalar_program(
            &parse_source(
                main_source.clone(),
                "%%start\nmath = namespace package \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".into(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source.clone(), math.items, Vec::new()),
            ],
            vec![main_source, math_source.clone(), math_source],
        ));
        let partition = emit_scalar_project_llvm(&validation).expect("valid scalar LLVM project");
        assert_eq!(partition.declarations.len(), 1);
        assert_eq!(partition.functions.len(), 2);
        assert!(partition
            .declarations
            .iter()
            .any(|function| function.name == "package__src_math_w__add"));
        assert!(partition
            .functions
            .iter()
            .any(|function| function.body.contains("@package__src_math_w__add")));
        assert_eq!(
            partition
                .functions
                .iter()
                .find(|function| function.name == "main")
                .expect("main")
                .body
                .matches("@package__src_math_w__add")
                .count(),
            1
        );
    }
}
