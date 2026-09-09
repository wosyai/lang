use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;
use rowan::NodeOrToken;
use serde::{Deserialize, Serialize};
use wosy_syntax::{
    ByteSpan, CanonicalCstRoot, CstNode, CstToken, ParseResult, SourceIdentity, SourceSpan,
    SyntaxKind,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarType {
    Unit,
    Bool,
    I32,
    Callable {
        result: Box<ScalarType>,
        parameters: Vec<ScalarType>,
    },
    Named(String),
    Error,
}

fn is_error_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Error)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarExpression {
    Name {
        name: String,
        span: ByteSpan,
    },
    Member {
        receiver: String,
        name: String,
        receiver_span: ByteSpan,
        name_span: ByteSpan,
        span: ByteSpan,
    },
    Integer {
        value: BigInt,
        span: ByteSpan,
    },
    InvalidInteger {
        span: ByteSpan,
    },
    Boolean {
        value: bool,
        span: ByteSpan,
    },
    Binary {
        operator: BinaryOperator,
        left: Box<ScalarExpression>,
        right: Box<ScalarExpression>,
        span: ByteSpan,
    },
    Call {
        receiver: Option<String>,
        name: String,
        receiver_span: Option<ByteSpan>,
        name_span: ByteSpan,
        arguments: Vec<ScalarExpression>,
        span: ByteSpan,
    },
    If {
        condition: Box<ScalarExpression>,
        then_branch: ScalarBlock,
        else_branch: ScalarBlock,
        span: ByteSpan,
    },
    Block(ScalarBlock),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarBlock {
    pub items: Vec<ScalarBlockItem>,
    pub terminated_items: Vec<bool>,
    pub expressions: Vec<ScalarExpression>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarBlockItem {
    LocalBinding(ScalarBinding),
    Expression(ScalarExpression),
    Assignment(ScalarAssignment),
    While(ScalarWhile),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAssignment {
    pub receiver: Option<String>,
    pub target: String,
    pub receiver_span: Option<ByteSpan>,
    pub target_span: ByteSpan,
    pub value: ScalarExpression,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarWhile {
    pub condition: ScalarExpression,
    pub body: ScalarBlock,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarBinding {
    pub name: String,
    pub declared_type: ScalarType,
    pub value: ScalarExpression,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarNamespace {
    pub binding: String,
    pub package: String,
    pub path: String,
    pub binding_span: ByteSpan,
    pub package_span: ByteSpan,
    pub path_span: ByteSpan,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarFunction {
    pub name: String,
    pub signature: ScalarType,
    pub parameters: Vec<String>,
    pub parameter_spans: Vec<ByteSpan>,
    pub body: ScalarBlock,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarItem {
    Namespace(ScalarNamespace),
    Binding(ScalarBinding),
    Function(ScalarFunction),
    Executable(ScalarBlockItem),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProgram {
    pub source: SourceIdentity,
    pub items: Vec<ScalarItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarNamespaceBinding {
    pub binding: String,
    pub target: SourceIdentity,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarInitializationNode {
    pub item_index: usize,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarModule {
    pub source: SourceIdentity,
    pub items: Vec<ScalarItem>,
    pub namespace_bindings: Vec<ScalarNamespaceBinding>,
    pub members: BTreeMap<String, ScalarType>,
    pub initialization_nodes: Vec<ScalarInitializationNode>,
}

impl ScalarModule {
    pub fn new(
        source: SourceIdentity,
        items: Vec<ScalarItem>,
        namespace_bindings: Vec<ScalarNamespaceBinding>,
    ) -> Self {
        let mut members = BTreeMap::new();
        let mut initialization_nodes = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            let (name, ty, span) = match item {
                ScalarItem::Namespace(_) => continue,
                ScalarItem::Binding(binding) => {
                    (&binding.name, &binding.declared_type, binding.span)
                }
                ScalarItem::Function(function) => {
                    (&function.name, &function.signature, function.span)
                }
                ScalarItem::Executable(_) => continue,
            };
            members.entry(name.clone()).or_insert_with(|| ty.clone());
            initialization_nodes.push(ScalarInitializationNode { item_index, span });
        }
        Self {
            source,
            items,
            namespace_bindings,
            members,
            initialization_nodes,
        }
    }
}

impl ScalarProject {
    pub fn new(modules: Vec<ScalarModule>, initialization_order: Vec<SourceIdentity>) -> Self {
        let mut unique_initialization_order = Vec::new();
        for source in initialization_order {
            if !unique_initialization_order.contains(&source) {
                unique_initialization_order.push(source);
            }
        }
        Self {
            modules,
            initialization_order: unique_initialization_order,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProject {
    pub modules: Vec<ScalarModule>,
    pub initialization_order: Vec<SourceIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProjectValidation {
    pub project: ScalarProject,
    pub diagnostics: Vec<super::Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarValidation {
    pub program: ScalarProgram,
    pub diagnostics: Vec<super::Diagnostic>,
}

pub fn derive_scalar_program(parse: &ParseResult) -> ScalarValidation {
    derive_scalar_program_from_cst(parse.canonical_cst())
}

pub fn derive_scalar_program_from_cst(canonical: &CanonicalCstRoot) -> ScalarValidation {
    let mut items = Vec::new();
    let source_root = canonical
        .root
        .children()
        .find(|node| node.kind() == SyntaxKind::Root)
        .expect("source root");
    for node in source_root.children() {
        match node.kind() {
            SyntaxKind::NamespaceDecl => items.push(ScalarItem::Namespace(derive_namespace(&node))),
            SyntaxKind::BindingDecl => items.push(ScalarItem::Binding(derive_binding(&node))),
            SyntaxKind::FunctionDecl => items.push(ScalarItem::Function(derive_function(&node))),
            SyntaxKind::Item => {
                for item in node.children() {
                    match item.kind() {
                        SyntaxKind::NamespaceDecl => {
                            items.push(ScalarItem::Namespace(derive_namespace(&item)))
                        }
                        SyntaxKind::BindingDecl => {
                            items.push(ScalarItem::Binding(derive_binding(&item)))
                        }
                        SyntaxKind::FunctionDecl => {
                            items.push(ScalarItem::Function(derive_function(&item)))
                        }
                        SyntaxKind::TopLevelItem => {
                            items.push(ScalarItem::Executable(derive_executable_item(&item)))
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let program = ScalarProgram {
        source: canonical.source.clone(),
        items,
    };
    let diagnostics = validate(&program);
    ScalarValidation {
        program,
        diagnostics,
    }
}

pub fn validate_scalar_project(project: ScalarProject) -> ScalarProjectValidation {
    let mut diagnostics = Vec::new();
    for module in &project.modules {
        let mut declarations = BTreeMap::new();
        let mut declaration_names = BTreeSet::new();
        let mut folded_declarations = BTreeMap::new();
        for item in &module.items {
            let (name, span, ty) = match item {
                ScalarItem::Namespace(namespace) => (&namespace.binding, namespace.span, None),
                ScalarItem::Binding(binding) => {
                    (&binding.name, binding.span, Some(&binding.declared_type))
                }
                ScalarItem::Function(function) => {
                    (&function.name, function.span, Some(&function.signature))
                }
                ScalarItem::Executable(_) => continue,
            };
            if declare_module_name(
                module,
                name,
                span,
                &mut declaration_names,
                &mut folded_declarations,
                &mut diagnostics,
            ) {
                if let Some(ty) = ty {
                    declarations.insert(name.clone(), ty.clone());
                    validate_module_type(module, ty, span, &mut diagnostics);
                }
            }
        }
        for item in &module.items {
            match item {
                ScalarItem::Namespace(_) => {}
                ScalarItem::Binding(binding) => {
                    let actual = expression_type_in_module(
                        &binding.value,
                        &declarations,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        module,
                        &project.modules,
                        &mut diagnostics,
                    );
                    expect_module_type(
                        module,
                        &binding.declared_type,
                        &actual,
                        binding.span,
                        &mut diagnostics,
                    );
                }
                ScalarItem::Function(function) => {
                    let ScalarType::Callable { result, parameters } = &function.signature else {
                        continue;
                    };
                    if function.parameters.len() != parameters.len() {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0004",
                            "function parameter arity does not match its type",
                            function.span,
                        ));
                    }
                    let mut scope = declarations.clone();
                    let mut visible_names = declaration_names.clone();
                    let mut folded_names = folded_declarations.clone();
                    for (index, name) in function.parameters.iter().enumerate() {
                        if index < parameters.len()
                            && declare_module_name(
                                module,
                                name,
                                function.span,
                                &mut visible_names,
                                &mut folded_names,
                                &mut diagnostics,
                            )
                        {
                            scope.insert(name.clone(), parameters[index].clone());
                        }
                    }
                    let actual = block_type_in_module(
                        &function.body,
                        &scope,
                        &visible_names,
                        &folded_names,
                        module,
                        &project.modules,
                        &mut diagnostics,
                    );
                    expect_module_type(
                        module,
                        result,
                        &actual,
                        function.body.span,
                        &mut diagnostics,
                    );
                }
                ScalarItem::Executable(executable) => {
                    let mut scope = declarations.clone();
                    let mut visible_names = declaration_names.clone();
                    let mut folded_names = folded_declarations.clone();
                    let _ = block_item_type_in_module(
                        executable,
                        &mut scope,
                        &mut visible_names,
                        &mut folded_names,
                        module,
                        &project.modules,
                        &mut diagnostics,
                    );
                }
            }
        }
        for span in unused_binding_spans(&module.items) {
            diagnostics.push(module_diagnostic(
                module,
                "B0009",
                "local binding or parameter has no required static use",
                span,
            ));
        }
    }
    ScalarProjectValidation {
        project,
        diagnostics,
    }
}

fn validate_module_type(
    module: &ScalarModule,
    ty: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named(_) => diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "unknown scalar type",
            span,
        )),
        ScalarType::Callable { result, parameters } => {
            validate_module_type(module, result, span, diagnostics);
            for parameter in parameters {
                validate_module_type(module, parameter, span, diagnostics);
            }
        }
        ScalarType::Unit | ScalarType::Bool | ScalarType::I32 => {}
        ScalarType::Error => diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "invalid scalar type",
            span,
        )),
    }
}

fn block_type_in_module(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = block_item_type_in_module(
            item,
            &mut scope,
            &mut visible_names,
            &mut folded_names,
            module,
            modules,
            diagnostics,
        );
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

fn block_item_type_in_module(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let actual = expression_type_in_module(
                &binding.value,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
            );
            expect_module_type(
                module,
                &binding.declared_type,
                &actual,
                binding.span,
                diagnostics,
            );
            if declare_module_name(
                module,
                &binding.name,
                binding.span,
                visible_names,
                folded_names,
                diagnostics,
            ) {
                scope.insert(binding.name.clone(), binding.declared_type.clone());
            }
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => expression_type_in_module(
            expression,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
        ),
        ScalarBlockItem::Assignment(assignment) => assignment_type_in_module(
            assignment,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
        ),
        ScalarBlockItem::While(while_expression) => while_type_in_module(
            while_expression,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
        ),
    }
}

fn assignment_type_in_module(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let actual = expression_type_in_module(
        &assignment.value,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
    );
    let expected = match &assignment.receiver {
        None => {
            if is_const_binding_name(&assignment.target)
                && (scope.contains_key(&assignment.target)
                    || module.items.iter().any(|item| {
                        matches!(
                            item,
                            ScalarItem::Namespace(namespace)
                                if namespace.binding == assignment.target
                        )
                    }))
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    assignment.target_span,
                ));
            }
            scope.get(&assignment.target)
        }
        Some(receiver) => {
            let Some(namespace) = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown assignment target",
                    assignment
                        .receiver_span
                        .expect("qualified assignment receiver span"),
                ));
                return ScalarType::Error;
            };
            let Some(target) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown assignment target",
                    assignment
                        .receiver_span
                        .expect("qualified assignment receiver span"),
                ));
                return ScalarType::Error;
            };
            let Some(expected) = target.members.get(&assignment.target) else {
                diagnostics.push(unknown_member_diagnostic(
                    module,
                    assignment.target_span,
                    &target.source,
                ));
                return ScalarType::Error;
            };
            if is_const_binding_name(&assignment.target) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    assignment.target_span,
                ));
            }
            Some(expected)
        }
    };
    let Some(expected) = expected else {
        diagnostics.push(module_diagnostic(
            module,
            "B0001",
            "unknown assignment target",
            assignment.target_span,
        ));
        return ScalarType::Error;
    };
    expect_module_type(module, expected, &actual, assignment.span, diagnostics);
    if is_error_type(&actual) {
        ScalarType::Error
    } else {
        expected.clone()
    }
}

fn while_type_in_module(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let condition = expression_type_in_module(
        &while_expression.condition,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
    );
    if !is_error_type(&condition) && condition != ScalarType::Bool {
        diagnostics.push(module_diagnostic(
            module,
            "B0005",
            "while expression requires bool",
            while_expression.span,
        ));
    }
    let _ = block_type_in_module(
        &while_expression.body,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
    );
    ScalarType::Unit
}

fn expression_type_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match expression {
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarExpression::Member {
            receiver,
            name,
            name_span,
            span,
            ..
        } => {
            let Some(namespace) = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
            else {
                diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
                return ScalarType::Error;
            };
            let Some(target) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            else {
                diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
                return ScalarType::Error;
            };
            let Some(member) = target.members.get(name) else {
                diagnostics.push(unknown_member_diagnostic(
                    module,
                    *name_span,
                    &target.source,
                ));
                return ScalarType::Error;
            };
            member.clone()
        }
        ScalarExpression::Integer { value, span } => {
            validate_integer_range(module, value, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::InvalidInteger { span } => {
            invalid_integer_diagnostic(module, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let left_type = expression_type_in_module(
                left,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
            );
            let right_type = expression_type_in_module(
                right,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
            );
            if is_error_type(&left_type) || is_error_type(&right_type) {
                return ScalarType::Error;
            }
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            let boolean = matches!(operator, BinaryOperator::And | BinaryOperator::Or);
            if boolean {
                expect_module_type(module, &ScalarType::Bool, &left_type, *span, diagnostics);
                expect_module_type(module, &ScalarType::Bool, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if comparison {
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else {
                expect_module_type(module, &ScalarType::I32, &left_type, *span, diagnostics);
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                ScalarType::I32
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            name_span,
            arguments,
            span,
            ..
        } => {
            let (callable, target) = match receiver {
                None => (scope.get(name), module),
                Some(binding) => {
                    let namespace = module
                        .namespace_bindings
                        .iter()
                        .find(|namespace| namespace.binding == *binding);
                    let Some(namespace) = namespace else {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0001",
                            "unknown callable name",
                            *span,
                        ));
                        return ScalarType::Error;
                    };
                    let Some(target) = modules
                        .iter()
                        .find(|module| module.source == namespace.target)
                    else {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0001",
                            "unknown callable name",
                            *span,
                        ));
                        return ScalarType::Error;
                    };
                    let Some(callable) = target.members.get(name) else {
                        diagnostics.push(unknown_member_diagnostic(
                            module,
                            *name_span,
                            &target.source,
                        ));
                        return ScalarType::Error;
                    };
                    (Some(callable), target)
                }
            };
            let Some(callable) = callable else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown callable name",
                    *span,
                ));
                return ScalarType::Error;
            };
            let ScalarType::Callable { result, parameters } = callable else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown callable name",
                    *span,
                ));
                return ScalarType::Error;
            };
            if arguments.len() != parameters.len() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            let mut error_argument = false;
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let actual = expression_type_in_module(
                    argument,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                );
                expect_module_type(target, parameter, &actual, *span, diagnostics);
                error_argument |= is_error_type(&actual);
            }
            if error_argument {
                ScalarType::Error
            } else {
                result.as_ref().clone()
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let condition_type = expression_type_in_module(
                condition,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
            );
            expect_module_type(
                module,
                &ScalarType::Bool,
                &condition_type,
                *span,
                diagnostics,
            );
            let then_type = block_type_in_module(
                then_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
            );
            let else_type = block_type_in_module(
                else_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
            );
            if !is_error_type(&condition_type)
                && !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && then_type != else_type
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            if is_error_type(&condition_type) || is_error_type(&then_type) {
                ScalarType::Error
            } else if is_error_type(&else_type) {
                ScalarType::Error
            } else {
                then_type
            }
        }
        ScalarExpression::Block(block) => block_type_in_module(
            block,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
        ),
    }
}

