use std::collections::BTreeMap;

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
    Integer {
        value: i32,
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
    pub target: String,
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
            members.insert(name.clone(), ty.clone());
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
        for item in &module.items {
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
            if declarations.insert(name.clone(), ty.clone()).is_some() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "duplicate declaration",
                    span,
                ));
            }
            validate_module_type(module, ty, span, &mut diagnostics);
        }
        for item in &module.items {
            match item {
                ScalarItem::Namespace(_) => {}
                ScalarItem::Binding(binding) => {
                    let actual = expression_type_in_module(
                        &binding.value,
                        &declarations,
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
                    let mut scope = declarations.clone();
                    for (index, name) in function.parameters.iter().enumerate() {
                        if index < parameters.len() {
                            scope.insert(name.clone(), parameters[index].clone());
                        }
                    }
                    let actual = block_type_in_module(
                        &function.body,
                        &scope,
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
                    let _ = block_item_type_in_module(
                        executable,
                        &mut scope,
                        module,
                        &project.modules,
                        &mut diagnostics,
                    );
                }
            }
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
    }
}

fn block_type_in_module(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut result = ScalarType::Unit;
    for item in &block.items {
        result = block_item_type_in_module(item, &mut scope, module, modules, diagnostics);
    }
    result
}

fn block_item_type_in_module(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let actual =
                expression_type_in_module(&binding.value, scope, module, modules, diagnostics);
            expect_module_type(
                module,
                &binding.declared_type,
                &actual,
                binding.span,
                diagnostics,
            );
            scope.insert(binding.name.clone(), binding.declared_type.clone());
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => {
            expression_type_in_module(expression, scope, module, modules, diagnostics)
        }
        ScalarBlockItem::Assignment(assignment) => {
            assignment_type_in_module(assignment, scope, module, modules, diagnostics)
        }
        ScalarBlockItem::While(while_expression) => {
            while_type_in_module(while_expression, scope, module, modules, diagnostics)
        }
    }
}

fn assignment_type_in_module(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let actual = expression_type_in_module(&assignment.value, scope, module, modules, diagnostics);
    let Some(expected) = scope.get(&assignment.target) else {
        diagnostics.push(module_diagnostic(
            module,
            "B0001",
            "unknown assignment target",
            assignment.target_span,
        ));
        return ScalarType::Unit;
    };
    expect_module_type(module, expected, &actual, assignment.span, diagnostics);
    expected.clone()
}

fn while_type_in_module(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let condition = expression_type_in_module(
        &while_expression.condition,
        scope,
        module,
        modules,
        diagnostics,
    );
    if condition != ScalarType::Bool {
        diagnostics.push(module_diagnostic(
            module,
            "B0005",
            "while expression requires bool",
            while_expression.span,
        ));
    }
    let _ = block_type_in_module(&while_expression.body, scope, module, modules, diagnostics);
    ScalarType::Unit
}

fn expression_type_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match expression {
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
            ScalarType::Unit
        }),
        ScalarExpression::Integer { .. } => ScalarType::I32,
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let left_type = expression_type_in_module(left, scope, module, modules, diagnostics);
            let right_type = expression_type_in_module(right, scope, module, modules, diagnostics);
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
                        return ScalarType::Unit;
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
                        return ScalarType::Unit;
                    };
                    let Some(callable) = target.members.get(name) else {
                        diagnostics.push(unknown_member_diagnostic(
                            module,
                            *name_span,
                            &target.source,
                        ));
                        return ScalarType::Unit;
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
                return ScalarType::Unit;
            };
            let ScalarType::Callable { result, parameters } = callable else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown callable name",
                    *span,
                ));
                return ScalarType::Unit;
            };
            if arguments.len() != parameters.len() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let actual =
                    expression_type_in_module(argument, scope, module, modules, diagnostics);
                expect_module_type(target, parameter, &actual, *span, diagnostics);
            }
            result.as_ref().clone()
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let condition_type =
                expression_type_in_module(condition, scope, module, modules, diagnostics);
            expect_module_type(
                module,
                &ScalarType::Bool,
                &condition_type,
                *span,
                diagnostics,
            );
            let then_type = block_type_in_module(then_branch, scope, module, modules, diagnostics);
            let else_type = block_type_in_module(else_branch, scope, module, modules, diagnostics);
            if then_type != else_type {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            then_type
        }
        ScalarExpression::Block(block) => {
            block_type_in_module(block, scope, module, modules, diagnostics)
        }
    }
}

