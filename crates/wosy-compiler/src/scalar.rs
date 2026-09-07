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
    pub expressions: Vec<ScalarExpression>,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProgram {
    pub source: SourceIdentity,
    pub items: Vec<ScalarItem>,
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
    let expressions = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::BlockItem)
        .filter_map(|item| {
            direct_nodes(&item)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Expression)
        })
        .map(|node| derive_expression(&node))
        .collect();
    ScalarBlock {
        expressions,
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
        };
        if declarations.insert(name.clone(), ty.clone()).is_some() {
            diagnostics.push(diagnostic(program, "B0002", "duplicate declaration", span));
        }
        validate_type(program, ty, span, &mut diagnostics);
    }
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
    block
        .expressions
        .last()
        .map_or(ScalarType::Unit, |expression| {
            expression_type(expression, scope, program, diagnostics)
        })
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
}