fn expect_module_type(
    module: &ScalarModule,
    expected: &ScalarType,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(expected) && !is_error_type(actual) && expected != actual {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
}

fn module_diagnostic(
    module: &ScalarModule,
    code: &str,
    message: &str,
    span: ByteSpan,
) -> super::Diagnostic {
    super::Diagnostic {
        code: code.to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(module.source.clone(), span),
            message: message.to_owned(),
        }],
        notes: Vec::new(),
    }
}

fn fold_name(name: &str) -> String {
    name.bytes()
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn declare_module_name(
    module: &ScalarModule,
    name: &str,
    span: ByteSpan,
    names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    if !names.insert(name.to_owned()) {
        diagnostics.push(module_diagnostic(
            module,
            "B0002",
            "duplicate declaration",
            span,
        ));
        return false;
    }
    let folded = fold_name(name);
    if let Some((first_name, first_span)) = folded_names.get(&folded) {
        if first_name != name {
            names.remove(name);
            diagnostics.push(module_collision_diagnostic(module, span, *first_span));
            return false;
        }
    } else {
        folded_names.insert(folded, (name.to_owned(), span));
    }
    true
}

fn module_collision_diagnostic(
    module: &ScalarModule,
    span: ByteSpan,
    first_span: ByteSpan,
) -> super::Diagnostic {
    let mut diagnostic = module_diagnostic(
        module,
        "B0008",
        "declaration collides with an existing name under ASCII case folding",
        span,
    );
    diagnostic.labels.push(super::DiagnosticLabel {
        kind: super::DiagnosticLabelKind::Secondary,
        span: SourceSpan::new(module.source.clone(), first_span),
        message: "first conflicting declaration".to_owned(),
    });
    diagnostic
}

fn unknown_member_diagnostic(
    importer: &ScalarModule,
    member_span: ByteSpan,
    target: &SourceIdentity,
) -> super::Diagnostic {
    super::Diagnostic {
        code: "M0002".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: "unknown module member".to_owned(),
        labels: vec![
            super::DiagnosticLabel {
                kind: super::DiagnosticLabelKind::Primary,
                span: SourceSpan::new(importer.source.clone(), member_span),
                message: "unknown module member".to_owned(),
            },
            super::DiagnosticLabel {
                kind: super::DiagnosticLabelKind::Secondary,
                span: SourceSpan::new(target.clone(), ByteSpan::new(0, 0)),
                message: "module resolved here".to_owned(),
            },
        ],
        notes: Vec::new(),
    }
}

fn derive_namespace(node: &CstNode) -> ScalarNamespace {
    let identifiers: Vec<CstToken> = node
        .children_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect();
    let path = direct_token(node, SyntaxKind::String).expect("namespace path");
    ScalarNamespace {
        binding: identifiers[0].text().to_owned(),
        package: identifiers[1].text().to_owned(),
        path: path.text().to_owned(),
        binding_span: token_span(&identifiers[0]),
        package_span: token_span(&identifiers[1]),
        path_span: token_span(&path),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_binding(node: &CstNode) -> ScalarBinding {
    let declared_type = derive_type(
        &direct_nodes(node)
            .into_iter()
            .find(|child| {
                matches!(
                    child.kind(),
                    SyntaxKind::TypeSpec | SyntaxKind::CallableType
                )
            })
            .expect("binding type"),
    );
    let name = direct_token(node, SyntaxKind::Identifier)
        .expect("binding name")
        .text()
        .to_owned();
    let value = derive_expression(
        &direct_nodes(node)
            .into_iter()
            .find(|child| child.kind() == SyntaxKind::Expression)
            .expect("binding value"),
    );
    ScalarBinding {
        name,
        declared_type,
        value,
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_function(node: &CstNode) -> ScalarFunction {
    let children = direct_nodes(node);
    let signature = derive_type(
        children
            .iter()
            .find(|child| child.kind() == SyntaxKind::CallableType)
            .expect("function signature"),
    );
    let name = direct_token(node, SyntaxKind::Identifier)
        .expect("function name")
        .text()
        .to_owned();
    let parameters = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Parameters)
        .map_or_else(Vec::new, |parameters| {
            parameters
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token.text().to_owned())
                    }
                    _ => None,
                })
                .collect()
        });
    let parameter_spans = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Parameters)
        .map_or_else(Vec::new, |parameters| {
            parameters
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token_span(&token))
                    }
                    _ => None,
                })
                .collect()
        });
    let body = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Block)
        .map(|node| derive_block(node))
        .expect("function block");
    ScalarFunction {
        name,
        signature,
        parameters,
        parameter_spans,
        body,
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_type(node: &CstNode) -> ScalarType {
    let actual = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::CallableType)
        .unwrap_or_else(|| node.clone());
    if actual.kind() == SyntaxKind::CallableType {
        let type_tokens: Vec<CstToken> = actual
            .children_with_tokens()
            .filter_map(|element| match element {
                NodeOrToken::Token(token) if token.kind() == SyntaxKind::TypeName => Some(token),
                _ => None,
            })
            .collect();
        let result = type_from_name(type_tokens[0].text());
        let parameters = type_tokens[1..]
            .iter()
            .map(|token| type_from_name(token.text()))
            .collect();
        ScalarType::Callable {
            result: Box::new(result),
            parameters,
        }
    } else {
        type_from_name(type_text(node).as_str())
    }
}