fn expect_module_type(
    module: &ScalarModule,
    expected: &ScalarType,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if expected != actual {
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
    let body = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Block)
        .map(|node| derive_block(node))
        .expect("function block");
    ScalarFunction {
        name,
        signature,
        parameters,
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
    let expressions = items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    ScalarBlock {
        items,
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
    let target = direct_token(node, SyntaxKind::Identifier).expect("assignment target");
    let value = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::Expression)
        .map(|child| derive_expression(&child))
        .expect("assignment value");
    ScalarAssignment {
        target: target.text().to_owned(),
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
            SyntaxKind::Integer => ScalarExpression::Integer {
                value: token.text().parse::<i32>().expect("integer grammar"),
                span: token_span(token),
            },
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
        | ScalarExpression::Integer { span, .. }
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
    for item in &program.items {
        let (name, ty, span) = match item {
            ScalarItem::Namespace(_) => continue,
            ScalarItem::Binding(binding) => (&binding.name, &binding.declared_type, binding.span),
            ScalarItem::Function(function) => (&function.name, &function.signature, function.span),
            ScalarItem::Executable(_) => continue,
        };
        if declarations.insert(name.clone(), ty.clone()).is_some() {
            diagnostics.push(diagnostic(program, "B0002", "duplicate declaration", span));
        }
        validate_type(program, ty, span, &mut diagnostics);
    }
    let mut scope = declarations.clone();
    for item in &program.items {
        match item {
            ScalarItem::Namespace(_) => {}
            ScalarItem::Binding(binding) => {
                let actual =
                    expression_type(&binding.value, &declarations, program, &mut diagnostics);
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
                for (index, name) in function.parameters.iter().enumerate() {
                    if index < parameters.len() {
                        scope.insert(name.clone(), parameters[index].clone());
                    }
                }
                let actual = block_type(&function.body, &scope, program, &mut diagnostics);
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
                let _ = block_item_type(executable, &mut scope, program, &mut diagnostics);
            }
        }
    }
    diagnostics
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
    }
}

fn block_type(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut result = ScalarType::Unit;
    for item in &block.items {
        result = block_item_type(item, &mut scope, program, diagnostics);
    }
    result
}

fn block_item_type(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let actual = expression_type(&binding.value, scope, program, diagnostics);
            expect_type(
                program,
                &binding.declared_type,
                &actual,
                binding.span,
                diagnostics,
            );
            scope.insert(binding.name.clone(), binding.declared_type.clone());
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => {
            expression_type(expression, scope, program, diagnostics)
        }
        ScalarBlockItem::Assignment(assignment) => {
            assignment_type(assignment, scope, program, diagnostics)
        }
        ScalarBlockItem::While(while_expression) => {
            while_type(while_expression, scope, program, diagnostics)
        }
    }
}

fn assignment_type(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let actual = expression_type(&assignment.value, scope, program, diagnostics);
    let Some(expected) = scope.get(&assignment.target) else {
        diagnostics.push(diagnostic(
            program,
            "B0001",
            "unknown assignment target",
            assignment.target_span,
        ));
        return ScalarType::Unit;
    };
    expect_type(program, expected, &actual, assignment.span, diagnostics);
    expected.clone()
}

fn while_type(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let condition = expression_type(&while_expression.condition, scope, program, diagnostics);
    if condition != ScalarType::Bool {
        diagnostics.push(diagnostic(
            program,
            "B0005",
            "while expression requires bool",
            while_expression.span,
        ));
    }
    let _ = block_type(&while_expression.body, scope, program, diagnostics);
    ScalarType::Unit
}

fn expression_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match expression {
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Unit
        }),
        ScalarExpression::Integer { .. } => ScalarType::I32,
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let left_type = expression_type(left, scope, program, diagnostics);
            let right_type = expression_type(right, scope, program, diagnostics);
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
                return ScalarType::Unit;
            };
            if arguments.len() != parameters.len() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let actual = expression_type(argument, scope, program, diagnostics);
                expect_type(program, parameter, &actual, *span, diagnostics);
            }
            (**result).clone()
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let condition_type = expression_type(condition, scope, program, diagnostics);
            if condition_type != ScalarType::Bool {
                diagnostics.push(diagnostic(
                    program,
                    "B0005",
                    "conditional expression requires bool",
                    *span,
                ));
            }
            let then_type = block_type(then_branch, scope, program, diagnostics);
            let else_type = block_type(else_branch, scope, program, diagnostics);
            if then_type != else_type {
                diagnostics.push(diagnostic(
                    program,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            then_type
        }
        ScalarExpression::Block(block) => block_type(block, scope, program, diagnostics),
    }
}

fn expect_type(
    program: &ScalarProgram,
    expected: &ScalarType,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if expected != actual {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
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
        assert!(branches
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0006"));
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
        assert_eq!(diagnostic.labels[0].span.source, main_source);
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(61, 64));
        assert_eq!(diagnostic.labels[1].span.source, math_source);
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