fn type_from_name(value: &str) -> ScalarType {
    match value {
        "unit" => ScalarType::Unit,
        "bool" => ScalarType::Bool,
        "i32" => ScalarType::I32,
        value => ScalarType::Named(value.to_owned()),
    }
}

fn derive_block(node: &CstNode) -> ScalarBlock {
    let items: Vec<ScalarBlockItem> = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::BlockItem)
        .map(|item| derive_block_item(&item))
        .collect();
    let terminated_items = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::BlockItem)
        .map(|item| {
            item.children_with_tokens().any(|element| {
                matches!(
                    element,
                    NodeOrToken::Token(token)
                        if token.kind() == SyntaxKind::Punctuation && token.text() == ";"
                )
            })
        })
        .collect();
    let expressions = items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    ScalarBlock {
        items,
        terminated_items,
        expressions,
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_block_item(node: &CstNode) -> ScalarBlockItem {
    let child = direct_nodes(node).into_iter().next().expect("block item");
    match child.kind() {
        SyntaxKind::LocalBinding => ScalarBlockItem::LocalBinding(derive_binding(&child)),
        SyntaxKind::While => ScalarBlockItem::While(derive_while(&child)),
        SyntaxKind::Expression => {
            let actual = direct_nodes(&child).into_iter().next().expect("expression");
            match actual.kind() {
                SyntaxKind::Assignment => ScalarBlockItem::Assignment(derive_assignment(&actual)),
                SyntaxKind::While => ScalarBlockItem::While(derive_while(&actual)),
                _ => ScalarBlockItem::Expression(derive_expression(&child)),
            }
        }
        _ => panic!("block item grammar"),
    }
}

fn derive_executable_item(node: &CstNode) -> ScalarBlockItem {
    let child = direct_nodes(node)
        .into_iter()
        .next()
        .expect("executable item");
    match child.kind() {
        SyntaxKind::Assignment => ScalarBlockItem::Assignment(derive_assignment(&child)),
        SyntaxKind::Expression => {
            let actual = direct_nodes(&child).into_iter().next().expect("expression");
            match actual.kind() {
                SyntaxKind::Assignment => ScalarBlockItem::Assignment(derive_assignment(&actual)),
                _ => ScalarBlockItem::Expression(derive_expression(&child)),
            }
        }
        _ => panic!("top-level item grammar"),
    }
}

fn derive_assignment(node: &CstNode) -> ScalarAssignment {
    let target_node = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::AssignmentTarget)
        .expect("assignment target node");
    let identifiers: Vec<CstToken> = target_node
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect();
    let target = identifiers.last().expect("assignment target name");
    let value = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::Expression)
        .map(|child| derive_expression(&child))
        .expect("assignment value");
    ScalarAssignment {
        receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
        target: target.text().to_owned(),
        receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
        target_span: token_span(&target),
        value,
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_while(node: &CstNode) -> ScalarWhile {
    let children = direct_nodes(node);
    ScalarWhile {
        condition: derive_expression(&children[0]),
        body: derive_block(&children[1]),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_expression(node: &CstNode) -> ScalarExpression {
    let actual = if node.kind() == SyntaxKind::Expression {
        direct_nodes(node)[0].clone()
    } else {
        node.clone()
    };
    match actual.kind() {
        SyntaxKind::IfExpr => {
            let children = direct_nodes(&actual);
            ScalarExpression::If {
                condition: Box::new(derive_expression(&children[0])),
                then_branch: derive_block(&children[1]),
                else_branch: derive_block(&children[2]),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::Binary => {
            let children = semantic_children(&actual);
            let mut value = derive_element(&children[0]);
            let mut index = 1;
            while index < children.len() {
                let NodeOrToken::Token(operator_token) = &children[index] else {
                    panic!("binary operator token")
                };
                let operator = operator(operator_token.text());
                let right = derive_element(&children[index + 1]);
                let span = ByteSpan::new(span_of(&value).start, span_of(&right).end);
                value = ScalarExpression::Binary {
                    operator,
                    left: Box::new(value),
                    right: Box::new(right),
                    span,
                };
                index += 2;
            }
            value
        }
        SyntaxKind::Call => {
            let qualified = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::QualifiedCall)
                .expect("qualified call");
            let identifiers: Vec<CstToken> = qualified
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            let receiver = identifiers
                .first()
                .filter(|_| identifiers.len() == 2)
                .map(|token| token.text().to_owned());
            let name_token = identifiers.last().expect("call member");
            let arguments = direct_nodes(&actual)
                .iter()
                .filter(|child| child.kind() == SyntaxKind::Expression)
                .map(derive_expression)
                .collect();
            ScalarExpression::Call {
                receiver,
                name: name_token.text().to_owned(),
                receiver_span: identifiers
                    .first()
                    .filter(|_| identifiers.len() == 2)
                    .map(token_span),
                name_span: token_span(name_token),
                arguments,
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::QualifiedMember => {
            let identifiers: Vec<CstToken> = actual
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            ScalarExpression::Member {
                receiver: identifiers[0].text().to_owned(),
                name: identifiers[1].text().to_owned(),
                receiver_span: token_span(&identifiers[0]),
                name_span: token_span(&identifiers[1]),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::Parenthesized => derive_expression(&direct_nodes(&actual)[0]),
        SyntaxKind::Expression => derive_element(&semantic_children(&actual)[0]),
        _ => derive_element(&semantic_children(&actual)[0]),
    }
}

fn direct_nodes(node: &CstNode) -> Vec<CstNode> {
    node.children().collect()
}

fn semantic_children(node: &CstNode) -> Vec<NodeOrToken<CstNode, CstToken>> {
    node.children_with_tokens()
        .filter(|element| match element {
            NodeOrToken::Node(_) => true,
            NodeOrToken::Token(token) => matches!(
                token.kind(),
                SyntaxKind::Identifier
                    | SyntaxKind::TypeName
                    | SyntaxKind::Integer
                    | SyntaxKind::Boolean
                    | SyntaxKind::Operator
            ),
        })
        .collect()
}

fn direct_token(node: &CstNode, kind: SyntaxKind) -> Option<CstToken> {
    node.children_with_tokens()
        .find_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == kind => Some(token),
            _ => None,
        })
}

fn type_text(node: &CstNode) -> String {
    direct_token(node, SyntaxKind::TypeName)
        .expect("type token")
        .text()
        .to_owned()
}

fn derive_element(element: &NodeOrToken<CstNode, CstToken>) -> ScalarExpression {
    match element {
        NodeOrToken::Node(node) => derive_expression(node),
        NodeOrToken::Token(token) => match token.kind() {
            SyntaxKind::Boolean => ScalarExpression::Boolean {
                value: token.text() == "true",
                span: token_span(token),
            },
            SyntaxKind::Identifier => ScalarExpression::Name {
                name: token.text().to_owned(),
                span: token_span(token),
            },
            SyntaxKind::Integer => {
                let span = token_span(token);
                match BigInt::parse_bytes(token.text().as_bytes(), 10) {
                    Some(value) => ScalarExpression::Integer { value, span },
                    None => ScalarExpression::InvalidInteger { span },
                }
            }
            _ => panic!("expression token"),
        },
    }
}

fn token_span(token: &CstToken) -> ByteSpan {
    let range = token.text_range();
    ByteSpan::new(u32::from(range.start()), u32::from(range.end()))
}

fn span_of(expression: &ScalarExpression) -> ByteSpan {
    match expression {
        ScalarExpression::Name { span, .. }
        | ScalarExpression::Member { span, .. }
        | ScalarExpression::Integer { span, .. }
        | ScalarExpression::InvalidInteger { span }
        | ScalarExpression::Boolean { span, .. }
        | ScalarExpression::Binary { span, .. }
        | ScalarExpression::Call { span, .. }
        | ScalarExpression::If { span, .. } => *span,
        ScalarExpression::Block(block) => block.span,
    }
}

fn operator(value: &str) -> BinaryOperator {
    match value {
        "+" => BinaryOperator::Add,
        "-" => BinaryOperator::Subtract,
        "*" => BinaryOperator::Multiply,
        "/" => BinaryOperator::Divide,
        "%" => BinaryOperator::Remainder,
        "==" => BinaryOperator::Equal,
        "!=" => BinaryOperator::NotEqual,
        "<" => BinaryOperator::Less,
        "<=" => BinaryOperator::LessEqual,
        ">" => BinaryOperator::Greater,
        ">=" => BinaryOperator::GreaterEqual,
        "&&" => BinaryOperator::And,
        "||" => BinaryOperator::Or,
        _ => panic!("operator grammar"),
    }
}

fn validate(program: &ScalarProgram) -> Vec<super::Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut declarations = BTreeMap::new();
    let mut declaration_names = BTreeSet::new();
    let mut folded_declarations = BTreeMap::new();
    for item in &program.items {
        let (name, span, ty) = match item {
            ScalarItem::Namespace(namespace) => (&namespace.binding, namespace.span, None),
            ScalarItem::Binding(binding) => {
                (&binding.name, binding.span, Some(&binding.declared_type))
            }
            ScalarItem::Function(function) => {
                (&function.name, function.span, Some(&function.signature))
            }
            ScalarItem::Executable(_) => continue,
        };
        if declare_program_name(
            program,
            name,
            span,
            &mut declaration_names,
            &mut folded_declarations,
            &mut diagnostics,
        ) {
            if let Some(ty) = ty {
                declarations.insert(name.clone(), ty.clone());
                validate_type(program, ty, span, &mut diagnostics);
            }
        }
    }
    let mut scope = declarations.clone();
    for item in &program.items {
        match item {
            ScalarItem::Namespace(_) => {}
            ScalarItem::Binding(binding) => {
                let actual = expression_type(
                    &binding.value,
                    &declarations,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    program,
                    &mut diagnostics,
                );
                expect_type(
                    program,
                    &binding.declared_type,
                    &actual,
                    binding.span,
                    &mut diagnostics,
                );
            }
            ScalarItem::Function(function) => {
                let ScalarType::Callable { result, parameters } = &function.signature else {
                    continue;
                };
                let mut scope = declarations.clone();
                let mut visible_names = declaration_names.clone();
                let mut folded_names = folded_declarations.clone();
                for (index, name) in function.parameters.iter().enumerate() {
                    if index < parameters.len()
                        && declare_program_name(
                            program,
                            name,
                            function.span,
                            &mut visible_names,
                            &mut folded_names,
                            &mut diagnostics,
                        )
                    {
                        scope.insert(name.clone(), parameters[index].clone());
                    }
                }
                let actual = block_type(
                    &function.body,
                    &scope,
                    &visible_names,
                    &folded_names,
                    program,
                    &mut diagnostics,
                );
                expect_type(
                    program,
                    result,
                    &actual,
                    function.body.span,
                    &mut diagnostics,
                );
                if function.parameters.len() != parameters.len() {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "function parameter arity does not match its type",
                        function.span,
                    ));
                }
            }
            ScalarItem::Executable(executable) => {
                let mut visible_names = declaration_names.clone();
                let mut folded_names = folded_declarations.clone();
                let _ = block_item_type(
                    executable,
                    &mut scope,
                    &mut visible_names,
                    &mut folded_names,
                    program,
                    &mut diagnostics,
                );
            }
        }
    }
    for span in unused_binding_spans(&program.items) {
        diagnostics.push(diagnostic(
            program,
            "B0009",
            "local binding or parameter has no required static use",
            span,
        ));
    }
    diagnostics
}

struct StaticUseAnalyzer {
    visible: BTreeMap<String, usize>,
    declarations: Vec<ByteSpan>,
    uses: BTreeSet<usize>,
}

impl StaticUseAnalyzer {
    fn new(function: &ScalarFunction) -> Self {
        let mut analyzer = Self {
            visible: BTreeMap::new(),
            declarations: Vec::new(),
            uses: BTreeSet::new(),
        };
        for (index, name) in function.parameters.iter().enumerate() {
            if let Some(span) = function.parameter_spans.get(index) {
                let id = analyzer.declarations.len();
                analyzer.visible.insert(name.clone(), id);
                analyzer.declarations.push(*span);
            }
        }
        analyzer
    }

    fn block(&mut self, block: &ScalarBlock, visible: &BTreeMap<String, usize>) {
        let mut visible = visible.clone();
        for item in &block.items {
            self.item(item, &mut visible);
        }
    }

    fn item(&mut self, item: &ScalarBlockItem, visible: &mut BTreeMap<String, usize>) {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                self.expression(&binding.value, visible);
                let id = self.declarations.len();
                visible.insert(binding.name.clone(), id);
                self.declarations.push(binding.span);
            }
            ScalarBlockItem::Expression(expression) => self.expression(expression, visible),
            ScalarBlockItem::Assignment(assignment) => {
                self.expression(&assignment.value, visible);
                if let Some(receiver) = &assignment.receiver {
                    self.use_name(receiver, visible);
                } else {
                    self.use_name(&assignment.target, visible);
                }
            }
            ScalarBlockItem::While(while_expression) => {
                self.expression(&while_expression.condition, visible);
                self.block(&while_expression.body, visible);
            }
        }
    }

    fn expression(&mut self, expression: &ScalarExpression, visible: &BTreeMap<String, usize>) {
        match expression {
            ScalarExpression::Name { name, .. } => self.use_name(name, visible),
            ScalarExpression::Member { receiver, .. } => self.use_name(receiver, visible),
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left, visible);
                self.expression(right, visible);
            }
            ScalarExpression::Call {
                receiver,
                name,
                arguments,
                ..
            } => {
                if let Some(receiver) = receiver {
                    self.use_name(receiver, visible);
                } else {
                    self.use_name(name, visible);
                }
                for argument in arguments {
                    self.expression(argument, visible);
                }
            }
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition, visible);
                self.block(then_branch, visible);
                self.block(else_branch, visible);
            }
            ScalarExpression::Block(block) => self.block(block, visible),
            ScalarExpression::Integer { .. }
            | ScalarExpression::InvalidInteger { .. }
            | ScalarExpression::Boolean { .. } => {}
        }
    }

    fn use_name(&mut self, name: &str, visible: &BTreeMap<String, usize>) {
        if let Some(id) = visible.get(name) {
            self.uses.insert(*id);
        }
    }

    fn unused(self) -> Vec<ByteSpan> {
        self.declarations
            .into_iter()
            .enumerate()
            .filter_map(|(id, span)| (!self.uses.contains(&id)).then_some(span))
            .collect()
    }
}

fn unused_binding_spans(items: &[ScalarItem]) -> Vec<ByteSpan> {
    items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) => {
                let mut analyzer = StaticUseAnalyzer::new(function);
                let visible = analyzer.visible.clone();
                analyzer.block(&function.body, &visible);
                Some(analyzer.unused())
            }
            ScalarItem::Namespace(_) | ScalarItem::Binding(_) | ScalarItem::Executable(_) => None,
        })
        .flatten()
        .collect()
}

fn validate_type(
    program: &ScalarProgram,
    ty: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named(_) => {
            diagnostics.push(diagnostic(program, "B0003", "unknown scalar type", span))
        }
        ScalarType::Callable { result, parameters } => {
            validate_type(program, result, span, diagnostics);
            for parameter in parameters {
                validate_type(program, parameter, span, diagnostics);
            }
        }
        ScalarType::Unit | ScalarType::Bool | ScalarType::I32 => {}
        ScalarType::Error => {
            diagnostics.push(diagnostic(program, "B0003", "invalid scalar type", span))
        }
    }
}

fn block_type(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = block_item_type(
            item,
            &mut scope,
            &mut visible_names,
            &mut folded_names,
            program,
            diagnostics,
        );
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

fn block_item_type(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let actual = expression_type(
                &binding.value,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
            );
            expect_type(
                program,
                &binding.declared_type,
                &actual,
                binding.span,
                diagnostics,
            );
            if declare_program_name(
                program,
                &binding.name,
                binding.span,
                visible_names,
                folded_names,
                diagnostics,
            ) {
                scope.insert(binding.name.clone(), binding.declared_type.clone());
            }
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => expression_type(
            expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
        ),
        ScalarBlockItem::Assignment(assignment) => assignment_type(
            assignment,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
        ),
        ScalarBlockItem::While(while_expression) => while_type(
            while_expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
        ),
    }
}

fn assignment_type(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let actual = expression_type(
        &assignment.value,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
    );
    let Some(expected) = scope.get(&assignment.target) else {
        if is_const_binding_name(&assignment.target)
            && program.items.iter().any(|item| {
                matches!(
                    item,
                    ScalarItem::Namespace(namespace) if namespace.binding == assignment.target
                )
            })
        {
            diagnostics.push(diagnostic(
                program,
                "B0007",
                "assignment targets a SCREAMING_SNAKE_CASE const binding",
                assignment.target_span,
            ));
            return ScalarType::Unit;
        }
        diagnostics.push(diagnostic(
            program,
            "B0001",
            "unknown assignment target",
            assignment.target_span,
        ));
        return ScalarType::Error;
    };
    if is_const_binding_name(&assignment.target) {
        diagnostics.push(diagnostic(
            program,
            "B0007",
            "assignment targets a SCREAMING_SNAKE_CASE const binding",
            assignment.target_span,
        ));
    }
    expect_type(program, expected, &actual, assignment.span, diagnostics);
    if is_error_type(&actual) {
        ScalarType::Error
    } else {
        expected.clone()
    }
}

fn while_type(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let condition = expression_type(
        &while_expression.condition,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
    );
    if !is_error_type(&condition) && condition != ScalarType::Bool {
        diagnostics.push(diagnostic(
            program,
            "B0005",
            "while expression requires bool",
            while_expression.span,
        ));
    }
    let _ = block_type(
        &while_expression.body,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
    );
    ScalarType::Unit
}

fn expression_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match expression {
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarExpression::Member { span, .. } => {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }
        ScalarExpression::Integer { value, span } => {
            validate_integer_range_program(program, value, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::InvalidInteger { span } => {
            invalid_integer_diagnostic_program(program, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let left_type = expression_type(
                left,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
            );
            let right_type = expression_type(
                right,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
            );
            if is_error_type(&left_type) || is_error_type(&right_type) {
                return ScalarType::Error;
            }
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            let boolean = matches!(operator, BinaryOperator::And | BinaryOperator::Or);
            if boolean {
                expect_type(program, &ScalarType::Bool, &left_type, *span, diagnostics);
                expect_type(program, &ScalarType::Bool, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if comparison {
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else {
                expect_type(program, &ScalarType::I32, &left_type, *span, diagnostics);
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                ScalarType::I32
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            span,
            ..
        } => {
            let lookup_name = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            let Some(ScalarType::Callable { result, parameters }) = scope.get(&lookup_name) else {
                diagnostics.push(diagnostic(program, "B0001", "unknown callable name", *span));
                return ScalarType::Error;
            };
            if arguments.len() != parameters.len() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            let mut error_argument = false;
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let actual = expression_type(
                    argument,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                );
                expect_type(program, parameter, &actual, *span, diagnostics);
                error_argument |= is_error_type(&actual);
            }
            if error_argument {
                ScalarType::Error
            } else {
                (**result).clone()
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let condition_type = expression_type(
                condition,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
            );
            if !is_error_type(&condition_type) && condition_type != ScalarType::Bool {
                diagnostics.push(diagnostic(
                    program,
                    "B0005",
                    "conditional expression requires bool",
                    *span,
                ));
            }
            let then_type = block_type(
                then_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
            );
            let else_type = block_type(
                else_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
            );
            if !is_error_type(&condition_type)
                && !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && then_type != else_type
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            if is_error_type(&condition_type) || is_error_type(&then_type) {
                ScalarType::Error
            } else if is_error_type(&else_type) {
                ScalarType::Error
            } else {
                then_type
            }
        }
        ScalarExpression::Block(block) => block_type(
            block,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
        ),
    }
}

fn expect_type(
    program: &ScalarProgram,
    expected: &ScalarType,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(expected) && !is_error_type(actual) && expected != actual {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
}

fn validate_integer_range(
    module: &ScalarModule,
    value: &BigInt,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if value > &BigInt::from(i32::MAX) {
        diagnostics.push(module_diagnostic(
            module,
            "B0010",
            "integer literal is outside the resolved target type range",
            span,
        ));
    }
}

fn validate_integer_range_program(
    program: &ScalarProgram,
    value: &BigInt,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if value > &BigInt::from(i32::MAX) {
        diagnostics.push(diagnostic(
            program,
            "B0010",
            "integer literal is outside the resolved target type range",
            span,
        ));
    }
}

fn invalid_integer_diagnostic(
    module: &ScalarModule,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    diagnostics.push(module_diagnostic(
        module,
        "B0010",
        "invalid integer literal",
        span,
    ));
}

fn invalid_integer_diagnostic_program(
    program: &ScalarProgram,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    diagnostics.push(diagnostic(
        program,
        "B0010",
        "invalid integer literal",
        span,
    ));
}

fn is_const_binding_name(name: &str) -> bool {
    name != "_"
        && name.bytes().any(|byte| byte.is_ascii_uppercase())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'_' || byte.is_ascii_digit())
}

fn diagnostic(
    program: &ScalarProgram,
    code: &str,
    message: &str,
    span: ByteSpan,
) -> super::Diagnostic {
    super::Diagnostic {
        code: code.to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(program.source.clone(), span),
            message: message.to_owned(),
        }],
        notes: Vec::new(),
    }
}

fn declare_program_name(
    program: &ScalarProgram,
    name: &str,
    span: ByteSpan,
    names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    if !names.insert(name.to_owned()) {
        diagnostics.push(diagnostic(program, "B0002", "duplicate declaration", span));
        return false;
    }
    let folded = fold_name(name);
    if let Some((first_name, first_span)) = folded_names.get(&folded) {
        if first_name != name {
            names.remove(name);
            let mut collision = diagnostic(
                program,
                "B0008",
                "declaration collides with an existing name under ASCII case folding",
                span,
            );
            collision.labels.push(super::DiagnosticLabel {
                kind: super::DiagnosticLabelKind::Secondary,
                span: SourceSpan::new(program.source.clone(), *first_span),
                message: "first conflicting declaration".to_owned(),
            });
            diagnostics.push(collision);
            return false;
        }
    } else {
        folded_names.insert(folded, (name.to_owned(), span));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_source;

    fn source() -> SourceIdentity {
        SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        )
    }

    fn validate_text(text: &str) -> ScalarValidation {
        let parsed = parse_source(source(), text.to_owned(), &[]);
        assert!(
            parsed.diagnostics.is_empty(),
            "syntax diagnostics: {:?}",
            parsed.diagnostics
        );
        derive_scalar_program(&parsed.result)
    }

    #[test]
    fn derives_valid_scalar_bindings_arithmetic_call_and_if() {
        let result = validate_text(
            "%%start\ni32(i32, i32) add = fn(left, right) {\n\tleft + right\n};\ni32 seed = 20;\ni32 increment = 22;\nbool choose_sum = true;\ni32 result = if (choose_sum) {\n\tadd(seed, increment)\n} else {\n\t0\n};\n%%end",
        );
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
        assert_eq!(result.program.items.len(), 5);
        assert!(matches!(result.program.items[0], ScalarItem::Function(_)));
        assert!(matches!(result.program.items[4], ScalarItem::Binding(_)));
    }

    #[test]
    fn reports_integer_range_at_literal_span_and_preserves_i32_max() {
        let invalid_text = "%%start\ni32 value = 2147483648;\n%%end";
        let invalid = validate_text(invalid_text);
        let diagnostic = invalid
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0010")
            .expect("integer range diagnostic");
        let start = invalid_text.find("2147483648").expect("literal") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + 10)
        );

        let valid = validate_text("%%start\ni32 minimum = 0;\ni32 maximum = 2147483647;\n%%end");
        assert!(
            valid.diagnostics.is_empty(),
            "diagnostics: {:?}",
            valid.diagnostics
        );

        let huge = validate_text(
            "%%start\ni32 value = 999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999;\n%%end",
        );
        assert!(huge.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0010"
                && diagnostic.message == "integer literal is outside the resolved target type range"
        }));
    }

    #[test]
    fn project_validation_reports_integer_range_at_literal_span() {
        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), "%%start\ni32 value = 2147483648;\n%%end");
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source.clone()],
        ));
        let diagnostic = validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0010")
            .expect("project integer range diagnostic");
        assert_eq!(diagnostic.labels[0].span.source, source);
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(20, 30));
    }

    #[test]
    fn terminated_final_expression_is_unit_in_program_and_project_blocks() {
        let invalid = validate_text("%%start\ni32() f = fn { 1; };\n%%end");
        assert!(invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
        let ScalarItem::Function(function) = &invalid.program.items[0] else {
            panic!("function item");
        };
        assert_eq!(function.body.terminated_items, vec![true]);

        let valid = validate_text("%%start\ni32() f = fn { 1 };\n%%end");
        assert!(
            valid.diagnostics.is_empty(),
            "diagnostics: {:?}",
            valid.diagnostics
        );
        let ScalarItem::Function(function) = &valid.program.items[0] else {
            panic!("function item");
        };
        assert_eq!(function.body.terminated_items, vec![false]);

        let source = module_source("src/main.w");
        let invalid_project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                source.clone(),
                invalid.program.items,
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(invalid_project
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));

        let valid_project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                source.clone(),
                valid.program.items,
                Vec::new(),
            )],
            vec![source],
        ));
        assert!(valid_project.diagnostics.is_empty());
    }

    #[test]
    fn validates_project_callable_parameter_arity_like_single_file_programs() {
        let source = "%%start\ni32(i32, i32) add = fn(value) { value };\n%%end";
        let single_file = validate_text(source);
        assert!(single_file
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0004"));

        let invalid_module_source = module_source("src/math.w");
        let module = module_from_text(invalid_module_source.clone(), source);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                invalid_module_source.clone(),
                module.items,
                Vec::new(),
            )],
            vec![invalid_module_source],
        ));
        let project_diagnostic = project
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0004")
            .expect("project callable arity diagnostic");
        assert_eq!(
            project_diagnostic.message,
            single_file
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "B0004")
                .expect("single-file callable arity diagnostic")
                .message
        );

        let valid_source = "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\n%%end";
        let valid_module_source = module_source("src/math.w");
        let valid_module = module_from_text(valid_module_source.clone(), valid_source);
        let valid_project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                valid_module_source.clone(),
                valid_module.items,
                Vec::new(),
            )],
            vec![valid_module_source],
        ));
        assert!(valid_project.diagnostics.is_empty());
    }

    #[test]
    fn rejects_visible_binding_collisions_in_single_and_project_validation() {
        let cases = [
            (
                "%%start\ni32 value = 1;\ni32(i32) f = fn(value) { value };\n%%end",
                "parameter/module",
            ),
            (
                "%%start\ni32(i32) f = fn(outer) { if (true) { i32 outer = 1; outer } else { outer } };\n%%end",
                "nested local/outer",
            ),
            (
                "%%start\ni32(i32, i32) f = fn(value, value) { value };\n%%end",
                "repeated parameter",
            ),
            (
                "%%start\nmath = namespace app \"src/math.w\";\ni32(i32) f = fn(math) { 0 };\n%%end",
                "parameter/namespace",
            ),
        ];
        for (text, case_name) in cases {
            let single = validate_text(text);
            assert!(
                single
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "B0002"),
                "missing single-program collision diagnostic for {case_name}: {:?}",
                single.diagnostics
            );

            let source = module_source("src/main.w");
            let program = module_from_text(source.clone(), text);
            let namespace_bindings = program
                .items
                .iter()
                .filter_map(|item| match item {
                    ScalarItem::Namespace(namespace) => Some(ScalarNamespaceBinding {
                        binding: namespace.binding.clone(),
                        target: module_source("src/math.w"),
                        span: namespace.span,
                    }),
                    _ => None,
                })
                .collect();
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::new(
                    source.clone(),
                    program.items,
                    namespace_bindings,
                )],
                vec![source],
            ));
            assert!(
                project
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "B0002"),
                "missing project collision diagnostic for {case_name}: {:?}",
                project.diagnostics
            );
        }

        let valid = validate_text(
            "%%start\ni32 module_value = 1;\ni32(i32) f = fn(parameter) { if (true) { i32 inner = parameter; inner } else { parameter } };\n%%end",
        );
        assert!(
            valid.diagnostics.is_empty(),
            "distinct bindings produced diagnostics: {:?}",
            valid.diagnostics
        );
    }

    #[test]
    fn derives_and_type_checks_top_level_assignment_in_module_scope() {
        let result = validate_text("%%start\ni32 value = 0;\nvalue = 1;\nvalue;\n%%end");
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
        assert_eq!(result.program.items.len(), 3);
        assert!(matches!(result.program.items[0], ScalarItem::Binding(_)));
        assert!(matches!(
            result.program.items[1],
            ScalarItem::Executable(ScalarBlockItem::Assignment(_))
        ));
        assert!(matches!(
            result.program.items[2],
            ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Name { .. }))
        ));

        let mismatch = validate_text("%%start\nbool value = false;\nvalue = 1;\n%%end");
        assert!(mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
        assert_eq!(
            mismatch
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "B0003")
                .expect("assignment type diagnostic")
                .labels[0]
                .span
                .range,
            ByteSpan::new(28, 37)
        );
    }

    #[test]
    fn derives_and_validates_ordered_local_mutation_and_unit_while() {
        let result = validate_text(
            "%%start\ni32(i32) loop = fn(start) {\n\ti32 value = start;\n\twhile (value < 3) {\n\t\tvalue = value + 1;\n\t}\n\tvalue\n};\n%%end",
        );
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
        let ScalarItem::Function(function) = &result.program.items[0] else {
            panic!("function item");
        };
        assert_eq!(result.program.source, source());
        assert_eq!(function.body.items.len(), 3);
        assert!(matches!(
            function.body.items[0],
            ScalarBlockItem::LocalBinding(_)
        ));
        let ScalarBlockItem::While(while_expression) = &function.body.items[1] else {
            panic!("while item");
        };
        assert_eq!(while_expression.span, ByteSpan::new(57, 100));
        assert!(matches!(
            while_expression.body.items[0],
            ScalarBlockItem::Assignment(_)
        ));
        assert_eq!(
            block_type(
                &while_expression.body,
                &BTreeMap::new(),
                &BTreeSet::new(),
                &BTreeMap::new(),
                &result.program,
                &mut Vec::new()
            ),
            ScalarType::Unit
        );
    }

    #[test]
    fn rejects_unknown_assignment_target_and_mismatched_assignment_type() {
        let unknown =
            validate_text("%%start\ni32(i32) f = fn(value) { missing = value; 0 };\n%%end");
        let diagnostic = unknown
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0001")
            .expect("unknown assignment target");
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(33, 40));

        let mismatch = validate_text(
            "%%start\ni32(i32) f = fn(value) { i32 local = value; local = true; local };\n%%end",
        );
        assert!(mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
    }

    #[test]
    fn rejects_const_binding_assignments_at_target_span_in_single_file() {
        let cases = [
            ("%%start\ni32 MAX_VALUE = 1;\nMAX_VALUE = 2;\n%%end", "MAX_VALUE"),
            (
                "%%start\ni32(i32) f = fn(PARAMETER) { PARAMETER = 1; PARAMETER };\n%%end",
                "PARAMETER",
            ),
            (
                "%%start\ni32(i32) F = fn(value) { i32 LOCAL_VALUE = value; LOCAL_VALUE = value; LOCAL_VALUE };\n%%end",
                "LOCAL_VALUE",
            ),
        ];
        for (text, target) in cases {
            let result = validate_text(text);
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "B0007")
                .expect("const assignment diagnostic");
            let start = text
                .match_indices(target)
                .nth(1)
                .map(|(start, _)| start)
                .expect("assignment target") as u32;
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + target.len() as u32)
            );
        }

        let valid = validate_text("%%start\ni32 MAX_VALUE = 1;\nMAX_VALUE;\n%%end");
        assert!(
            valid.diagnostics.is_empty(),
            "diagnostics: {:?}",
            valid.diagnostics
        );
    }

    #[test]
    fn rejects_const_binding_assignments_at_target_span_in_project() {
        let child_source = module_source("src/child.w");
        let child = module_from_text(child_source.clone(), "%%start\ni32 MAX_VALUE = 1;\n%%end");
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\ni32 value = child.MAX_VALUE;\nchild.MAX_VALUE = value;\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let assignment_target = match &main.items[2] {
            ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) => {
                assignment.target_span
            }
            _ => panic!("assignment item"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(child_source, child.items, Vec::new()),
            ],
            Vec::new(),
        ));
        let diagnostic = validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0007")
            .expect("project const assignment diagnostic");
        assert_eq!(diagnostic.labels[0].span.source, main_source);
        assert_eq!(diagnostic.labels[0].span.range, assignment_target);
    }

    #[test]
    fn rejects_non_boolean_while_condition() {
        let result =
            validate_text("%%start\ni32(i32) f = fn(value) { while (value) { value } 0 };\n%%end");
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0005"));
    }

    #[test]
    fn rejects_unknown_name() {
        let result = validate_text("%%start\ni32 value = missing;\n%%end");
        assert_eq!(result.diagnostics[0].code, "B0001");
        assert_eq!(
            result.diagnostics[0].labels[0].span.range,
            ByteSpan::new(20, 27)
        );
    }

    #[test]
    fn unknown_binding_suppresses_only_its_derived_type_diagnostic() {
        let text = "%%start\ni32 value = missing;\nbool sibling = 1;\n%%end";
        let single = validate_text(text);
        assert_eq!(
            single
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0001")
                .count(),
            1
        );
        assert_eq!(
            single
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .count(),
            1
        );

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert_eq!(
            project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0001")
                .count(),
            1
        );
        assert_eq!(
            project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .count(),
            1
        );
    }

    #[test]
    fn unknown_namespace_receiver_suppresses_only_its_derived_type_diagnostic() {
        let text = "%%start\ni32 value = missing.value;\nbool sibling = 1;\n%%end";
        let single = validate_text(text);
        assert_eq!(
            single
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0001")
                .count(),
            1
        );
        assert_eq!(
            single
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .count(),
            1
        );

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert_eq!(
            project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0001")
                .count(),
            1
        );
        assert_eq!(
            project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .count(),
            1
        );
    }

    #[test]
    fn unknown_callable_suppresses_only_its_derived_type_diagnostic() {
        let text = "%%start\ni32 value = missing();\nbool sibling = 1;\n%%end";
        let single = validate_text(text);
        assert_eq!(
            single
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0001")
                .count(),
            1
        );
        assert_eq!(
            single
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .count(),
            1
        );

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert_eq!(
            project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0001")
                .count(),
            1
        );
        assert_eq!(
            project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .count(),
            1
        );
    }

    #[test]
    fn rejects_invalid_type() {
        let result = validate_text("%%start\nu32 value = 1;\n%%end");
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
    }

    #[test]
    fn rejects_invalid_call_arity() {
        let result = validate_text(
            "%%start\ni32(i32) one = fn(value) { value };\ni32 result = one(1, 2);\n%%end",
        );
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0004"));
    }

    #[test]
    fn rejects_non_boolean_condition_and_mismatched_branches() {
        let condition = validate_text("%%start\ni32 value = if (1) { 1 } else { 1 };\n%%end");
        assert!(condition
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0005"));
        let branches = validate_text("%%start\ni32 value = if (true) { 1 } else { true };\n%%end");
        assert!(branches.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0006"
                && diagnostic.message == "conditional branches must have equal types"
        }));
    }

    #[test]
    fn derives_namespace_and_qualified_member_without_text_reparsing() {
        let result = validate_text(
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end",
        );
        let ScalarItem::Namespace(namespace) = &result.program.items[0] else {
            panic!("namespace item");
        };
        assert_eq!(namespace.binding, "math");
        assert_eq!(namespace.package, "app");
        assert_eq!(namespace.path, "\"src/math.w\"");
        assert_eq!(namespace.binding_span, ByteSpan::new(8, 12));
        assert_eq!(namespace.package_span, ByteSpan::new(25, 28));
        assert_eq!(namespace.path_span, ByteSpan::new(29, 41));

        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("result binding");
        };
        let ScalarExpression::Call {
            receiver,
            name,
            receiver_span,
            name_span,
            ..
        } = &binding.value
        else {
            panic!("qualified call");
        };
        assert_eq!(receiver.as_deref(), Some("math"));
        assert_eq!(name, "add");
        assert_eq!(*receiver_span, Some(ByteSpan::new(56, 60)));
        assert_eq!(*name_span, ByteSpan::new(61, 64));
    }

    #[test]
    fn rejects_duplicate_namespace_binding_in_single_program() {
        let result = validate_text(
            "%%start\nmath = namespace app \"src/first.w\";\nmath = namespace app \"src/second.w\";\n%%end",
        );
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0002")
            .collect();
        let ScalarItem::Namespace(second) = &result.program.items[1] else {
            panic!("second namespace item");
        };
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].labels[0].span.range, second.span);
    }

    #[test]
    fn reports_ascii_case_collision_with_first_declaration_label_in_single_file() {
        let result = validate_text("%%start\ni32 value = 1;\ni32 VALUE = 2;\n%%end");
        let first_span = match &result.program.items[0] {
            ScalarItem::Binding(binding) => binding.span,
            _ => panic!("first binding"),
        };
        let second_span = match &result.program.items[1] {
            ScalarItem::Binding(binding) => binding.span,
            _ => panic!("second binding"),
        };
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0008")
            .expect("case collision diagnostic");
        assert_eq!(diagnostic.labels[0].span.range, second_span);
        assert_eq!(
            diagnostic.labels[1].kind,
            crate::DiagnosticLabelKind::Secondary
        );
        assert_eq!(diagnostic.labels[1].span.range, first_span);
    }

    #[test]
    fn reports_case_collisions_for_parameters_and_locals_without_changing_lookup() {
        let result = validate_text(
            "%%start\ni32(i32, i32) parameter_collision = fn(first, FIRST) { first };\ni32(i32) local_collision = fn(value) { i32 VALUE = value; i32 value = value; value };\n%%end",
        );
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0008")
                .count(),
            2
        );
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0001"));
    }

    #[test]
    fn reports_ascii_case_collision_in_project_namespace_scope() {
        let source = module_source("src/main.w");
        let program = module_from_text(
            source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\nMATH = namespace app \"src/other.w\";\n%%end",
        );
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source, program.items, Vec::new())],
            Vec::new(),
        ));
        let diagnostic = validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0008")
            .expect("project case collision diagnostic");
        assert_eq!(diagnostic.labels.len(), 2);
    }

    #[test]
    fn reports_unused_local_and_parameter_at_declaration_spans() {
        let text = "%%start\nunit(i32) f = fn(unused_parameter) { i32 unused_local = 1; };\n%%end";
        let result = validate_text(text);
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0009")
            .collect();
        assert_eq!(diagnostics.len(), 2);
        let ScalarItem::Function(function) = &result.program.items[0] else {
            panic!("function item");
        };
        let local_span = match &function.body.items[0] {
            ScalarBlockItem::LocalBinding(binding) => binding.span,
            _ => panic!("local binding item"),
        };
        assert_eq!(
            diagnostics[0].labels[0].span.range,
            function.parameter_spans[0]
        );
        assert_eq!(diagnostics[1].labels[0].span.range, local_span);
    }

    #[test]
    fn counts_resolved_uses_across_initializers_assignments_conditions_calls_and_branches() {
        let result = validate_text(
            "%%start\nunit(i32) consume = fn(value) { value; };\ni32(i32) f = fn(input) { i32 local = input; while (local < 2) { local = local + 1; } if (local == 2) { consume(local); } else { consume(input); } local };\n%%end",
        );
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn validates_static_uses_in_project_path() {
        let source = module_source("src/main.w");
        let program = module_from_text(
            source.clone(),
            "%%start\nunit(i32) f = fn(unused_parameter) { i32 unused_local = 1; };\n%%end",
        );
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source.clone()],
        ));
        assert_eq!(
            validation
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0009")
                .count(),
            2
        );
        assert!(validation
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.labels[0].span.source == source));
    }

    fn module_source(path: &str) -> SourceIdentity {
        SourceIdentity::new("project".into(), "app".into(), path.into(), "r1".into())
    }

    fn module_from_text(source: SourceIdentity, text: &str) -> ScalarProgram {
        let parsed = parse_source(source, text.to_owned(), &[]);
        assert!(
            parsed.diagnostics.is_empty(),
            "syntax diagnostics: {:?}",
            parsed.diagnostics
        );
        derive_scalar_program(&parsed.result).program
    }

    #[test]
    fn resolves_qualified_member_only_through_bound_module() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(
            math_source.clone(),
            "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".to_owned(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source.clone(), math.items, Vec::new()),
            ],
            vec![main_source, math_source],
        );
        let result = validate_scalar_project(project);
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn validates_project_namespace_uniqueness_and_preserves_distinct_targets() {
        let first_source = module_source("src/first.w");
        let second_source = module_source("src/second.w");
        let main_source = module_source("src/main.w");
        let duplicate = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/first.w\";\nmath = namespace app \"src/second.w\";\n%%end",
        );
        let duplicate_bindings = vec![
            match &duplicate.items[0] {
                ScalarItem::Namespace(namespace) => ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: first_source.clone(),
                    span: namespace.span,
                },
                _ => panic!("namespace item"),
            },
            match &duplicate.items[1] {
                ScalarItem::Namespace(namespace) => ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: second_source.clone(),
                    span: namespace.span,
                },
                _ => panic!("namespace item"),
            },
        ];
        let second_span = duplicate_bindings[1].span;
        let duplicate_result = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(main_source.clone(), duplicate.items, duplicate_bindings),
                ScalarModule::new(
                    first_source.clone(),
                    module_from_text(first_source.clone(), "%%start\ni32 value = 1;\n%%end").items,
                    Vec::new(),
                ),
                ScalarModule::new(
                    second_source.clone(),
                    module_from_text(second_source.clone(), "%%start\ni32 value = 2;\n%%end").items,
                    Vec::new(),
                ),
            ],
            vec![
                main_source.clone(),
                first_source.clone(),
                second_source.clone(),
            ],
        ));
        let diagnostics: Vec<_> = duplicate_result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0002")
            .collect();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].labels[0].span.range, second_span);

        let distinct = module_from_text(
            main_source.clone(),
            "%%start\nfirst = namespace app \"src/first.w\";\nsecond = namespace app \"src/second.w\";\n%%end",
        );
        let distinct_bindings = vec![
            ScalarNamespaceBinding {
                binding: "first".to_owned(),
                target: first_source.clone(),
                span: match &distinct.items[0] {
                    ScalarItem::Namespace(namespace) => namespace.span,
                    _ => panic!("namespace item"),
                },
            },
            ScalarNamespaceBinding {
                binding: "second".to_owned(),
                target: second_source.clone(),
                span: match &distinct.items[1] {
                    ScalarItem::Namespace(namespace) => namespace.span,
                    _ => panic!("namespace item"),
                },
            },
        ];
        let distinct_result = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                main_source,
                distinct.items,
                distinct_bindings.clone(),
            )],
            Vec::new(),
        ));
        assert!(distinct_result.diagnostics.is_empty());
        assert_eq!(
            distinct_result.project.modules[0].namespace_bindings,
            distinct_bindings
        );
    }

    #[test]
    fn reports_unknown_member_at_member_span_and_target_source() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".to_owned(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source.clone(), math.items, Vec::new()),
            ],
            vec![main_source.clone(), math_source.clone()],
        );
        let result = validate_scalar_project(project);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "M0002")
            .expect("unknown member diagnostic");
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
        assert_eq!(diagnostic.labels[0].span.source, main_source);
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(61, 64));
        assert_eq!(diagnostic.labels[1].span.source, math_source);
    }

    #[test]
    fn resolves_qualified_member_read_and_assignment_through_bound_module() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.value;\nmath.value = result;\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let read = match &main.items[1] {
            ScalarItem::Binding(binding) => &binding.value,
            _ => panic!("result binding"),
        };
        let ScalarExpression::Member {
            receiver,
            name,
            receiver_span,
            name_span,
            span,
        } = read
        else {
            panic!("qualified member read");
        };
        assert_eq!(receiver, "math");
        assert_eq!(name, "value");
        assert_eq!(*receiver_span, ByteSpan::new(56, 60));
        assert_eq!(*name_span, ByteSpan::new(61, 66));
        assert_eq!(*span, ByteSpan::new(56, 66));
        let ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) = &main.items[2] else {
            panic!("qualified member assignment");
        };
        assert_eq!(assignment.receiver.as_deref(), Some("math"));
        assert_eq!(assignment.target, "value");
        assert_eq!(assignment.receiver_span, Some(ByteSpan::new(68, 72)));
        assert_eq!(assignment.target_span, ByteSpan::new(73, 78));
        let result = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".to_owned(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source, math.items, Vec::new()),
            ],
            Vec::new(),
        ));
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn reports_unknown_qualified_member_read_and_assignment_at_member_span() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.missing;\nmath.missing = result;\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let result = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".to_owned(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source.clone(), math.items, Vec::new()),
            ],
            Vec::new(),
        ));
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "M0002")
            .collect();
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].labels[0].span.range, ByteSpan::new(61, 68));
        assert_eq!(diagnostics[1].labels[0].span.range, ByteSpan::new(75, 82));
        assert_eq!(diagnostics[0].labels[1].span.source, math_source);
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
    }

    #[test]
    fn preserves_sibling_diagnostics_after_error_namespace_member() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 missing = math.missing;\ni32 invalid = true;\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let result = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source,
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".to_owned(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source, math.items, Vec::new()),
            ],
            Vec::new(),
        ));
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "M0002"));
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
    }

    #[test]
    fn rejects_importer_unqualified_member_lookup() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(
            math_source.clone(),
            "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = add(20, 22);\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let result = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source,
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "math".to_owned(),
                        target: math_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(math_source, math.items, Vec::new()),
            ],
            Vec::new(),
        ));
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0001"));
    }

    #[test]
    fn represents_source_ordered_module_initialization_once() {
        let first_source = module_source("src/first.w");
        let second_source = module_source("src/second.w");
        let first = ScalarModule::new(
            first_source.clone(),
            module_from_text(first_source.clone(), "%%start\ni32 first = 1;\n%%end").items,
            Vec::new(),
        );
        let second = ScalarModule::new(
            second_source.clone(),
            module_from_text(second_source.clone(), "%%start\ni32 second = 2;\n%%end").items,
            Vec::new(),
        );
        assert_eq!(first.initialization_nodes.len(), 1);
        assert_eq!(second.initialization_nodes.len(), 1);
        let project = ScalarProject::new(
            vec![first, second],
            vec![
                second_source.clone(),
                first_source.clone(),
                second_source.clone(),
            ],
        );
        assert_eq!(
            project.initialization_order,
            vec![second_source, first_source]
        );
    }
}
