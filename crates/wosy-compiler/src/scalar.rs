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
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    ArtifactId,
    RawPointer(Box<ScalarType>),
    Callable {
        outputs: ScalarOutputSequence,
        parameters: Vec<ScalarType>,
    },
    Named {
        name: String,
        span: ByteSpan,
    },
    Qualified {
        receiver: String,
        receiver_span: ByteSpan,
        member: String,
        member_span: ByteSpan,
        span: ByteSpan,
    },
    Struct(ScalarStructId),
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructId {
    pub source: SourceIdentity,
    pub index: usize,
}

impl Ord for ScalarStructId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.source.project,
            &self.source.package,
            &self.source.path,
            &self.source.revision,
            self.index,
        )
            .cmp(&(
                &other.source.project,
                &other.source.package,
                &other.source.path,
                &other.source.revision,
                other.index,
            ))
    }
}

impl PartialOrd for ScalarStructId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ScalarStructFieldId {
    pub structure: ScalarStructId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarFieldReference {
    Unresolved { name: String, span: ByteSpan },
    Resolved(ScalarStructFieldId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarTargetLayout {
    pub pointer_size: u64,
    pub pointer_alignment: u64,
}

impl ScalarTargetLayout {
    pub const WASM32: Self = Self {
        pointer_size: 4,
        pointer_alignment: 4,
    };
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutput {
    pub ty: ScalarType,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputSequence {
    pub outputs: Vec<ScalarOutput>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarPlace {
    Name {
        name: String,
        span: ByteSpan,
    },
    Field {
        base: Box<ScalarPlace>,
        field: ScalarFieldReference,
        span: ByteSpan,
    },
    Dereference {
        pointer: Box<ScalarExpression>,
        span: ByteSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarLayout {
    pub size: u64,
    pub alignment: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructField {
    pub id: ScalarStructFieldId,
    pub name: String,
    pub name_span: ByteSpan,
    pub ty: ScalarType,
    pub declaration_index: usize,
    pub offset: u64,
    pub layout: ScalarLayout,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStruct {
    pub id: ScalarStructId,
    pub name: String,
    pub name_span: ByteSpan,
    pub fields: Vec<ScalarStructField>,
    pub span: ByteSpan,
    pub layout: ScalarLayout,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputReceiver {
    pub name: String,
    pub name_span: ByteSpan,
    pub ty: ScalarType,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputValue {
    pub position: usize,
    pub ty: ScalarType,
    pub span: ByteSpan,
    pub value: ScalarExpression,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructLiteral {
    pub fields: Vec<ScalarStructLiteralField>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructLiteralField {
    pub name: String,
    pub name_span: ByteSpan,
    pub value: ScalarExpression,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarTypeArgument {
    pub ty: ScalarType,
    pub span: ByteSpan,
}

fn is_error_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Error)
}

fn scalar_type_equal(left: &ScalarType, right: &ScalarType) -> bool {
    match (left, right) {
        (ScalarType::RawPointer(left), ScalarType::RawPointer(right)) => {
            scalar_type_equal(left, right)
        }
        (
            ScalarType::Callable {
                outputs: left_outputs,
                parameters: left_parameters,
            },
            ScalarType::Callable {
                outputs: right_outputs,
                parameters: right_parameters,
            },
        ) => {
            left_outputs.outputs.len() == right_outputs.outputs.len()
                && left_outputs
                    .outputs
                    .iter()
                    .zip(&right_outputs.outputs)
                    .all(|(left, right)| scalar_type_equal(&left.ty, &right.ty))
                && left_parameters.len() == right_parameters.len()
                && left_parameters
                    .iter()
                    .zip(right_parameters)
                    .all(|(left, right)| scalar_type_equal(left, right))
        }
        (ScalarType::RawPointer(_), _) | (_, ScalarType::RawPointer(_)) => false,
        (ScalarType::Callable { .. }, _) | (_, ScalarType::Callable { .. }) => false,
        _ => left == right,
    }
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
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UnaryOperator {
    LogicalNot,
    BitwiseNot,
    Negate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        error_span: Option<ByteSpan>,
    },
    Float {
        value: f64,
        spelling: String,
        span: ByteSpan,
    },
    InvalidFloat {
        span: ByteSpan,
    },
    Boolean {
        value: bool,
        span: ByteSpan,
    },
    Char {
        value: char,
        spelling: String,
        span: ByteSpan,
    },
    Utf8 {
        value: Vec<u8>,
        span: ByteSpan,
    },
    RawAddress {
        place: ScalarPlace,
        span: ByteSpan,
    },
    StructLiteral {
        fields: Vec<ScalarStructLiteralField>,
        span: ByteSpan,
    },
    Binary {
        operator: BinaryOperator,
        left: Box<ScalarExpression>,
        right: Box<ScalarExpression>,
        span: ByteSpan,
    },
    Unary {
        operator: UnaryOperator,
        operand: Box<ScalarExpression>,
        span: ByteSpan,
    },
    Call {
        receiver: Option<String>,
        name: String,
        receiver_span: Option<ByteSpan>,
        name_span: ByteSpan,
        type_arguments: Vec<ScalarTypeArgument>,
        arguments: Vec<ScalarExpression>,
        span: ByteSpan,
    },
    If {
        condition: Box<ScalarExpression>,
        then_branch: ScalarBlock,
        else_branch: ScalarBlock,
        span: ByteSpan,
    },
    UnitIf {
        condition: Box<ScalarExpression>,
        then_branch: ScalarBlock,
        span: ByteSpan,
    },
    Block(ScalarBlock),
}

impl Eq for ScalarExpression {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarExternModule {
    Valid(String),
    Invalid {
        span: ByteSpan,
        error_span: ByteSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarBlock {
    pub items: Vec<ScalarBlockItem>,
    pub terminated_items: Vec<bool>,
    pub expressions: Vec<ScalarExpression>,
    pub span: ByteSpan,
    pub unsafe_context: bool,
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
    pub targets: Vec<ScalarAssignmentTarget>,
    pub value: ScalarExpression,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAssignmentTarget {
    pub receiver: Option<String>,
    pub target: String,
    pub receiver_span: Option<ByteSpan>,
    pub target_span: ByteSpan,
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
    pub receivers: Vec<ScalarOutputReceiver>,
    pub output_sequence: ScalarOutputSequence,
    pub output_values: Vec<ScalarOutputValue>,
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
pub struct ScalarExternFunction {
    pub name: String,
    pub signature: ScalarType,
    pub unsafe_marker: bool,
    pub name_span: ByteSpan,
    pub signature_span: ByteSpan,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarExtern {
    pub binding: String,
    pub actual_module: ScalarExternModule,
    pub kind: String,
    pub functions: Vec<ScalarExternFunction>,
    pub binding_span: ByteSpan,
    pub module_span: ByteSpan,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarItem {
    Namespace(ScalarNamespace),
    Extern(ScalarExtern),
    Binding(ScalarBinding),
    Function(ScalarFunction),
    Executable(ScalarBlockItem),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProgram {
    pub source: SourceIdentity,
    pub items: Vec<ScalarItem>,
    pub structs: Vec<ScalarStruct>,
    pub target_layout: ScalarTargetLayout,
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
    pub structs: Vec<ScalarStruct>,
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
                ScalarItem::Extern(extern_decl) => {
                    for function in &extern_decl.functions {
                        members.insert(function.name.clone(), function.signature.clone());
                    }
                    continue;
                }
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
            structs: Vec::new(),
        }
    }

    pub fn from_program(
        program: ScalarProgram,
        namespace_bindings: Vec<ScalarNamespaceBinding>,
    ) -> Self {
        let mut module = Self::new(program.source, program.items, namespace_bindings);
        module.structs = program.structs;
        module
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
    let mut diagnostics = string_diagnostics(canonical);
    diagnostics.extend(char_diagnostics(canonical));
    diagnostics.extend(integer_diagnostics(canonical));
    diagnostics.extend(float_diagnostics(canonical));
    let source_root = canonical
        .root
        .children()
        .find(|node| node.kind() == SyntaxKind::Root)
        .expect("source root");
    let structs = source_root
        .children()
        .chain(source_root.descendants())
        .filter(|node| node.kind() == SyntaxKind::StructDecl)
        .enumerate()
        .map(|(index, node)| {
            derive_struct(
                node,
                ScalarStructId {
                    source: canonical.source.clone(),
                    index,
                },
            )
        })
        .collect();
    for node in source_root.children() {
        match node.kind() {
            SyntaxKind::NamespaceDecl => items.push(ScalarItem::Namespace(derive_namespace(&node))),
            SyntaxKind::ExternDecl => items.push(ScalarItem::Extern(derive_extern(&node))),
            SyntaxKind::BindingDecl => items.push(ScalarItem::Binding(derive_binding(&node))),
            SyntaxKind::FunctionDecl => items.push(ScalarItem::Function(derive_function(&node))),
            SyntaxKind::Item => {
                for item in node.children() {
                    match item.kind() {
                        SyntaxKind::NamespaceDecl => {
                            items.push(ScalarItem::Namespace(derive_namespace(&item)))
                        }
                        SyntaxKind::ExternDecl => {
                            items.push(ScalarItem::Extern(derive_extern(&item)))
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
    let mut program = ScalarProgram {
        source: canonical.source.clone(),
        items,
        structs,
        target_layout: ScalarTargetLayout::WASM32,
    };
    resolve_program_types(&mut program);
    resolve_program_places(&mut program);
    let mut validation_diagnostics = validate(&program);
    let mut diagnostics = diagnostics;
    diagnostics.append(&mut validation_diagnostics);
    ScalarValidation {
        program,
        diagnostics,
    }
}

fn resolve_program_types(program: &mut ScalarProgram) {
    let names: BTreeMap<String, ScalarStructId> = program
        .structs
        .iter()
        .map(|structure| (structure.name.clone(), structure.id.clone()))
        .collect();
    for structure in &mut program.structs {
        for field in &mut structure.fields {
            field.ty = resolve_type(&field.ty, &names);
            field.layout = layout_for_type(&field.ty);
        }
        let mut offset = 0;
        let mut alignment = 1;
        for field in &mut structure.fields {
            alignment = alignment.max(field.layout.alignment);
            offset = align_offset(offset, field.layout.alignment);
            field.offset = offset;
            offset += field.layout.size;
        }
        structure.layout = ScalarLayout {
            size: align_offset(offset, alignment),
            alignment,
        };
    }
    let structs = program.structs.clone();
    for structure in &mut program.structs {
        recompute_struct_layout(structure, &structs);
    }
    for item in &mut program.items {
        match item {
            ScalarItem::Binding(binding) => {
                binding.declared_type = resolve_type(&binding.declared_type, &names);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type(&receiver.ty, &names);
                }
                binding.output_sequence.outputs = binding
                    .output_sequence
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: resolve_type(&output.ty, &names),
                        span: output.span,
                    })
                    .collect();
            }
            ScalarItem::Function(function) => {
                function.signature = resolve_type(&function.signature, &names);
                resolve_block_types(&mut function.body, &names);
            }
            ScalarItem::Extern(extern_decl) => {
                for function in &mut extern_decl.functions {
                    function.signature = resolve_type(&function.signature, &names);
                }
            }
            ScalarItem::Executable(item) => resolve_item_types(item, &names),
            ScalarItem::Namespace(_) => {}
        }
    }
}

fn resolve_item_types(item: &mut ScalarBlockItem, names: &BTreeMap<String, ScalarStructId>) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            binding.declared_type = resolve_type(&binding.declared_type, names);
            for receiver in &mut binding.receivers {
                receiver.ty = resolve_type(&receiver.ty, names);
            }
        }
        ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
            resolve_block_types(block, names)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_block_types(&mut while_expression.body, names)
        }
        ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
    }
}

fn resolve_block_types(block: &mut ScalarBlock, names: &BTreeMap<String, ScalarStructId>) {
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                binding.declared_type = resolve_type(&binding.declared_type, names);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type(&receiver.ty, names);
                }
            }
            ScalarBlockItem::While(while_expression) => {
                resolve_block_types(&mut while_expression.body, names)
            }
            ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
        }
    }
}

fn resolve_type(ty: &ScalarType, names: &BTreeMap<String, ScalarStructId>) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => names.get(name).cloned().map_or_else(
            || ScalarType::Named {
                name: name.clone(),
                span: *span,
            },
            ScalarType::Struct,
        ),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(resolve_type(inner, names)))
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: resolve_type(&output.ty, names),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| resolve_type(parameter, names))
                .collect(),
        },
        value => value.clone(),
    }
}

fn resolve_program_places(program: &mut ScalarProgram) {
    let context = program.clone();
    let declarations = program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => {
                Some((binding.name.clone(), binding.declared_type.clone()))
            }
            ScalarItem::Function(function) => {
                Some((function.name.clone(), function.signature.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for item in &mut program.items {
        match item {
            ScalarItem::Binding(binding) => {
                resolve_expression_places(&mut binding.value, &declarations, &context);
            }
            ScalarItem::Function(function) => {
                let mut scope = declarations.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        scope.insert(name.clone(), ty.clone());
                    }
                }
                resolve_block_places(&mut function.body, &mut scope, &context);
            }
            ScalarItem::Executable(item) => {
                resolve_item_places(item, &mut declarations.clone(), &context);
            }
            ScalarItem::Extern(_) | ScalarItem::Namespace(_) => {}
        }
    }
}

fn resolve_item_places(
    item: &mut ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            resolve_expression_places(&mut binding.value, scope, program);
            scope.insert(binding.name.clone(), binding.declared_type.clone());
        }
        ScalarBlockItem::Expression(expression) => {
            resolve_expression_places(expression, scope, program)
        }
        ScalarBlockItem::Assignment(assignment) => {
            resolve_expression_places(&mut assignment.value, scope, program)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_expression_places(&mut while_expression.condition, scope, program);
            resolve_block_places(&mut while_expression.body, scope, program);
        }
    }
}

fn resolve_block_places(
    block: &mut ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) {
    for item in &mut block.items {
        resolve_item_places(item, scope, program);
    }
}

fn resolve_expression_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) {
    match expression {
        ScalarExpression::RawAddress { place, .. } => resolve_place(place, scope, program),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_places(&mut field.value, scope, program);
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            resolve_expression_places(left, scope, program);
            resolve_expression_places(right, scope, program);
        }
        ScalarExpression::Unary { operand, .. } => {
            resolve_expression_places(operand, scope, program);
        }
        ScalarExpression::Call { arguments, .. } => {
            for argument in arguments {
                resolve_expression_places(argument, scope, program);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_places(condition, scope, program);
            let mut then_scope = scope.clone();
            resolve_block_places(then_branch, &mut then_scope, program);
            let mut else_scope = scope.clone();
            resolve_block_places(else_branch, &mut else_scope, program);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_places(condition, scope, program);
            let mut then_scope = scope.clone();
            resolve_block_places(then_branch, &mut then_scope, program);
        }
        ScalarExpression::Block(block) => {
            let mut scope = scope.clone();
            resolve_block_places(block, &mut scope, program);
        }
        ScalarExpression::Name { .. }
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

fn resolve_place(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            resolve_expression_places(pointer, scope, program)
        }
        ScalarPlace::Field { base, field, span } => {
            resolve_place(base, scope, program);
            let base_type = place_type(base, scope, program, &mut Vec::new());
            let structure = match base_type {
                ScalarType::Struct(id) => program.structs.get(id.index),
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => program.structs.get(id.index),
                    _ => None,
                },
                _ => None,
            };
            if let Some(declared) = structure.and_then(|structure| {
                structure
                    .fields
                    .iter()
                    .find(|candidate| {
                        matches!(field, ScalarFieldReference::Unresolved { name, .. } if candidate.name == *name)
                    })
            }) {
                *field = ScalarFieldReference::Resolved(declared.id.clone());
            } else {
                let _ = span;
            }
        }
    }
}

pub fn derive_scalar_diagnostics_from_cst(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    let mut diagnostics = string_diagnostics(canonical);
    diagnostics.extend(char_diagnostics(canonical));
    diagnostics.extend(integer_diagnostics(canonical));
    diagnostics.extend(float_diagnostics(canonical));
    diagnostics
}

pub fn validate_scalar_project(project: ScalarProject) -> ScalarProjectValidation {
    let mut project = project;
    let struct_lookup = project.modules.clone();
    for module in &mut project.modules {
        resolve_module_types_in_project(module, &struct_lookup);
    }
    for module in &mut project.modules {
        for structure in &mut module.structs {
            recompute_struct_layout_project(structure, &struct_lookup);
        }
    }
    for module in &mut project.modules {
        resolve_module_places(module, &struct_lookup);
    }
    let mut diagnostics = Vec::new();
    for module in &project.modules {
        validate_unit_if_positions_in_module(module, &mut diagnostics);
        let mut declarations = BTreeMap::new();
        let mut declaration_names = BTreeSet::new();
        let mut folded_declarations = BTreeMap::new();
        for item in &module.items {
            let (name, span, ty) = match item {
                ScalarItem::Namespace(namespace) => (&namespace.binding, namespace.span, None),
                ScalarItem::Extern(extern_decl) => (&extern_decl.binding, extern_decl.span, None),
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
                    if let ScalarItem::Binding(binding) = item {
                        for receiver in &binding.receivers {
                            declarations.insert(receiver.name.clone(), receiver.ty.clone());
                        }
                    }
                    validate_module_type(module, ty, span, &mut diagnostics);
                }
            }
        }
        for item in &module.items {
            if let ScalarItem::Extern(extern_decl) = item {
                validate_extern(module, extern_decl, &mut diagnostics);
            }
        }
        for item in &module.items {
            match item {
                ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
                ScalarItem::Binding(binding) => {
                    let actual = expression_type_in_module_expected(
                        &binding.value,
                        Some(&binding.declared_type),
                        &declarations,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        module,
                        &project.modules,
                        &mut diagnostics,
                        false,
                    );
                    validate_output_receivers_in_module(
                        &binding.value,
                        &binding.receivers,
                        binding.span,
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
                    let ScalarType::Callable {
                        outputs,
                        parameters,
                    } = &function.signature
                    else {
                        continue;
                    };
                    let output = &outputs.outputs[0];
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
                    let actual = block_type_in_module_expected(
                        &function.body,
                        Some(&output.ty),
                        &scope,
                        &visible_names,
                        &folded_names,
                        module,
                        &project.modules,
                        &mut diagnostics,
                        false,
                    );
                    expect_module_type(
                        module,
                        &output.ty,
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
                        false,
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

fn resolve_module_types_in_project(module: &mut ScalarModule, modules: &[ScalarModule]) {
    let context = module.clone();
    let names = module
        .structs
        .iter()
        .map(|structure| (structure.name.clone(), structure.id.clone()))
        .collect::<BTreeMap<_, _>>();
    for structure in &mut module.structs {
        for field in &mut structure.fields {
            field.ty = resolve_type_in_project(&field.ty, &names, &context, modules);
        }
    }
    for item in &mut module.items {
        match item {
            ScalarItem::Binding(binding) => {
                binding.declared_type =
                    resolve_type_in_project(&binding.declared_type, &names, &context, modules);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type_in_project(&receiver.ty, &names, &context, modules);
                }
            }
            ScalarItem::Function(function) => {
                function.signature =
                    resolve_type_in_project(&function.signature, &names, &context, modules);
                resolve_block_types_project(&mut function.body, &names, &context, modules);
            }
            ScalarItem::Extern(extern_decl) => {
                for function in &mut extern_decl.functions {
                    function.signature =
                        resolve_type_in_project(&function.signature, &names, &context, modules);
                }
            }
            ScalarItem::Executable(item) => {
                resolve_item_types_project(item, &names, &context, modules)
            }
            ScalarItem::Namespace(_) => {}
        }
    }
    for item in &module.items {
        match item {
            ScalarItem::Binding(binding) => {
                module
                    .members
                    .insert(binding.name.clone(), binding.declared_type.clone());
            }
            ScalarItem::Function(function) => {
                module
                    .members
                    .insert(function.name.clone(), function.signature.clone());
            }
            _ => {}
        }
    }
}

fn resolve_item_types_project(
    item: &mut ScalarBlockItem,
    names: &BTreeMap<String, ScalarStructId>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            binding.declared_type =
                resolve_type_in_project(&binding.declared_type, names, module, modules);
            for receiver in &mut binding.receivers {
                receiver.ty = resolve_type_in_project(&receiver.ty, names, module, modules);
            }
        }
        ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
            resolve_block_types_project(block, names, module, modules)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_block_types_project(&mut while_expression.body, names, module, modules)
        }
        ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
    }
}

fn resolve_block_types_project(
    block: &mut ScalarBlock,
    names: &BTreeMap<String, ScalarStructId>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    for item in &mut block.items {
        resolve_item_types_project(item, names, module, modules);
    }
}

fn resolve_type_in_project(
    ty: &ScalarType,
    names: &BTreeMap<String, ScalarStructId>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => {
            if let Some(id) = names.get(name) {
                return ScalarType::Struct(id.clone());
            }
            ScalarType::Named {
                name: name.clone(),
                span: *span,
            }
        }
        ScalarType::Qualified {
            receiver,
            receiver_span,
            member,
            member_span,
            span,
        } => match module
            .namespace_bindings
            .iter()
            .find(|namespace| namespace.binding == *receiver)
            .and_then(|namespace| {
                modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)
            })
            .and_then(|target| {
                target
                    .structs
                    .iter()
                    .find(|structure| structure.name == *member)
            }) {
            Some(structure) => ScalarType::Struct(structure.id.clone()),
            None => ScalarType::Qualified {
                receiver: receiver.clone(),
                receiver_span: *receiver_span,
                member: member.clone(),
                member_span: *member_span,
                span: *span,
            },
        },
        ScalarType::RawPointer(inner) => ScalarType::RawPointer(Box::new(resolve_type_in_project(
            inner, names, module, modules,
        ))),
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: resolve_type_in_project(&output.ty, names, module, modules),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| resolve_type_in_project(parameter, names, module, modules))
                .collect(),
        },
        value => value.clone(),
    }
}

fn recompute_struct_layout(structure: &mut ScalarStruct, structs: &[ScalarStruct]) {
    let mut offset = 0;
    let mut alignment = 1;
    for field in &mut structure.fields {
        field.layout = layout_for_type_in_structs(&field.ty, structs);
        alignment = alignment.max(field.layout.alignment);
        offset = align_offset(offset, field.layout.alignment);
        field.offset = offset;
        offset += field.layout.size;
    }
    structure.layout = ScalarLayout {
        size: align_offset(offset, alignment),
        alignment,
    };
}

fn recompute_struct_layout_project(structure: &mut ScalarStruct, modules: &[ScalarModule]) {
    let mut offset = 0;
    let mut alignment = 1;
    for field in &mut structure.fields {
        field.layout = layout_for_type_in_modules(&field.ty, modules);
        alignment = alignment.max(field.layout.alignment);
        offset = align_offset(offset, field.layout.alignment);
        field.offset = offset;
        offset += field.layout.size;
    }
    structure.layout = ScalarLayout {
        size: align_offset(offset, alignment),
        alignment,
    };
}

fn layout_for_type_in_structs(ty: &ScalarType, structs: &[ScalarStruct]) -> ScalarLayout {
    match ty {
        ScalarType::Struct(id) => structs
            .iter()
            .find(|structure| structure.id == *id)
            .map(|structure| structure.layout.clone())
            .expect("resolved local struct layout"),
        _ => layout_for_type(ty),
    }
}

fn layout_for_type_in_modules(ty: &ScalarType, modules: &[ScalarModule]) -> ScalarLayout {
    match ty {
        ScalarType::Struct(id) => modules
            .iter()
            .flat_map(|module| module.structs.iter())
            .find(|structure| structure.id == *id)
            .map(|structure| structure.layout.clone())
            .expect("resolved project struct layout"),
        _ => layout_for_type(ty),
    }
}

fn resolve_module_places(module: &mut ScalarModule, modules: &[ScalarModule]) {
    let context = module.clone();
    let declarations = module
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => {
                Some((binding.name.clone(), binding.declared_type.clone()))
            }
            ScalarItem::Function(function) => {
                Some((function.name.clone(), function.signature.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for item in &mut module.items {
        match item {
            ScalarItem::Binding(binding) => {
                resolve_expression_module_places(
                    &mut binding.value,
                    &declarations,
                    &context,
                    modules,
                );
            }
            ScalarItem::Function(function) => {
                let mut scope = declarations.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        scope.insert(name.clone(), ty.clone());
                    }
                }
                resolve_block_module_places(&mut function.body, &mut scope, &context, modules);
            }
            ScalarItem::Executable(item) => {
                let mut scope = declarations.clone();
                resolve_item_module_places(item, &mut scope, &context, modules);
            }
            ScalarItem::Extern(_) | ScalarItem::Namespace(_) => {}
        }
    }
}

fn resolve_item_module_places(
    item: &mut ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            resolve_expression_module_places(&mut binding.value, scope, module, modules);
            scope.insert(binding.name.clone(), binding.declared_type.clone());
        }
        ScalarBlockItem::Expression(expression) => {
            resolve_expression_module_places(expression, scope, module, modules)
        }
        ScalarBlockItem::Assignment(assignment) => {
            resolve_expression_module_places(&mut assignment.value, scope, module, modules)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_expression_module_places(
                &mut while_expression.condition,
                scope,
                module,
                modules,
            );
            resolve_block_module_places(&mut while_expression.body, scope, module, modules);
        }
    }
}

fn resolve_block_module_places(
    block: &mut ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    for item in &mut block.items {
        resolve_item_module_places(item, scope, module, modules);
    }
}

fn resolve_expression_module_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    match expression {
        ScalarExpression::RawAddress { place, .. } => {
            resolve_module_place(place, scope, module, modules)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_module_places(&mut field.value, scope, module, modules);
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            resolve_expression_module_places(left, scope, module, modules);
            resolve_expression_module_places(right, scope, module, modules);
        }
        ScalarExpression::Unary { operand, .. } => {
            resolve_expression_module_places(operand, scope, module, modules);
        }
        ScalarExpression::Call { arguments, .. } => {
            for argument in arguments {
                resolve_expression_module_places(argument, scope, module, modules);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_module_places(condition, scope, module, modules);
            let mut then_scope = scope.clone();
            resolve_block_module_places(then_branch, &mut then_scope, module, modules);
            let mut else_scope = scope.clone();
            resolve_block_module_places(else_branch, &mut else_scope, module, modules);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_module_places(condition, scope, module, modules);
            let mut then_scope = scope.clone();
            resolve_block_module_places(then_branch, &mut then_scope, module, modules);
        }
        ScalarExpression::Block(block) => {
            let mut scope = scope.clone();
            resolve_block_module_places(block, &mut scope, module, modules);
        }
        ScalarExpression::Name { .. }
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

fn resolve_module_place(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            resolve_expression_module_places(pointer, scope, module, modules)
        }
        ScalarPlace::Field { base, field, .. } => {
            resolve_module_place(base, scope, module, modules);
            let base_type = place_type_in_module(base, scope, module, modules, &mut Vec::new());
            let structure = match base_type {
                ScalarType::Struct(id) => modules
                    .iter()
                    .find_map(|module| module.structs.iter().find(|structure| structure.id == id)),
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => modules.iter().find_map(|module| {
                        module.structs.iter().find(|structure| structure.id == *id)
                    }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(declared) = structure.and_then(|structure| {
                structure
                    .fields
                    .iter()
                    .find(|candidate| {
                        matches!(field, ScalarFieldReference::Unresolved { name, .. } if candidate.name == *name)
                    })
            }) {
                *field = ScalarFieldReference::Resolved(declared.id.clone());
            }
        }
    }
}

fn validate_module_type(
    module: &ScalarModule,
    ty: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { .. } | ScalarType::Qualified { .. } => diagnostics.push(
            module_diagnostic(module, "B0003", "unknown scalar type", span),
        ),
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_module_type(module, &output.ty, output.span, diagnostics);
            }
            for parameter in parameters {
                validate_module_type(module, parameter, span, diagnostics);
            }
        }
        ScalarType::Unit
        | ScalarType::Bool
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
        | ScalarType::ArtifactId => {}
        ScalarType::RawPointer(inner)
            if matches!(inner.as_ref(), ScalarType::U8 | ScalarType::Struct(_)) => {}
        ScalarType::RawPointer(_) => diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "unsupported raw pointer type",
            span,
        )),
        ScalarType::Struct(_) => {}
        ScalarType::Error => diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "invalid scalar type",
            span,
        )),
    }
}

fn validate_extern(
    module: &ScalarModule,
    extern_decl: &ScalarExtern,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let valid_binding = extern_decl.kind == "wasm"
        && matches!(extern_decl.actual_module, ScalarExternModule::Valid(_))
        && !extern_decl.functions.is_empty();
    if !valid_binding {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "unsupported extern declaration",
            extern_decl.span,
        ));
    }
    for function in &extern_decl.functions {
        validate_module_type(
            module,
            &function.signature,
            function.signature_span,
            diagnostics,
        );
    }
}

fn call_output_sequence_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) -> Option<ScalarOutputSequence> {
    let ScalarExpression::Call { receiver, name, .. } = expression else {
        return None;
    };
    let callable = match receiver {
        None => scope.get(name),
        Some(binding) => {
            let namespace = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding);
            if let Some(namespace) = namespace {
                modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)
                    .and_then(|target| target.members.get(name))
            } else {
                module.items.iter().find_map(|item| match item {
                    ScalarItem::Extern(extern_decl) if extern_decl.binding == *binding => {
                        extern_decl
                            .functions
                            .iter()
                            .find(|function| function.name == *name)
                            .map(|function| &function.signature)
                    }
                    _ => None,
                })
            }
        }
    }?;
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs.clone())
}

fn call_output_sequence(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) -> Option<ScalarOutputSequence> {
    let ScalarExpression::Call { receiver, name, .. } = expression else {
        return None;
    };
    let callable = if receiver.is_none() {
        scope.get(name)
    } else {
        program.items.iter().find_map(|item| match item {
            ScalarItem::Extern(extern_decl)
                if receiver.as_deref() == Some(extern_decl.binding.as_str()) =>
            {
                extern_decl
                    .functions
                    .iter()
                    .find(|function| function.name == *name)
                    .map(|function| &function.signature)
            }
            _ => None,
        })
    }?;
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs.clone())
}

fn scalar_call_result(outputs: &ScalarOutputSequence) -> ScalarType {
    outputs
        .outputs
        .first()
        .map_or(ScalarType::Unit, |output| output.ty.clone())
}

fn scalar_call_result_in_module(outputs: &ScalarOutputSequence) -> ScalarType {
    outputs
        .outputs
        .first()
        .map_or(ScalarType::Unit, |output| output.ty.clone())
}

fn reject_multi_output_statement_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if let Some(outputs) = call_output_sequence_in_module(expression, scope, module, modules) {
        if outputs.outputs.len() > 1 {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "multi-output call requires output receivers",
                outputs.span,
            ));
        }
    }
}

fn reject_multi_output_statement(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if let Some(outputs) = call_output_sequence(expression, scope, program) {
        if outputs.outputs.len() > 1 {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "multi-output call requires output receivers",
                outputs.span,
            ));
        }
    }
}

fn expression_span(expression: &ScalarExpression) -> ByteSpan {
    match expression {
        ScalarExpression::Name { span, .. }
        | ScalarExpression::Integer { span, .. }
        | ScalarExpression::InvalidInteger { span, .. }
        | ScalarExpression::Float { span, .. }
        | ScalarExpression::InvalidFloat { span }
        | ScalarExpression::Boolean { span, .. }
        | ScalarExpression::Char { span, .. }
        | ScalarExpression::Utf8 { span, .. }
        | ScalarExpression::RawAddress { span, .. }
        | ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::Binary { span, .. }
        | ScalarExpression::Unary { span, .. }
        | ScalarExpression::Call { span, .. }
        | ScalarExpression::If { span, .. }
        | ScalarExpression::UnitIf { span, .. } => *span,
        ScalarExpression::Member { span, .. } => *span,
        ScalarExpression::Block(block) => block.span,
    }
}

fn validate_output_receivers_in_module(
    expression: &ScalarExpression,
    receivers: &[ScalarOutputReceiver],
    binding_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let Some(outputs) = call_output_sequence_in_module(expression, scope, module, modules) else {
        return;
    };
    if receivers.len() != outputs.outputs.len() {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            if receivers.len() > outputs.outputs.len() {
                "call has fewer outputs than receivers"
            } else {
                "call has more outputs than receivers"
            },
            binding_span,
        ));
    }
    for (receiver, output) in receivers.iter().zip(&outputs.outputs) {
        expect_module_type(module, &output.ty, &receiver.ty, receiver.span, diagnostics);
    }
}

fn validate_output_receivers(
    expression: &ScalarExpression,
    receivers: &[ScalarOutputReceiver],
    binding_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let Some(outputs) = call_output_sequence(expression, scope, program) else {
        return;
    };
    if receivers.len() != outputs.outputs.len() {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            if receivers.len() > outputs.outputs.len() {
                "call has fewer outputs than receivers"
            } else {
                "call has more outputs than receivers"
            },
            binding_span,
        ));
    }
    for (receiver, output) in receivers.iter().zip(&outputs.outputs) {
        expect_type(
            program,
            &output.ty,
            &receiver.ty,
            receiver.span,
            diagnostics,
        );
    }
}

fn validate_assignment_outputs_in_module(
    assignment: &ScalarAssignment,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let Some(outputs) = call_output_sequence_in_module(&assignment.value, scope, module, modules)
    else {
        return;
    };
    if assignment.targets.len() != outputs.outputs.len() {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            if assignment.targets.len() > outputs.outputs.len() {
                "call has fewer outputs than assignment targets"
            } else {
                "call has more outputs than assignment targets"
            },
            assignment.span,
        ));
    }
    for (target, output) in assignment.targets.iter().zip(&outputs.outputs) {
        let expected = match &target.receiver {
            None => scope.get(&target.target),
            Some(receiver) => module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
                .and_then(|namespace| {
                    modules
                        .iter()
                        .find(|candidate| candidate.source == namespace.target)
                })
                .and_then(|target_module| target_module.members.get(&target.target)),
        };
        if let Some(expected) = expected {
            expect_module_type(
                module,
                expected,
                &output.ty,
                target.target_span,
                diagnostics,
            );
        }
    }
}

fn validate_assignment_outputs(
    assignment: &ScalarAssignment,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let Some(outputs) = call_output_sequence(&assignment.value, scope, program) else {
        return;
    };
    if assignment.targets.len() != outputs.outputs.len() {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            if assignment.targets.len() > outputs.outputs.len() {
                "call has fewer outputs than assignment targets"
            } else {
                "call has more outputs than assignment targets"
            },
            assignment.span,
        ));
    }
    for (target, output) in assignment.targets.iter().zip(&outputs.outputs) {
        if let Some(expected) = scope.get(&target.target) {
            expect_type(
                program,
                expected,
                &output.ty,
                target.target_span,
                diagnostics,
            );
        }
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
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
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
            unsafe_context,
        );
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

fn block_type_in_module_expected(
    block: &ScalarBlock,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = match item {
            ScalarBlockItem::Expression(expression)
                if index + 1 == block.items.len() && !block.terminated_items[index] =>
            {
                expression_type_in_module_expected(
                    expression,
                    expected,
                    &scope,
                    &visible_names,
                    &folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                )
            }
            _ => block_item_type_in_module(
                item,
                &mut scope,
                &mut visible_names,
                &mut folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            ),
        };
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
    unsafe_context: bool,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let actual = expression_type_in_module_expected(
                &binding.value,
                Some(&binding.declared_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            validate_output_receivers_in_module(
                &binding.value,
                &binding.receivers,
                binding.span,
                scope,
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
                for receiver in &binding.receivers {
                    scope.insert(receiver.name.clone(), receiver.ty.clone());
                }
            }
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => {
            reject_multi_output_statement_in_module(
                expression,
                scope,
                module,
                modules,
                diagnostics,
            );
            expression_type_in_module(
                expression,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            )
        }
        ScalarBlockItem::Assignment(assignment) => assignment_type_in_module(
            assignment,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
        ScalarBlockItem::While(while_expression) => while_type_in_module(
            while_expression,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
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
    unsafe_context: bool,
) -> ScalarType {
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
    let actual = expression_type_in_module_expected(
        &assignment.value,
        Some(expected),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    validate_assignment_outputs_in_module(assignment, scope, module, modules, diagnostics);
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
    unsafe_context: bool,
) -> ScalarType {
    let condition = expression_type_in_module(
        &while_expression.condition,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    if !is_error_type(&condition) && !scalar_type_equal(&condition, &ScalarType::Bool) {
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
        unsafe_context,
    );
    ScalarType::Unit
}

fn type_core_cast_in_module(
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "core.cast requires one destination type argument",
            span,
        ));
        return ScalarType::Error;
    }
    if type_arguments[0].ty != ScalarType::U32 {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "core.cast supports only a u32 destination type",
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 2 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "core.cast requires one value and one static mode",
            span,
        ));
        return ScalarType::Error;
    }
    let actual = expression_type_in_module_expected(
        &arguments[0],
        Some(&ScalarType::U64),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    validate_exact_u32_source_in_module(&arguments[0], module, diagnostics);
    expect_module_type(
        module,
        &ScalarType::U64,
        &actual,
        expression_span(&arguments[0]),
        diagnostics,
    );
    if !matches!(&arguments[1], ScalarExpression::Utf8 { value, .. } if value == b"exact") {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "core.cast requires the static mode \"exact\"",
            expression_span(&arguments[1]),
        ));
    }
    if is_error_type(&actual) {
        ScalarType::Error
    } else {
        ScalarType::U32
    }
}

fn type_core_cast(
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "core.cast requires one destination type argument",
            span,
        ));
        return ScalarType::Error;
    }
    if type_arguments[0].ty != ScalarType::U32 {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "core.cast supports only a u32 destination type",
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 2 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "core.cast requires one value and one static mode",
            span,
        ));
        return ScalarType::Error;
    }
    let actual = expression_type_expected(
        &arguments[0],
        &ScalarType::U64,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    validate_exact_u32_source(program, &arguments[0], diagnostics);
    expect_type(
        program,
        &ScalarType::U64,
        &actual,
        expression_span(&arguments[0]),
        diagnostics,
    );
    if !matches!(&arguments[1], ScalarExpression::Utf8 { value, .. } if value == b"exact") {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "core.cast requires the static mode \"exact\"",
            expression_span(&arguments[1]),
        ));
    }
    if is_error_type(&actual) {
        ScalarType::Error
    } else {
        ScalarType::U32
    }
}

fn validate_exact_u32_source_in_module(
    expression: &ScalarExpression,
    module: &ScalarModule,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if let ScalarExpression::Integer { value, span } = expression {
        if value > &BigInt::from(u32::MAX) {
            diagnostics.push(module_diagnostic(
                module,
                "B0010",
                "integer literal is outside the resolved target type range",
                *span,
            ));
        }
    }
}

fn validate_exact_u32_source(
    program: &ScalarProgram,
    expression: &ScalarExpression,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if let ScalarExpression::Integer { value, span } = expression {
        if value > &BigInt::from(u32::MAX) {
            diagnostics.push(diagnostic(
                program,
                "B0010",
                "integer literal is outside the resolved target type range",
                *span,
            ));
        }
    }
}

fn expression_type_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match expression {
        ScalarExpression::Name { name, span } if name == "null" => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "null requires a pointer context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::RawAddress { place, span } => {
            if !unsafe_context {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0012",
                    "raw address requires an unsafe block",
                    *span,
                ));
            }
            ScalarType::RawPointer(Box::new(place_type_in_module(
                place,
                scope,
                module,
                modules,
                diagnostics,
            )))
        }
        ScalarExpression::StructLiteral { .. } => ScalarType::Error,
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
            if let Some(receiver_type) = scope.get(receiver) {
                if matches!(receiver_type, ScalarType::Struct(_))
                    || matches!(
                        receiver_type,
                        ScalarType::RawPointer(inner)
                            if matches!(inner.as_ref(), ScalarType::Struct(_))
                    )
                {
                    return field_type_in_module(
                        receiver_type,
                        name,
                        *name_span,
                        *span,
                        module,
                        modules,
                        diagnostics,
                    );
                }
            }
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
        ScalarExpression::Float { .. } => ScalarType::Error,
        ScalarExpression::InvalidFloat { .. } => ScalarType::Error,
        ScalarExpression::InvalidInteger {
            span: _,
            error_span,
        } => {
            if error_span.is_some() {
                ScalarType::Error
            } else {
                ScalarType::I32
            }
        }
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Char { .. } => ScalarType::Char,
        ScalarExpression::Utf8 { span, .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "string literal requires a std.utf8 context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Unary {
            operator,
            operand,
            span,
        } => {
            if matches!(operator, UnaryOperator::Negate) {
                if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                    let value = -value;
                    validate_integer_range(module, &value, *span, diagnostics);
                    return ScalarType::I32;
                }
            }
            let operand_type = expression_type_in_module(
                operand,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            match operator {
                UnaryOperator::LogicalNot => {
                    expect_module_type(
                        module,
                        &ScalarType::Bool,
                        &operand_type,
                        *span,
                        diagnostics,
                    );
                    ScalarType::Bool
                }
                UnaryOperator::BitwiseNot => {
                    expect_module_integer(module, &operand_type, *span, diagnostics);
                    operand_type
                }
                UnaryOperator::Negate => {
                    expect_module_integer(module, &operand_type, *span, diagnostics);
                    operand_type
                }
            }
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            if comparison && (is_null_expression(left) || is_null_expression(right)) {
                return expression_type_in_module_expected(
                    expression,
                    None,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            let left_type = expression_type_in_module(
                left,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let right_type = expression_type_in_module(
                right,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
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
            let bitwise = matches!(
                operator,
                BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
            );
            if boolean {
                expect_module_type(module, &ScalarType::Bool, &left_type, *span, diagnostics);
                expect_module_type(module, &ScalarType::Bool, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if comparison {
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if bitwise {
                expect_module_integer(module, &left_type, *span, diagnostics);
                expect_module_integer(module, &right_type, *span, diagnostics);
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                left_type
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
            type_arguments,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "this_artifact_id" {
                if !arguments.is_empty() {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                return ScalarType::ArtifactId;
            }
            if receiver.as_deref() == Some("core") && name == "declare_artifact" {
                if arguments.len() != 1 {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                let actual = expression_type_in_module_expected(
                    &arguments[0],
                    Some(&std_utf8_type(module, modules, *span, diagnostics)),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                let expected = std_utf8_type(module, modules, *span, diagnostics);
                expect_module_type(module, &expected, &actual, *span, diagnostics);
                return if is_error_type(&actual) {
                    ScalarType::Error
                } else {
                    ScalarType::ArtifactId
                };
            }
            if receiver.as_deref() == Some("core") && name == "cast" {
                return type_core_cast_in_module(
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            let (callable, target, unsafe_callable) = match receiver {
                None => (scope.get(name), module, false),
                Some(binding) => {
                    let namespace = module
                        .namespace_bindings
                        .iter()
                        .find(|namespace| namespace.binding == *binding);
                    if let Some(namespace) = namespace {
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
                        (Some(callable), target, false)
                    } else {
                        let extern_decl = module.items.iter().find_map(|item| match item {
                            ScalarItem::Extern(extern_decl) if extern_decl.binding == *binding => {
                                Some(extern_decl)
                            }
                            _ => None,
                        });
                        let Some(extern_decl) = extern_decl else {
                            diagnostics.push(module_diagnostic(
                                module,
                                "B0001",
                                "unknown callable name",
                                *span,
                            ));
                            return ScalarType::Error;
                        };
                        let Some(function) = extern_decl
                            .functions
                            .iter()
                            .find(|function| function.name == *name)
                        else {
                            diagnostics.push(module_diagnostic(
                                module,
                                "B0001",
                                "unknown callable name",
                                *span,
                            ));
                            return ScalarType::Error;
                        };
                        (Some(&function.signature), module, function.unsafe_marker)
                    }
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
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
            else {
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
                let actual = expression_type_in_module_expected(
                    argument,
                    Some(parameter),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                expect_module_type(target, parameter, &actual, *span, diagnostics);
                error_argument |= is_error_type(&actual);
            }
            if unsafe_callable && !unsafe_context {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0012",
                    "unsafe extern call requires an unsafe block",
                    *span,
                ));
            }
            if error_argument {
                ScalarType::Error
            } else {
                scalar_call_result_in_module(outputs)
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
                unsafe_context,
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
                unsafe_context,
            );
            let else_type = block_type_in_module(
                else_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && !scalar_type_equal(&then_type, &else_type)
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
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            let condition_type = expression_type_in_module(
                condition,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            expect_module_type(
                module,
                &ScalarType::Bool,
                &condition_type,
                expression_span(condition),
                diagnostics,
            );
            block_type_in_module(
                then_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            ScalarType::Unit
        }
        ScalarExpression::Block(block) => block_type_in_module(
            block,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
    }
}

fn expression_type_in_module_expected(
    expression: &ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if let ScalarExpression::Unary {
        operator: UnaryOperator::Negate,
        operand,
        span,
    } = expression
    {
        if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
            if let Some(
                expected @ (ScalarType::I8
                | ScalarType::I16
                | ScalarType::I32
                | ScalarType::I64
                | ScalarType::I128
                | ScalarType::U8
                | ScalarType::U16
                | ScalarType::U32
                | ScalarType::U64
                | ScalarType::U128),
            ) = expected
            {
                let value = -value;
                validate_integer_range_for_type(module, &value, *span, expected, diagnostics);
                return expected.clone();
            }
        }
    }
    if let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        span,
    } = expression
    {
        let condition_type = expression_type_in_module(
            condition,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        if !is_error_type(&condition_type) && !scalar_type_equal(&condition_type, &ScalarType::Bool)
        {
            diagnostics.push(module_diagnostic(
                module,
                "B0005",
                "conditional expression requires bool",
                *span,
            ));
        }
        let then_type = block_type_in_module_expected(
            then_branch,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        let else_type = block_type_in_module_expected(
            else_branch,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        if !is_error_type(&then_type)
            && !is_error_type(&else_type)
            && !scalar_type_equal(&then_type, &else_type)
        {
            diagnostics.push(module_diagnostic(
                module,
                "B0006",
                "conditional branches must have equal types",
                *span,
            ));
        }
        return then_type;
    }
    if let ScalarExpression::Name { name, span } = expression {
        if name == "null" {
            if let Some(ScalarType::RawPointer(pointer)) = expected {
                return ScalarType::RawPointer(pointer.clone());
            }
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "null requires a pointer context",
                *span,
            ));
            return ScalarType::Error;
        }
    }
    if let ScalarExpression::Utf8 { span, .. } = expression {
        let Some(expected) = expected else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "string literal requires a std.utf8 context",
                *span,
            ));
            return ScalarType::Error;
        };
        let std_utf8 = std_utf8_type(module, modules, *span, diagnostics);
        if scalar_type_equal(&std_utf8, expected) {
            return std_utf8;
        }
        expect_module_type(module, &std_utf8, expected, *span, diagnostics);
        return ScalarType::Error;
    }
    if let ScalarExpression::StructLiteral { fields, span } = expression {
        let Some(ScalarType::Struct(id)) = expected else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "struct literal requires a struct context",
                *span,
            ));
            return ScalarType::Error;
        };
        let Some(structure) = modules
            .iter()
            .flat_map(|module| module.structs.iter())
            .find(|structure| structure.id == *id)
        else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "invalid struct type",
                *span,
            ));
            return ScalarType::Error;
        };
        let mut seen = BTreeSet::new();
        let mut initialized = BTreeSet::new();
        for field in fields {
            if !seen.insert(field.name.clone()) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "duplicate struct literal field",
                    field.name_span,
                ));
                continue;
            }
            let Some(declared) = structure.fields.iter().find(|item| item.name == field.name)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "M0002",
                    "unknown struct field",
                    field.name_span,
                ));
                continue;
            };
            initialized.insert(field.name.clone());
            let actual = expression_type_in_module_expected(
                &field.value,
                Some(&declared.ty),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            expect_module_type(module, &declared.ty, &actual, field.span, diagnostics);
        }
        if initialized.len() != structure.fields.len() {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "struct literal must initialize every field",
                *span,
            ));
        }
        return ScalarType::Struct(id.clone());
    }
    if let ScalarExpression::Integer { value, span } = expression {
        if let Some(
            expected @ (ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128),
        ) = expected
        {
            validate_integer_range_for_type(module, value, *span, expected, diagnostics);
            return expected.clone();
        }
    }
    if let ScalarExpression::Float { .. } = expression {
        if matches!(expected, Some(ScalarType::F32 | ScalarType::F64)) {
            if let ScalarExpression::Float { value, span, .. } = expression {
                if matches!(expected, Some(ScalarType::F32)) && !(*value as f32).is_finite() {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0010",
                        "floating-point literal is outside the f32 range",
                        *span,
                    ));
                    return ScalarType::Error;
                }
            }
            return expected.cloned().expect("float context");
        }
    }
    if let ScalarExpression::Block(block) = expression {
        return block_type_in_module_expected(
            block,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
    }
    if let ScalarExpression::Binary {
        operator,
        left,
        right,
        span,
    } = expression
    {
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
        let bitwise = matches!(
            operator,
            BinaryOperator::BitAnd
                | BinaryOperator::BitOr
                | BinaryOperator::BitXor
                | BinaryOperator::ShiftLeft
                | BinaryOperator::ShiftRight
        );
        let arithmetic_context = expected.filter(|ty| {
            matches!(
                ty,
                ScalarType::I8
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
            )
        });
        let bool_context = ScalarType::Bool;
        let (left_type, right_type) = if comparison && is_null_expression(left) {
            let right_type = expression_type_in_module_expected(
                right,
                None,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let left_type = expression_type_in_module_expected(
                left,
                Some(&right_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        } else if comparison && matches!(left.as_ref(), ScalarExpression::Float { .. }) {
            let right_type = expression_type_in_module(
                right,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let left_type = expression_type_in_module_expected(
                left,
                Some(&right_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        } else if comparison && matches!(right.as_ref(), ScalarExpression::Float { .. }) {
            let left_type = expression_type_in_module(
                left,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let right_type = expression_type_in_module_expected(
                right,
                Some(&left_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        } else {
            let left_type = expression_type_in_module_expected(
                left,
                if boolean {
                    Some(&bool_context)
                } else {
                    arithmetic_context
                },
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let right_type = expression_type_in_module_expected(
                right,
                if boolean {
                    Some(&bool_context)
                } else if comparison && is_null_expression(right) {
                    Some(&left_type)
                } else {
                    arithmetic_context
                },
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        };
        if boolean {
            expect_module_type(module, &ScalarType::Bool, &left_type, *span, diagnostics);
            expect_module_type(module, &ScalarType::Bool, &right_type, *span, diagnostics);
            return ScalarType::Bool;
        }
        if is_error_type(&left_type) || is_error_type(&right_type) {
            return ScalarType::Error;
        }
        if comparison {
            expect_module_type(module, &left_type, &right_type, *span, diagnostics);
            return ScalarType::Bool;
        }
        if bitwise {
            expect_module_integer(module, &left_type, *span, diagnostics);
            expect_module_integer(module, &right_type, *span, diagnostics);
            expect_module_type(module, &left_type, &right_type, *span, diagnostics);
            return left_type;
        }
        expect_module_type(
            module,
            arithmetic_context.unwrap_or(&ScalarType::I32),
            &left_type,
            *span,
            diagnostics,
        );
        expect_module_type(module, &left_type, &right_type, *span, diagnostics);
        if let Some(expected) = arithmetic_context {
            return expected.clone();
        }
        return ScalarType::I32;
    }
    expression_type_in_module(
        expression,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    )
}

fn std_utf8_type(
    module: &ScalarModule,
    modules: &[ScalarModule],
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let structure = module
        .namespace_bindings
        .iter()
        .find(|binding| binding.binding == "std")
        .and_then(|binding| {
            modules
                .iter()
                .find(|candidate| candidate.source == binding.target)
        })
        .and_then(|stdlib| {
            stdlib
                .structs
                .iter()
                .find(|structure| structure.name == "utf8")
        });
    match structure {
        Some(structure) => ScalarType::Struct(structure.id.clone()),
        None => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "std.utf8 is unavailable",
                span,
            ));
            ScalarType::Error
        }
    }
}

fn field_type_in_module(
    receiver: &ScalarType,
    name: &str,
    name_span: ByteSpan,
    span: ByteSpan,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let structure_id = match receiver {
        ScalarType::Struct(id) => id,
        ScalarType::RawPointer(inner) => match inner.as_ref() {
            ScalarType::Struct(id) => id,
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    span,
                ));
                return ScalarType::Error;
            }
        },
        _ => unreachable!("field receiver must be a struct or pointer to a struct"),
    };
    let Some(structure) = modules
        .iter()
        .flat_map(|module| module.structs.iter())
        .find(|structure| structure.id == *structure_id)
    else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            span,
        ));
        return ScalarType::Error;
    };
    let Some(field) = structure.fields.iter().find(|field| field.name == name) else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            name_span,
        ));
        return ScalarType::Error;
    };
    field.ty.clone()
}

fn place_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarPlace::Field {
            base, field, span, ..
        } => {
            let base_type = place_type(base, scope, program, diagnostics);
            let id = match base_type {
                ScalarType::Struct(id) => id,
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => id.clone(),
                    _ => {
                        diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                        return ScalarType::Error;
                    }
                },
                _ => {
                    diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                    return ScalarType::Error;
                }
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let Some(resolved_field) = program
                .structs
                .get(id.index)
                .filter(|structure| field.structure == structure.id)
                .and_then(|structure| structure.fields.get(field.index))
                .filter(|resolved_field| resolved_field.id == *field)
            else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            resolved_field.ty.clone()
        }
        ScalarPlace::Dereference { pointer, span } => match expression_type(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            true,
        ) {
            ScalarType::RawPointer(inner) => *inner,
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "cannot dereference value",
                    *span,
                ));
                ScalarType::Error
            }
        },
    }
}

fn place_type_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarPlace::Dereference { pointer, span } => match expression_type_in_module(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            module,
            modules,
            diagnostics,
            true,
        ) {
            ScalarType::RawPointer(inner) => *inner,
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "cannot dereference value",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Field {
            base, field, span, ..
        } => {
            let base_type = place_type_in_module(base, scope, module, modules, diagnostics);
            let id = match base_type {
                ScalarType::Struct(id) => id,
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => id.clone(),
                    _ => {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0003",
                            "value has no field",
                            *span,
                        ));
                        return ScalarType::Error;
                    }
                },
                _ => {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "value has no field",
                        *span,
                    ));
                    return ScalarType::Error;
                }
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            };
            if field.structure != id {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            }
            let resolved = modules
                .iter()
                .flat_map(|module| module.structs.iter())
                .find(|structure| field.structure == structure.id)
                .and_then(|structure| structure.fields.get(field.index))
                .filter(|resolved_field| resolved_field.id == *field)
                .map(|field| field.ty.clone());
            match resolved {
                Some(ty) => ty,
                None => {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "value has no field",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
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
    if !is_error_type(expected) && !is_error_type(actual) && !scalar_type_equal(expected, actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
}

fn is_integer_type(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128
    )
}

fn expect_module_integer(
    module: &ScalarModule,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(actual) && !is_integer_type(actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "integer operation requires integer operands",
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

fn derive_extern(node: &CstNode) -> ScalarExtern {
    let identifiers: Vec<CstToken> = node
        .children_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect();
    let module = direct_token(node, SyntaxKind::String).expect("extern module");
    let functions = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::ExternFunction)
        .map(|callable| {
            let signature = callable
                .children()
                .find(|child| child.kind() == SyntaxKind::CallableType)
                .expect("extern signature");
            let name = callable
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .next()
                .expect("extern symbol");
            ScalarExternFunction {
                name: name.text().to_owned(),
                signature: derive_type(&signature),
                unsafe_marker: callable
                    .children_with_tokens()
                    .any(|element| element.kind() == SyntaxKind::Unsafe),
                name_span: token_span(&name),
                signature_span: wosy_syntax::byte_span(&signature),
                span: wosy_syntax::byte_span(&callable),
            }
        })
        .collect();
    ScalarExtern {
        binding: identifiers[0].text().to_owned(),
        kind: identifiers[1].text().to_owned(),
        actual_module: match decode_string(module.text()) {
            Ok(value) => ScalarExternModule::Valid(value),
            Err((start, length)) => ScalarExternModule::Invalid {
                span: token_span(&module),
                error_span: string_error_span(&module, start, length),
            },
        },
        functions,
        binding_span: token_span(&identifiers[0]),
        module_span: token_span(&module),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_binding(node: &CstNode) -> ScalarBinding {
    let receiver_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::ReceiverList)
        .expect("receiver list");
    let receivers = receiver_list
        .children()
        .filter(|child| child.kind() == SyntaxKind::Receiver)
        .map(|receiver| {
            let ty_node = direct_nodes(&receiver)
                .into_iter()
                .next()
                .expect("receiver type");
            let name = direct_token(&receiver, SyntaxKind::Identifier).expect("receiver name");
            ScalarOutputReceiver {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                ty: derive_type(&ty_node),
                span: wosy_syntax::byte_span(&receiver),
            }
        })
        .collect::<Vec<_>>();
    let declared_type = receivers[0].ty.clone();
    let output_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::OutputList)
        .expect("output list");
    let outputs = output_list
        .descendants()
        .filter(|child| child.kind() == SyntaxKind::Expression)
        .map(|child| derive_expression(&child))
        .collect::<Vec<_>>();
    let value = outputs[0].clone();
    let name = receivers[0].name.clone();
    let output_values = receivers
        .iter()
        .enumerate()
        .map(|(position, receiver)| ScalarOutputValue {
            position,
            ty: receiver.ty.clone(),
            span: span_of(if outputs.len() == 1 {
                &value
            } else {
                &outputs[position]
            }),
            value: if outputs.len() == 1 {
                value.clone()
            } else {
                outputs[position].clone()
            },
        })
        .collect::<Vec<_>>();
    let output_sequence = ScalarOutputSequence {
        outputs: receivers
            .iter()
            .enumerate()
            .map(|(position, receiver)| ScalarOutput {
                ty: receiver.ty.clone(),
                span: output_values[position].span,
            })
            .collect(),
        span: wosy_syntax::byte_span(&output_list),
    };
    ScalarBinding {
        name,
        declared_type,
        value,
        receivers,
        output_sequence,
        output_values,
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_struct(node: CstNode, id: ScalarStructId) -> ScalarStruct {
    let name = direct_token(&node, SyntaxKind::Identifier).expect("struct name");
    let fields = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::StructField)
        .enumerate()
        .map(|(index, field)| {
            let ty = derive_type(&direct_nodes(&field).into_iter().next().expect("field type"));
            let field_name = direct_token(&field, SyntaxKind::Identifier).expect("field name");
            ScalarStructField {
                id: ScalarStructFieldId {
                    structure: id.clone(),
                    index,
                },
                name: field_name.text().to_owned(),
                name_span: token_span(&field_name),
                layout: layout_for_type(&ty),
                ty,
                declaration_index: index,
                offset: 0,
            }
        })
        .collect::<Vec<_>>();
    let mut offset = 0;
    let mut alignment = 1;
    let mut fields = fields;
    for field in &mut fields {
        alignment = alignment.max(field.layout.alignment);
        offset = align_offset(offset, field.layout.alignment);
        field.offset = offset;
        offset += field.layout.size;
    }
    let layout = ScalarLayout {
        size: align_offset(offset, alignment),
        alignment,
    };
    ScalarStruct {
        id,
        name: name.text().to_owned(),
        name_span: token_span(&name),
        fields,
        span: wosy_syntax::byte_span(&node),
        layout,
    }
}

fn align_offset(offset: u64, alignment: u64) -> u64 {
    (offset + alignment - 1) / alignment * alignment
}

fn layout_for_type(ty: &ScalarType) -> ScalarLayout {
    match ty {
        ScalarType::Bool | ScalarType::I8 | ScalarType::U8 => ScalarLayout {
            size: 1,
            alignment: 1,
        },
        ScalarType::I16 | ScalarType::U16 => ScalarLayout {
            size: 2,
            alignment: 2,
        },
        ScalarType::I32 | ScalarType::U32 | ScalarType::Char => ScalarLayout {
            size: 4,
            alignment: 4,
        },
        ScalarType::I64 | ScalarType::U64 => ScalarLayout {
            size: 8,
            alignment: 8,
        },
        ScalarType::I128 | ScalarType::U128 => ScalarLayout {
            size: 16,
            alignment: 16,
        },
        ScalarType::F32 => ScalarLayout {
            size: 4,
            alignment: 4,
        },
        ScalarType::F64 => ScalarLayout {
            size: 8,
            alignment: 8,
        },
        ScalarType::RawPointer(_) | ScalarType::ArtifactId => ScalarLayout {
            size: ScalarTargetLayout::WASM32.pointer_size,
            alignment: ScalarTargetLayout::WASM32.pointer_alignment,
        },
        _ => ScalarLayout {
            size: 0,
            alignment: 1,
        },
    }
}

fn output_sequence(node: &CstNode) -> ScalarOutputSequence {
    ScalarOutputSequence {
        outputs: node
            .children()
            .filter(|child| child.kind() == SyntaxKind::CallableOutput)
            .flat_map(|output| {
                let children = direct_nodes(&output);
                if children.is_empty() {
                    output
                        .children_with_tokens()
                        .filter_map(|element| match element {
                            NodeOrToken::Node(node)
                                if node.kind() == SyntaxKind::RawPointerType =>
                            {
                                Some(ScalarOutput {
                                    ty: derive_type(&node),
                                    span: wosy_syntax::byte_span(&node),
                                })
                            }
                            NodeOrToken::Token(token) if token.kind() == SyntaxKind::TypeName => {
                                Some(ScalarOutput {
                                    ty: type_from_name(token.text(), token_span(&token)),
                                    span: token_span(&token),
                                })
                            }
                            NodeOrToken::Node(node) if node.kind() == SyntaxKind::QualifiedType => {
                                Some(ScalarOutput {
                                    ty: derive_type(&node),
                                    span: wosy_syntax::byte_span(&node),
                                })
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                } else {
                    children
                        .into_iter()
                        .map(|child| ScalarOutput {
                            ty: derive_type(&child),
                            span: wosy_syntax::byte_span(&child),
                        })
                        .collect()
                }
            })
            .collect(),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_struct_literal(node: CstNode) -> Option<ScalarStructLiteral> {
    let literal = direct_nodes(&node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::StructLiteral)
        .or_else(|| {
            node.descendants()
                .find(|child| child.kind() == SyntaxKind::StructLiteral)
        })?;
    let fields = literal
        .children()
        .filter(|child| child.kind() == SyntaxKind::StructLiteralField)
        .map(|field| {
            let name = direct_token(&field, SyntaxKind::Identifier).expect("literal field name");
            let value = direct_nodes(&field)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Expression)
                .or_else(|| {
                    field
                        .descendants()
                        .find(|child| child.kind() == SyntaxKind::Expression)
                })
                .map(|child| derive_expression(&child))
                .expect("literal field value");
            ScalarStructLiteralField {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                value,
                span: wosy_syntax::byte_span(&field),
            }
        })
        .collect();
    Some(ScalarStructLiteral {
        fields,
        span: wosy_syntax::byte_span(&literal),
    })
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
    if node.kind() != SyntaxKind::RawPointerType {
        if let Some(qualified) = direct_nodes(node)
            .into_iter()
            .find(|child| child.kind() == SyntaxKind::QualifiedType)
        {
            return qualified_type(&qualified);
        }
    }
    if node.kind() == SyntaxKind::RawPointerType {
        if let Some(qualified) = direct_nodes(node)
            .into_iter()
            .find(|child| child.kind() == SyntaxKind::QualifiedType)
        {
            return ScalarType::RawPointer(Box::new(derive_type(&qualified)));
        }
        let token = direct_token(node, SyntaxKind::TypeName).expect("raw pointer type");
        let inner = type_from_name(token.text(), token_span(&token));
        return ScalarType::RawPointer(Box::new(inner));
    }
    let actual = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::CallableType)
        .unwrap_or_else(|| node.clone());
    if actual.kind() == SyntaxKind::CallableType {
        let outputs = output_sequence(&actual);
        let parameters = actual
            .children_with_tokens()
            .filter_map(|element| match element {
                NodeOrToken::Node(node) if node.kind() == SyntaxKind::RawPointerType => {
                    Some(derive_type(&node))
                }
                NodeOrToken::Node(node) if node.kind() == SyntaxKind::QualifiedType => {
                    Some(derive_type(&node))
                }
                NodeOrToken::Token(token) if token.kind() == SyntaxKind::TypeName => {
                    Some(type_from_name(token.text(), token_span(&token)))
                }
                _ => None,
            })
            .collect();
        ScalarType::Callable {
            outputs,
            parameters,
        }
    } else {
        match direct_nodes(node)
            .into_iter()
            .find(|child| child.kind() == SyntaxKind::RawPointerType)
        {
            Some(pointer) => derive_type(&pointer),
            None if node.kind() == SyntaxKind::QualifiedType => qualified_type(node),
            None => {
                let token = direct_token(node, SyntaxKind::TypeName).expect("type token");
                type_from_name(token.text(), token_span(&token))
            }
        }
    }
}

fn qualified_type(node: &CstNode) -> ScalarType {
    let identifiers = node
        .children_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect::<Vec<_>>();
    ScalarType::Qualified {
        receiver: identifiers[0].text().to_owned(),
        receiver_span: token_span(&identifiers[0]),
        member: identifiers[1].text().to_owned(),
        member_span: token_span(&identifiers[1]),
        span: wosy_syntax::byte_span(node),
    }
}

fn type_from_name(value: &str, span: ByteSpan) -> ScalarType {
    match value {
        "unit" => ScalarType::Unit,
        "bool" => ScalarType::Bool,
        "i8" => ScalarType::I8,
        "i16" => ScalarType::I16,
        "i32" => ScalarType::I32,
        "i64" => ScalarType::I64,
        "i128" => ScalarType::I128,
        "u8" => ScalarType::U8,
        "u16" => ScalarType::U16,
        "u32" => ScalarType::U32,
        "u64" => ScalarType::U64,
        "u128" => ScalarType::U128,
        "f32" => ScalarType::F32,
        "f64" => ScalarType::F64,
        "char" => ScalarType::Char,
        "artifact_id" => ScalarType::ArtifactId,
        value => ScalarType::Named {
            name: value.to_owned(),
            span,
        },
    }
}

fn decode_string(value: &str) -> Result<String, (usize, usize)> {
    let Some(quoted) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err((0, value.len()));
    };
    let mut decoded = String::new();
    let mut chars = quoted.char_indices();
    while let Some((index, character)) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let Some((escape_index, escape)) = chars.next() else {
            return Err((index, 1));
        };
        match escape {
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '\\' => decoded.push('\\'),
            'u' => {
                let Some((brace_index, brace)) = chars.next() else {
                    return Err((index, escape_index + escape.len_utf8() - index));
                };
                if brace != '{' {
                    return Err((index, brace_index + brace.len_utf8() - index));
                }
                let mut code_point = 0u32;
                let mut digit_count = 0;
                let mut end = brace_index + brace.len_utf8();
                let mut closed = false;
                while let Some((digit_index, digit)) = chars.next() {
                    end = digit_index + digit.len_utf8();
                    if digit == '}' {
                        closed = true;
                        break;
                    }
                    let Some(digit_value) = digit.to_digit(16) else {
                        return Err((index, end - index));
                    };
                    code_point = match code_point
                        .checked_mul(16)
                        .and_then(|value| value.checked_add(digit_value))
                    {
                        Some(value) => value,
                        None => return Err((index, end - index)),
                    };
                    digit_count += 1;
                }
                if !closed || digit_count == 0 {
                    return Err((index, end - index));
                }
                let Some(character) = char::from_u32(code_point) else {
                    return Err((index, end - index));
                };
                decoded.push(character);
            }
            _ => return Err((index, escape_index + escape.len_utf8() - index)),
        }
    }
    Ok(decoded)
}

fn decode_char(value: &str) -> Result<char, (usize, usize)> {
    let decoded = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or((0, value.len()))?;
    let decoded = decode_string(&format!("\"{}\"", decoded))
        .map_err(|(start, length)| (start.saturating_sub(1), length))?;
    let mut characters = decoded.chars();
    let character = characters
        .next()
        .ok_or((1, value.len().saturating_sub(2)))?;
    if characters.next().is_some() {
        return Err((1, value.len().saturating_sub(2)));
    }
    Ok(character)
}

fn string_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::String => {
                decode_string(token.text())
                    .err()
                    .map(|(start, length)| super::Diagnostic {
                        code: "B0003".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid string literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(
                                canonical.source.clone(),
                                string_error_span(&token, start, length),
                            ),
                            message: "invalid string literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
            }
            _ => None,
        })
        .collect()
}

fn char_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Char => {
                decode_char(token.text())
                    .err()
                    .map(|(start, length)| super::Diagnostic {
                        code: "B0003".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid character literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(
                                canonical.source.clone(),
                                string_error_span(&token, start, length),
                            ),
                            message: "invalid character literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
            }
            _ => None,
        })
        .collect()
}

fn integer_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Integer => {
                if parse_integer(token.text()).is_none() {
                    Some(super::Diagnostic {
                        code: "B0010".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid integer literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(canonical.source.clone(), token_span(&token)),
                            message: "invalid integer literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn parse_float(text: &str) -> Option<f64> {
    if text.starts_with('_') || text.ends_with('_') || text.contains("__") {
        return None;
    }
    let mut previous = None;
    for character in text.chars() {
        if character == '_'
            && matches!(
                previous,
                None | Some('.') | Some('e') | Some('E') | Some('+') | Some('-')
            )
        {
            return None;
        }
        previous = Some(character);
    }
    let value = text.replace('_', "").parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn float_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Float => {
                parse_float(token.text())
                    .is_none()
                    .then(|| super::Diagnostic {
                        code: "B0010".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid finite floating-point literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(canonical.source.clone(), token_span(&token)),
                            message: "invalid finite floating-point literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
            }
            _ => None,
        })
        .collect()
}

fn string_error_span(token: &CstToken, start: usize, length: usize) -> ByteSpan {
    let span = token_span(token);
    ByteSpan::new(
        span.start + 1 + start as u32,
        span.start + 1 + (start + length) as u32,
    )
}

fn derive_block(node: &CstNode) -> ScalarBlock {
    derive_block_with_context(node, false)
}

fn derive_block_with_context(node: &CstNode, unsafe_context: bool) -> ScalarBlock {
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
        unsafe_context,
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
    let targets_node = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::AssignmentTargets)
        .expect("assignment targets node");
    let targets = targets_node
        .children()
        .filter(|child| child.kind() == SyntaxKind::AssignmentTarget)
        .map(|target_node| {
            let identifiers: Vec<CstToken> = target_node
                .descendants_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            let target = identifiers.last().expect("assignment target name");
            ScalarAssignmentTarget {
                receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
                target: target.text().to_owned(),
                receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
                target_span: token_span(target),
            }
        })
        .collect::<Vec<_>>();
    let target_node = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::AssignmentTarget)
        .or_else(|| {
            node.descendants()
                .find(|child| child.kind() == SyntaxKind::AssignmentTarget)
        })
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
        .or_else(|| {
            node.descendants()
                .find(|child| child.kind() == SyntaxKind::Expression)
        })
        .map(|child| derive_expression(&child))
        .expect("assignment value");
    ScalarAssignment {
        receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
        target: target.text().to_owned(),
        receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
        target_span: token_span(&target),
        targets,
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
        SyntaxKind::UnitIfExpr => {
            let children = direct_nodes(&actual);
            ScalarExpression::UnitIf {
                condition: Box::new(derive_expression(&children[0])),
                then_branch: derive_block(&children[1]),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::Unsafe => ScalarExpression::Block(derive_block_with_context(
            &direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Block)
                .expect("unsafe block"),
            true,
        )),
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
        SyntaxKind::Unary => {
            let children = semantic_children(&actual);
            let Some(first) = children.first() else {
                panic!("unary expression children")
            };
            let NodeOrToken::Token(operator_token) = first else {
                return derive_element(first);
            };
            let operator = match operator_token.text() {
                "!" => UnaryOperator::LogicalNot,
                "~" => UnaryOperator::BitwiseNot,
                "-" => UnaryOperator::Negate,
                _ => panic!("unary operator grammar"),
            };
            let operand = derive_element(children.get(1).expect("unary operand"));
            ScalarExpression::Unary {
                operator,
                span: ByteSpan::new(
                    operator_token.text_range().start().into(),
                    span_of(&operand).end,
                ),
                operand: Box::new(operand),
            }
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
            let type_arguments = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::GenericTypeArguments)
                .map(|generic| {
                    generic
                        .children()
                        .filter(|child| child.kind() == SyntaxKind::GenericTypeArgument)
                        .map(|argument| ScalarTypeArgument {
                            ty: derive_type(
                                &direct_nodes(&argument)
                                    .into_iter()
                                    .next()
                                    .expect("generic type argument type"),
                            ),
                            span: wosy_syntax::byte_span(&argument),
                        })
                        .collect()
                })
                .map_or_else(Vec::new, |arguments| arguments);
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
                type_arguments,
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
        SyntaxKind::DereferencedField => {
            let name = direct_token(&actual, SyntaxKind::Identifier).expect("field name");
            let receiver = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Parenthesized)
                .map(|child| derive_expression(&child))
                .unwrap_or_else(|| ScalarExpression::Name {
                    name: String::new(),
                    span: wosy_syntax::byte_span(&actual),
                });
            let receiver_name = match &receiver {
                ScalarExpression::Name { name, .. } => name.clone(),
                _ => String::new(),
            };
            ScalarExpression::Member {
                receiver: receiver_name,
                name: name.text().to_owned(),
                receiver_span: span_of(&receiver),
                name_span: token_span(&name),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::RawAddress => {
            let target = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::AssignmentTarget)
                .expect("raw address target");
            ScalarExpression::RawAddress {
                place: derive_place(&target),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::StructLiteral => ScalarExpression::StructLiteral {
            fields: actual
                .children()
                .filter(|child| child.kind() == SyntaxKind::StructLiteralField)
                .map(|field| {
                    let name = direct_token(&field, SyntaxKind::Identifier).expect("field name");
                    let value = field
                        .descendants()
                        .find(|child| child.kind() == SyntaxKind::Expression)
                        .map(|child| derive_expression(&child))
                        .expect("field value");
                    ScalarStructLiteralField {
                        name: name.text().to_owned(),
                        name_span: token_span(&name),
                        value,
                        span: wosy_syntax::byte_span(&field),
                    }
                })
                .collect(),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Parenthesized => derive_expression(&direct_nodes(&actual)[0]),
        SyntaxKind::Expression => derive_element(&semantic_children(&actual)[0]),
        _ => derive_element(&semantic_children(&actual)[0]),
    }
}

fn derive_place(node: &CstNode) -> ScalarPlace {
    match node.kind() {
        SyntaxKind::AssignmentTarget => {
            if let Some(child) = direct_nodes(node).into_iter().next() {
                derive_place(&child)
            } else {
                let name = direct_token(node, SyntaxKind::Identifier).expect("place name");
                ScalarPlace::Name {
                    name: name.text().to_owned(),
                    span: token_span(&name),
                }
            }
        }
        SyntaxKind::Identifier => {
            let token = direct_token(node, SyntaxKind::Identifier).expect("place name");
            ScalarPlace::Name {
                name: token.text().to_owned(),
                span: token_span(&token),
            }
        }
        SyntaxKind::QualifiedMember => {
            let identifiers: Vec<_> = node
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            ScalarPlace::Field {
                base: Box::new(ScalarPlace::Name {
                    name: identifiers[0].text().to_owned(),
                    span: token_span(&identifiers[0]),
                }),
                field: ScalarFieldReference::Unresolved {
                    name: identifiers[1].text().to_owned(),
                    span: token_span(&identifiers[1]),
                },
                span: wosy_syntax::byte_span(node),
            }
        }
        SyntaxKind::DereferencedField => {
            let field = direct_token(node, SyntaxKind::Identifier).expect("field name");
            let parent = direct_nodes(node)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Parenthesized)
                .expect("dereference receiver");
            ScalarPlace::Field {
                base: Box::new(ScalarPlace::Dereference {
                    pointer: Box::new(derive_expression(&direct_nodes(&parent)[0])),
                    span: wosy_syntax::byte_span(&parent),
                }),
                field: ScalarFieldReference::Unresolved {
                    name: field.text().to_owned(),
                    span: token_span(&field),
                },
                span: wosy_syntax::byte_span(node),
            }
        }
        _ => panic!("place grammar"),
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
                    | SyntaxKind::Float
                    | SyntaxKind::Char
                    | SyntaxKind::Boolean
                    | SyntaxKind::String
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
                match parse_integer(token.text()) {
                    Some(value) => ScalarExpression::Integer { value, span },
                    None => ScalarExpression::InvalidInteger {
                        span,
                        error_span: None,
                    },
                }
            }
            SyntaxKind::Float => {
                let span = token_span(token);
                match parse_float(token.text()) {
                    Some(value) => ScalarExpression::Float {
                        value,
                        spelling: token.text().to_owned(),
                        span,
                    },
                    None => ScalarExpression::InvalidFloat { span },
                }
            }
            SyntaxKind::Char => {
                let span = token_span(token);
                match decode_char(token.text()) {
                    Ok(value) => ScalarExpression::Char {
                        value,
                        spelling: token.text().to_owned(),
                        span,
                    },
                    Err((start, length)) => ScalarExpression::InvalidInteger {
                        span,
                        error_span: Some(string_error_span(token, start, length)),
                    },
                }
            }
            SyntaxKind::String => {
                let span = token_span(token);
                match decode_string(token.text()) {
                    Ok(value) => ScalarExpression::Utf8 {
                        value: value.into_bytes(),
                        span,
                    },
                    Err((start, length)) => ScalarExpression::InvalidInteger {
                        span,
                        error_span: Some(string_error_span(token, start, length)),
                    },
                }
            }
            _ => panic!("expression token"),
        },
    }
}

fn parse_integer(text: &str) -> Option<BigInt> {
    let (digits, radix) = if text.starts_with("0x") || text.starts_with("0X") {
        (&text[2..], 16)
    } else if text.starts_with("0b") || text.starts_with("0B") {
        (&text[2..], 2)
    } else if text.starts_with("0o") || text.starts_with("0O") {
        (&text[2..], 8)
    } else {
        (text, 10)
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
        return None;
    }
    let mut previous_underscore = false;
    for character in digits.chars() {
        if character == '_' {
            if previous_underscore {
                return None;
            }
            previous_underscore = true;
        } else {
            let valid = character.to_digit(radix).is_some();
            if !valid {
                return None;
            }
            previous_underscore = false;
        }
    }
    BigInt::parse_bytes(
        digits
            .as_bytes()
            .iter()
            .filter(|byte| **byte != b'_')
            .copied()
            .collect::<Vec<_>>()
            .as_slice(),
        radix,
    )
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
        | ScalarExpression::Float { span, .. }
        | ScalarExpression::InvalidFloat { span }
        | ScalarExpression::InvalidInteger { span, .. }
        | ScalarExpression::Boolean { span, .. }
        | ScalarExpression::Char { span, .. }
        | ScalarExpression::Utf8 { span, .. }
        | ScalarExpression::Binary { span, .. }
        | ScalarExpression::Unary { span, .. }
        | ScalarExpression::Call { span, .. }
        | ScalarExpression::If { span, .. }
        | ScalarExpression::UnitIf { span, .. } => *span,
        ScalarExpression::RawAddress { span, .. }
        | ScalarExpression::StructLiteral { span, .. } => *span,
        ScalarExpression::Block(block) => block.span,
    }
}

fn validate_unit_if_positions(program: &ScalarProgram, diagnostics: &mut Vec<super::Diagnostic>) {
    validate_unit_if_positions_in_items(&program.items, &program.source, diagnostics);
}

fn validate_unit_if_positions_in_module(
    module: &ScalarModule,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    validate_unit_if_positions_in_items(&module.items, &module.source, diagnostics);
}

fn validate_unit_if_positions_in_items(
    items: &[ScalarItem],
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                validate_unit_if_position(&binding.value, false, source, diagnostics)
            }
            ScalarItem::Function(function) => {
                validate_unit_if_positions_in_block(&function.body, source, diagnostics)
            }
            ScalarItem::Executable(item) => {
                validate_unit_if_positions_in_item(item, true, source, diagnostics)
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
        }
    }
}

fn validate_unit_if_positions_in_block(
    block: &ScalarBlock,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (index, item) in block.items.iter().enumerate() {
        let statement = index + 1 != block.items.len() || block.terminated_items[index];
        validate_unit_if_positions_in_item(item, statement, source, diagnostics);
    }
}

fn validate_unit_if_positions_in_item(
    item: &ScalarBlockItem,
    statement: bool,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            validate_unit_if_position(&binding.value, false, source, diagnostics)
        }
        ScalarBlockItem::Expression(expression) => {
            validate_unit_if_position(expression, statement, source, diagnostics)
        }
        ScalarBlockItem::Assignment(assignment) => {
            validate_unit_if_position(&assignment.value, false, source, diagnostics)
        }
        ScalarBlockItem::While(while_expression) => {
            validate_unit_if_position(&while_expression.condition, false, source, diagnostics);
            validate_unit_if_positions_in_block(&while_expression.body, source, diagnostics);
        }
    }
}

fn validate_unit_if_position(
    expression: &ScalarExpression,
    statement: bool,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match expression {
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            span,
        } => {
            if !statement {
                diagnostics.push(super::Diagnostic {
                    code: "B0003".to_owned(),
                    severity: super::DiagnosticSeverity::Error,
                    message: "if without else is only valid as a unit statement".to_owned(),
                    labels: vec![super::DiagnosticLabel {
                        kind: super::DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(source.clone(), *span),
                        message: "if without else is only valid as a unit statement".to_owned(),
                    }],
                    notes: Vec::new(),
                });
            }
            validate_unit_if_position(condition, false, source, diagnostics);
            validate_unit_if_positions_in_block(then_branch, source, diagnostics);
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            validate_unit_if_position(condition, false, source, diagnostics);
            validate_unit_if_positions_in_block(then_branch, source, diagnostics);
            validate_unit_if_positions_in_block(else_branch, source, diagnostics);
        }
        ScalarExpression::Binary { left, right, .. } => {
            validate_unit_if_position(left, false, source, diagnostics);
            validate_unit_if_position(right, false, source, diagnostics);
        }
        ScalarExpression::Unary { operand, .. } => {
            validate_unit_if_position(operand, false, source, diagnostics);
        }
        ScalarExpression::Call { arguments, .. } => {
            for argument in arguments {
                validate_unit_if_position(argument, false, source, diagnostics);
            }
        }
        ScalarExpression::RawAddress { place, .. } => {
            validate_unit_if_position_in_place(place, source, diagnostics)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                validate_unit_if_position(&field.value, false, source, diagnostics);
            }
        }
        ScalarExpression::Block(block) => {
            validate_unit_if_positions_in_block(block, source, diagnostics)
        }
        ScalarExpression::Name { .. }
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

fn validate_unit_if_position_in_place(
    place: &ScalarPlace,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match place {
        ScalarPlace::Dereference { pointer, .. } => {
            validate_unit_if_position(pointer, false, source, diagnostics)
        }
        ScalarPlace::Field { base, .. } => {
            validate_unit_if_position_in_place(base, source, diagnostics)
        }
        ScalarPlace::Name { .. } => {}
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
        "&" => BinaryOperator::BitAnd,
        "|" => BinaryOperator::BitOr,
        "^" => BinaryOperator::BitXor,
        "<<" => BinaryOperator::ShiftLeft,
        ">>" => BinaryOperator::ShiftRight,
        _ => panic!("operator grammar"),
    }
}

fn validate(program: &ScalarProgram) -> Vec<super::Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_unit_if_positions(program, &mut diagnostics);
    let mut declarations = BTreeMap::new();
    let mut declaration_names = BTreeSet::new();
    let mut folded_declarations = BTreeMap::new();
    let mut struct_names = BTreeSet::new();
    for structure in &program.structs {
        if !declare_program_name(
            program,
            &structure.name,
            structure.span,
            &mut struct_names,
            &mut folded_declarations,
            &mut diagnostics,
        ) {
            continue;
        }
        let mut field_names = BTreeSet::new();
        for field in &structure.fields {
            if field_names.insert(field.name.clone()) {
                validate_type(program, &field.ty, field.name_span, &mut diagnostics);
            } else {
                diagnostics.push(diagnostic(
                    program,
                    "B0002",
                    "duplicate declaration",
                    field.name_span,
                ));
            }
        }
    }
    for item in &program.items {
        let (name, span, ty) = match item {
            ScalarItem::Namespace(namespace) => (&namespace.binding, namespace.span, None),
            ScalarItem::Extern(extern_decl) => (&extern_decl.binding, extern_decl.span, None),
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
                if let ScalarItem::Binding(binding) = item {
                    for receiver in &binding.receivers {
                        declarations.insert(receiver.name.clone(), receiver.ty.clone());
                    }
                }
                validate_type(program, ty, span, &mut diagnostics);
            }
        }
    }
    for item in &program.items {
        if let ScalarItem::Extern(extern_decl) = item {
            let module =
                ScalarModule::new(program.source.clone(), program.items.clone(), Vec::new());
            validate_extern(&module, extern_decl, &mut diagnostics);
        }
    }
    let mut scope = declarations.clone();
    for item in &program.items {
        match item {
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
            ScalarItem::Binding(binding) => {
                let actual = expression_type_expected(
                    &binding.value,
                    &binding.declared_type,
                    &declarations,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    program,
                    &mut diagnostics,
                    false,
                );
                validate_output_receivers(
                    &binding.value,
                    &binding.receivers,
                    binding.span,
                    &declarations,
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
                let ScalarType::Callable {
                    outputs,
                    parameters,
                } = &function.signature
                else {
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
                let output = &outputs.outputs[0];
                let actual = block_type_expected(
                    &function.body,
                    &output.ty,
                    &scope,
                    &visible_names,
                    &folded_names,
                    program,
                    &mut diagnostics,
                    false,
                );
                expect_type(
                    program,
                    &output.ty,
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
                    false,
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

fn validate_struct_literal(
    literal: &ScalarStructLiteral,
    expected: &ScalarType,
    declarations: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let ScalarType::Struct(id) = expected else {
        return;
    };
    let Some(structure) = program.structs.iter().find(|item| item.id == *id) else {
        return;
    };
    let mut seen = BTreeSet::new();
    let mut initialized = BTreeSet::new();
    for field in &literal.fields {
        if !seen.insert(field.name.clone()) {
            diagnostics.push(diagnostic(
                program,
                "B0002",
                "duplicate struct literal field",
                field.name_span,
            ));
            continue;
        }
        let Some(declared) = structure.fields.iter().find(|item| item.name == field.name) else {
            diagnostics.push(diagnostic(
                program,
                "M0002",
                "unknown struct field",
                field.name_span,
            ));
            continue;
        };
        initialized.insert(field.name.clone());
        let actual = expression_type_expected(
            &field.value,
            &declared.ty,
            declarations,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            false,
        );
        expect_type(program, &declared.ty, &actual, field.span, diagnostics);
    }
    if initialized.len() != structure.fields.len() {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "struct literal must initialize every field",
            literal.span,
        ));
    }
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
            ScalarExpression::Unary { operand, .. } => self.expression(operand, visible),
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
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition, visible);
                self.block(then_branch, visible);
            }
            ScalarExpression::Block(block) => self.block(block, visible),
            ScalarExpression::RawAddress { place, .. } => self.place(place, visible),
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, visible);
                }
            }
            ScalarExpression::Integer { .. }
            | ScalarExpression::InvalidInteger { .. }
            | ScalarExpression::Float { .. }
            | ScalarExpression::InvalidFloat { .. }
            | ScalarExpression::Boolean { .. }
            | ScalarExpression::Char { .. }
            | ScalarExpression::Utf8 { .. } => {}
        }
    }

    fn place(&mut self, place: &ScalarPlace, visible: &BTreeMap<String, usize>) {
        match place {
            ScalarPlace::Name { name, .. } => self.use_name(name, visible),
            ScalarPlace::Field { base, .. } => self.place(base, visible),
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer, visible),
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
            ScalarItem::Namespace(_)
            | ScalarItem::Extern(_)
            | ScalarItem::Binding(_)
            | ScalarItem::Executable(_) => None,
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
        ScalarType::Named { name, .. }
            if !program.structs.iter().any(|item| item.name == *name) =>
        {
            diagnostics.push(diagnostic(program, "B0003", "unknown scalar type", span))
        }
        ScalarType::Named { .. } | ScalarType::Qualified { .. } => {}
        ScalarType::Struct(_) => {}
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_type(program, &output.ty, output.span, diagnostics);
            }
            for parameter in parameters {
                validate_type(program, parameter, span, diagnostics);
            }
        }
        ScalarType::Unit
        | ScalarType::Bool
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
        | ScalarType::ArtifactId => {}
        ScalarType::RawPointer(inner) if matches!(inner.as_ref(), ScalarType::U8) => {}
        ScalarType::RawPointer(inner) => validate_type(program, inner, span, diagnostics),
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
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = block_item_type(
            item,
            &mut scope,
            &mut visible_names,
            &mut folded_names,
            program,
            diagnostics,
            unsafe_context,
        );
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

fn block_type_expected(
    block: &ScalarBlock,
    expected: &ScalarType,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = match item {
            ScalarBlockItem::Expression(expression)
                if index + 1 == block.items.len() && !block.terminated_items[index] =>
            {
                expression_type_expected(
                    expression,
                    expected,
                    &scope,
                    &visible_names,
                    &folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                )
            }
            _ => block_item_type(
                item,
                &mut scope,
                &mut visible_names,
                &mut folded_names,
                program,
                diagnostics,
                unsafe_context,
            ),
        };
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
    unsafe_context: bool,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let actual = expression_type_expected(
                &binding.value,
                &binding.declared_type,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            validate_output_receivers(
                &binding.value,
                &binding.receivers,
                binding.span,
                scope,
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
                for receiver in &binding.receivers {
                    scope.insert(receiver.name.clone(), receiver.ty.clone());
                }
            }
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => {
            reject_multi_output_statement(expression, scope, program, diagnostics);
            expression_type(
                expression,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            )
        }
        ScalarBlockItem::Assignment(assignment) => assignment_type(
            assignment,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        ScalarBlockItem::While(while_expression) => while_type(
            while_expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
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
    unsafe_context: bool,
) -> ScalarType {
    let field_expected = assignment.receiver.as_ref().and_then(|receiver| {
        let receiver_type = scope.get(receiver);
        let receiver_type = receiver_type?;
        let structure = match receiver_type {
            ScalarType::Struct(id) => program.structs.get(id.index),
            ScalarType::RawPointer(inner) => match inner.as_ref() {
                ScalarType::Struct(id) => program.structs.get(id.index),
                _ => None,
            },
            _ => None,
        }?;
        structure
            .fields
            .iter()
            .find(|field| field.name == assignment.target)
            .map(|field| field.ty.clone())
    });
    let Some(expected) = field_expected.or_else(|| scope.get(&assignment.target).cloned()) else {
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
    let actual = expression_type_expected(
        &assignment.value,
        &expected,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    validate_assignment_outputs(assignment, scope, program, diagnostics);
    expect_type(program, &expected, &actual, assignment.span, diagnostics);
    if is_error_type(&actual) {
        ScalarType::Error
    } else {
        expected
    }
}

fn while_type(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let condition = expression_type(
        &while_expression.condition,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    if !is_error_type(&condition) && !scalar_type_equal(&condition, &ScalarType::Bool) {
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
        unsafe_context,
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
    unsafe_context: bool,
) -> ScalarType {
    match expression {
        ScalarExpression::RawAddress { place, span } => {
            if !unsafe_context {
                diagnostics.push(diagnostic(
                    program,
                    "B0012",
                    "raw address requires an unsafe block",
                    *span,
                ));
            }
            ScalarType::RawPointer(Box::new(place_type(place, scope, program, diagnostics)))
        }
        ScalarExpression::StructLiteral { .. } => ScalarType::Error,
        ScalarExpression::Name { name, span } if name == "null" => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "null requires a pointer context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarExpression::Member {
            receiver,
            name,
            name_span,
            span,
            ..
        } => field_type(
            scope.get(receiver),
            name,
            *name_span,
            *span,
            program,
            diagnostics,
        ),
        ScalarExpression::Integer { value, span } => {
            validate_integer_range_program(program, value, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::Float { .. } => ScalarType::Error,
        ScalarExpression::InvalidFloat { .. } => ScalarType::Error,
        ScalarExpression::InvalidInteger {
            span: _,
            error_span,
        } => {
            if error_span.is_some() {
                ScalarType::Error
            } else {
                ScalarType::I32
            }
        }
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Char { .. } => ScalarType::Char,
        ScalarExpression::Utf8 { span, .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "string literal requires a std.utf8 context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Unary {
            operator,
            operand,
            span,
        } => {
            if matches!(operator, UnaryOperator::Negate) {
                if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                    let value = -value;
                    validate_integer_range_program(program, &value, *span, diagnostics);
                    return ScalarType::I32;
                }
            }
            let operand_type = expression_type(
                operand,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            match operator {
                UnaryOperator::LogicalNot => {
                    expect_type(
                        program,
                        &ScalarType::Bool,
                        &operand_type,
                        *span,
                        diagnostics,
                    );
                    ScalarType::Bool
                }
                UnaryOperator::BitwiseNot => {
                    expect_integer(program, &operand_type, *span, diagnostics);
                    operand_type
                }
                UnaryOperator::Negate => {
                    expect_integer(program, &operand_type, *span, diagnostics);
                    operand_type
                }
            }
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            if comparison && (is_null_expression(left) || is_null_expression(right)) {
                return expression_type_expected(
                    expression,
                    &ScalarType::Bool,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            let left_type = expression_type(
                left,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let right_type = expression_type(
                right,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
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
            let bitwise = matches!(
                operator,
                BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
            );
            if boolean {
                expect_type(program, &ScalarType::Bool, &left_type, *span, diagnostics);
                expect_type(program, &ScalarType::Bool, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if comparison {
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if bitwise {
                expect_integer(program, &left_type, *span, diagnostics);
                expect_integer(program, &right_type, *span, diagnostics);
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                left_type
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
            type_arguments,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "this_artifact_id" {
                if !arguments.is_empty() {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                return ScalarType::ArtifactId;
            }
            if receiver.as_deref() == Some("core") && name == "declare_artifact" {
                if arguments.len() != 1 {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                let actual = expression_type_expected(
                    &arguments[0],
                    &ScalarType::Error,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(program, &ScalarType::Error, &actual, *span, diagnostics);
                return if is_error_type(&actual) {
                    ScalarType::Error
                } else {
                    ScalarType::ArtifactId
                };
            }
            if receiver.as_deref() == Some("core") && name == "cast" {
                return type_core_cast(
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            let lookup_name = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            let callable = scope.get(&lookup_name).cloned().or_else(|| {
                program.items.iter().find_map(|item| match item {
                    ScalarItem::Extern(extern_decl)
                        if receiver.as_deref() == Some(extern_decl.binding.as_str()) =>
                    {
                        extern_decl
                            .functions
                            .iter()
                            .find(|function| function.name == *name)
                            .map(|function| function.signature.clone())
                    }
                    _ => None,
                })
            });
            let unsafe_callable = program.items.iter().any(|item| {
                matches!(
                    item,
                    ScalarItem::Extern(extern_decl)
                        if receiver.as_deref() == Some(extern_decl.binding.as_str())
                            && extern_decl.functions.iter().any(|function| {
                                function.name == *name && function.unsafe_marker
                            })
                )
            });
            let Some(ScalarType::Callable {
                outputs,
                parameters,
            }) = callable
            else {
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
                let actual = expression_type_expected(
                    argument,
                    &parameter,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(program, &parameter, &actual, *span, diagnostics);
                error_argument |= is_error_type(&actual);
            }
            if unsafe_callable && !unsafe_context {
                diagnostics.push(diagnostic(
                    program,
                    "B0012",
                    "unsafe extern call requires an unsafe block",
                    *span,
                ));
            }
            if error_argument {
                ScalarType::Error
            } else {
                scalar_call_result(&outputs)
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
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !scalar_type_equal(&condition_type, &ScalarType::Bool)
            {
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
                unsafe_context,
            );
            let else_type = block_type(
                else_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && !scalar_type_equal(&then_type, &else_type)
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
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            let condition_type = expression_type(
                condition,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            expect_type(
                program,
                &ScalarType::Bool,
                &condition_type,
                expression_span(condition),
                diagnostics,
            );
            block_type(
                then_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            ScalarType::Unit
        }
        ScalarExpression::Block(block) => block_type(
            block,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
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
    if !is_error_type(expected) && !is_error_type(actual) && !scalar_type_equal(expected, actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
}

fn expect_integer(
    program: &ScalarProgram,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(actual) && !is_integer_type(actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "integer operation requires integer operands",
            span,
        ));
    }
}

fn expression_type_expected(
    expression: &ScalarExpression,
    expected: &ScalarType,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if let ScalarExpression::Unary {
        operator,
        operand,
        span,
    } = expression
    {
        if matches!(operator, UnaryOperator::Negate) {
            if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                let value = -value;
                if is_integer_type(expected) {
                    validate_integer_range_for_type_program(
                        program,
                        &value,
                        *span,
                        expected,
                        diagnostics,
                    );
                    return expected.clone();
                }
            }
        }
        let operand_type = expression_type_expected(
            operand,
            if matches!(operator, UnaryOperator::LogicalNot) {
                &ScalarType::Bool
            } else {
                expected
            },
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        );
        match operator {
            UnaryOperator::LogicalNot => {
                expect_type(
                    program,
                    &ScalarType::Bool,
                    &operand_type,
                    *span,
                    diagnostics,
                );
                ScalarType::Bool
            }
            UnaryOperator::BitwiseNot => {
                expect_integer(program, &operand_type, *span, diagnostics);
                expect_type(program, expected, &operand_type, *span, diagnostics);
                operand_type
            }
            UnaryOperator::Negate => {
                expect_integer(program, &operand_type, *span, diagnostics);
                expect_type(program, expected, &operand_type, *span, diagnostics);
                operand_type
            }
        }
    } else {
        if let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } = expression
        {
            let condition_type = expression_type(
                condition,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !scalar_type_equal(&condition_type, &ScalarType::Bool)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0005",
                    "conditional expression requires bool",
                    *span,
                ));
            }
            let then_type = block_type_expected(
                then_branch,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let else_type = block_type_expected(
                else_branch,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && !scalar_type_equal(&then_type, &else_type)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            return then_type;
        }
        if matches!(expression, ScalarExpression::Name { name, .. } if name == "null")
            && matches!(expected, ScalarType::RawPointer(_))
        {
            return expected.clone();
        }
        if matches!(expression, ScalarExpression::Integer { .. })
            && matches!(
                expected,
                ScalarType::I8
                    | ScalarType::I16
                    | ScalarType::I32
                    | ScalarType::I64
                    | ScalarType::I128
                    | ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
            )
        {
            if let ScalarExpression::Integer { value, span } = expression {
                validate_integer_range_for_type_program(
                    program,
                    value,
                    *span,
                    expected,
                    diagnostics,
                );
            }
            return expected.clone();
        }
        if matches!(expression, ScalarExpression::Float { .. })
            && matches!(expected, ScalarType::F32 | ScalarType::F64)
        {
            if let ScalarExpression::Float { value, span, .. } = expression {
                if *expected == ScalarType::F32 && !(*value as f32).is_finite() {
                    diagnostics.push(diagnostic(
                        program,
                        "B0010",
                        "floating-point literal is outside the f32 range",
                        *span,
                    ));
                    return ScalarType::Error;
                }
            }
            return expected.clone();
        }
        if let ScalarExpression::Block(block) = expression {
            return block_type_expected(
                block,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
        }
        if let ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } = expression
        {
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
            let arithmetic_context = matches!(
                expected,
                ScalarType::I8
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
            );
            let bitwise = matches!(
                operator,
                BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
            );
            let bool_context = ScalarType::Bool;
            let (left_type, right_type) = if comparison && is_null_expression(left) {
                let right_type = expression_type(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                let left_type = expression_type_expected(
                    left,
                    &right_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            } else if comparison && matches!(left.as_ref(), ScalarExpression::Float { .. }) {
                let right_type = expression_type(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                let left_type = expression_type_expected(
                    left,
                    &right_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            } else if comparison && matches!(right.as_ref(), ScalarExpression::Float { .. }) {
                let left_type = expression_type(
                    left,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                let right_type = expression_type_expected(
                    right,
                    &left_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            } else {
                let left_type = if boolean {
                    expression_type_expected(
                        left,
                        &bool_context,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                } else if comparison {
                    expression_type(
                        left,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                } else {
                    expression_type_expected(
                        left,
                        expected,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                };
                let right_type = expression_type_expected(
                    right,
                    if boolean {
                        &bool_context
                    } else if comparison && is_null_expression(right) {
                        &left_type
                    } else {
                        expected
                    },
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            };
            if boolean {
                expect_type(program, &bool_context, &left_type, *span, diagnostics);
                expect_type(program, &bool_context, &right_type, *span, diagnostics);
                return ScalarType::Bool;
            }
            if is_error_type(&left_type) || is_error_type(&right_type) {
                return ScalarType::Error;
            }
            if comparison {
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                return ScalarType::Bool;
            }
            if bitwise {
                expect_integer(program, &left_type, *span, diagnostics);
                expect_integer(program, &right_type, *span, diagnostics);
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                return left_type;
            }
            expect_type(
                program,
                if arithmetic_context {
                    expected
                } else {
                    &ScalarType::I32
                },
                &left_type,
                *span,
                diagnostics,
            );
            expect_type(program, &left_type, &right_type, *span, diagnostics);
            if arithmetic_context {
                return expected.clone();
            }
            return ScalarType::I32;
        }
        if let ScalarExpression::StructLiteral { fields, span } = expression {
            let ScalarType::Struct(id) = expected else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "struct literal requires a struct context",
                    *span,
                ));
                return ScalarType::Error;
            };
            let Some(structure) = program
                .structs
                .get(id.index)
                .filter(|structure| structure.id == *id)
            else {
                diagnostics.push(diagnostic(program, "B0003", "invalid struct type", *span));
                return ScalarType::Error;
            };
            let mut seen = BTreeSet::new();
            let mut initialized = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.name.clone()) {
                    diagnostics.push(diagnostic(
                        program,
                        "B0002",
                        "duplicate struct literal field",
                        field.name_span,
                    ));
                    continue;
                }
                let Some(declared) = structure.fields.iter().find(|item| item.name == field.name)
                else {
                    diagnostics.push(diagnostic(
                        program,
                        "M0002",
                        "unknown struct field",
                        field.name_span,
                    ));
                    continue;
                };
                initialized.insert(field.name.clone());
                let actual = expression_type_expected(
                    &field.value,
                    &declared.ty,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(program, &declared.ty, &actual, field.span, diagnostics);
            }
            if initialized.len() != structure.fields.len() {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "struct literal must initialize every field",
                    *span,
                ));
            }
            return expected.clone();
        }
        expression_type(
            expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        )
    }
}

fn is_null_expression(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Name { name, .. } => name == "null",
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => is_null_block(then_branch) && is_null_block(else_branch),
        ScalarExpression::Block(block) => is_null_block(block),
        _ => false,
    }
}

fn is_null_block(block: &ScalarBlock) -> bool {
    match (block.items.last(), block.terminated_items.last()) {
        (Some(ScalarBlockItem::Expression(expression)), Some(false)) => {
            is_null_expression(expression)
        }
        _ => false,
    }
}

fn field_type(
    receiver: Option<&ScalarType>,
    name: &str,
    name_span: ByteSpan,
    span: ByteSpan,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let Some(receiver) = receiver else {
        diagnostics.push(diagnostic(program, "B0001", "unknown name", span));
        return ScalarType::Error;
    };
    let structure = match receiver {
        ScalarType::Struct(id) => program.structs.get(id.index),
        ScalarType::RawPointer(inner) => match inner.as_ref() {
            ScalarType::Struct(id) => program.structs.get(id.index),
            _ => None,
        },
        _ => None,
    };
    let Some(structure) = structure else {
        diagnostics.push(diagnostic(program, "B0001", "unknown field", name_span));
        return ScalarType::Error;
    };
    let Some(field) = structure.fields.iter().find(|field| field.name == name) else {
        diagnostics.push(diagnostic(program, "B0001", "unknown field", name_span));
        return ScalarType::Error;
    };
    field.ty.clone()
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

fn validate_integer_range_for_type(
    module: &ScalarModule,
    value: &BigInt,
    span: ByteSpan,
    ty: &ScalarType,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (minimum, maximum) = match ty {
        ScalarType::I8 => (BigInt::from(i8::MIN), BigInt::from(i8::MAX)),
        ScalarType::I16 => (BigInt::from(i16::MIN), BigInt::from(i16::MAX)),
        ScalarType::I32 => (BigInt::from(i32::MIN), BigInt::from(i32::MAX)),
        ScalarType::I64 => (BigInt::from(i64::MIN), BigInt::from(i64::MAX)),
        ScalarType::I128 => (BigInt::from(i128::MIN), BigInt::from(i128::MAX)),
        ScalarType::U8 => (BigInt::from(0), BigInt::from(u8::MAX)),
        ScalarType::U16 => (BigInt::from(0), BigInt::from(u16::MAX)),
        ScalarType::U32 => (BigInt::from(0), BigInt::from(u32::MAX)),
        ScalarType::U64 => (BigInt::from(0), BigInt::from(u64::MAX)),
        ScalarType::U128 => (BigInt::from(0), BigInt::from(u128::MAX)),
        _ => return,
    };
    if value < &minimum || value > &maximum {
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

fn validate_integer_range_for_type_program(
    program: &ScalarProgram,
    value: &BigInt,
    span: ByteSpan,
    ty: &ScalarType,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (minimum, maximum) = match ty {
        ScalarType::I8 => (BigInt::from(i8::MIN), BigInt::from(i8::MAX)),
        ScalarType::I16 => (BigInt::from(i16::MIN), BigInt::from(i16::MAX)),
        ScalarType::I32 => (BigInt::from(i32::MIN), BigInt::from(i32::MAX)),
        ScalarType::I64 => (BigInt::from(i64::MIN), BigInt::from(i64::MAX)),
        ScalarType::I128 => (BigInt::from(i128::MIN), BigInt::from(i128::MAX)),
        ScalarType::U8 => (BigInt::from(0), BigInt::from(u8::MAX)),
        ScalarType::U16 => (BigInt::from(0), BigInt::from(u16::MAX)),
        ScalarType::U32 => (BigInt::from(0), BigInt::from(u32::MAX)),
        ScalarType::U64 => (BigInt::from(0), BigInt::from(u64::MAX)),
        ScalarType::U128 => (BigInt::from(0), BigInt::from(u128::MAX)),
        _ => return,
    };
    if value < &minimum || value > &maximum {
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
    fn derives_unit_if_statement_and_requires_bool_condition_span() {
        let text =
            "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (true) { touch(); }; };\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Function(function) = &result.program.items[1] else {
            panic!("unit conditional function")
        };
        assert!(matches!(
            function.body.items[0],
            ScalarBlockItem::Expression(ScalarExpression::UnitIf { .. })
        ));

        let invalid = validate_text(
            "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (1) { touch(); }; };\n%%end",
        );
        let condition = invalid
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "expression type does not match expected type")
            .expect("bool condition diagnostic");
        let start = "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (".len() as u32;
        assert_eq!(
            condition.labels[0].span.range,
            ByteSpan::new(start, start + 1)
        );
    }

    #[test]
    fn rejects_unit_if_in_value_contexts_at_the_conditional_span() {
        let cases = [
            "%%start\nunit() touch = fn { 1; };\nunit value = if (true) { touch(); };\n%%end",
            "%%start\nunit() touch = fn { 1; };\nunit() value = fn { if (true) { touch(); } };\n%%end",
            "%%start\nunit() touch = fn { 1; };\nunit(unit) accept = fn(value) { touch(); };\naccept(if (true) { touch(); });\n%%end",
            "%%start\nunit() touch = fn { 1; };\nunit target = touch();\ntarget = if (true) { touch(); };\n%%end",
            "%%start\nunit() touch = fn { 1; };\ni32 value = (if (true) { touch(); }) + 1;\n%%end",
        ];
        for text in cases {
            let result = validate_text(text);
            let start = text.find("if (true)").expect("conditional") as u32;
            let end = start + "if (true) { touch(); }".len() as u32;
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "if without else is only valid as a unit statement"
                    && diagnostic.labels[0].span.range == ByteSpan::new(start, end)
            }));
        }
    }

    #[test]
    fn derives_and_validates_wasi_fd_write_with_typed_utf8() {
        let result = validate_text(
            "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\ni32 out = wasi.fd_write(1, 1);\ni32 err = wasi.fd_write(2, 2);\n%%end",
        );
        assert!(
            result.diagnostics.is_empty(),
            "{:?}\n{:?}",
            result.diagnostics,
            result.program
        );
        let ScalarItem::Extern(extern_decl) = &result.program.items[0] else {
            panic!("extern item");
        };
        assert_eq!(
            extern_decl.actual_module,
            ScalarExternModule::Valid("wasi_snapshot_preview1".to_owned())
        );
        assert_eq!(extern_decl.functions[0].name, "fd_write");
        assert!(!extern_decl.functions[0].unsafe_marker);
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("binding item");
        };
        assert!(matches!(binding.value, ScalarExpression::Call { .. }));
        assert!(matches!(binding.declared_type, ScalarType::I32));
    }

    #[test]
    fn accepts_generic_extern_calls_without_wasi_adapter_validation() {
        let invalid = validate_text(
            "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\ni32 out = wasi.fd_write(0, 1);\n%%end",
        );
        assert!(invalid.diagnostics.is_empty(), "{:?}", invalid.diagnostics);

        let dynamic = validate_text(
            "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\ni32 text = 1;\ni32 out = wasi.fd_write(1, text);\n%%end",
        );
        assert!(dynamic.diagnostics.is_empty(), "{:?}", dynamic.diagnostics);

        let missing = validate_text("%%start\ni32 out = wasi.fd_write(1, \"x\");\n%%end");
        assert!(missing
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0001"));

        let multiple = validate_text(
            "%%start\nenv = extern wasm \"helper\" { (u64, bool)() read; };\nu64 first, bool second = env.read<u32>();\n%%end",
        );
        assert!(
            multiple.diagnostics.is_empty(),
            "{:?}",
            multiple.diagnostics
        );
        let ScalarItem::Binding(binding) = &multiple.program.items[1] else {
            panic!("generic extern binding")
        };
        assert_eq!(binding.receivers.len(), 2);
        assert_eq!(binding.output_sequence.outputs.len(), 2);
        assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::U64);
        assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::Bool);
        let call_span = expression_span(&binding.value);
        assert_eq!(binding.output_sequence.outputs[0].span, call_span);
        assert_eq!(binding.output_sequence.outputs[1].span, call_span);
        assert_eq!(binding.output_values[0].position, 0);
        assert_eq!(binding.output_values[1].position, 1);
        assert_eq!(binding.output_values[0].ty, ScalarType::U64);
        assert_eq!(binding.output_values[1].ty, ScalarType::Bool);
        assert_eq!(binding.output_values[0].span, call_span);
        assert_eq!(binding.output_values[1].span, call_span);
        let ScalarItem::Extern(extern_decl) = &multiple.program.items[0] else {
            panic!("generic extern item")
        };
        let ScalarType::Callable { outputs, .. } = &extern_decl.functions[0].signature else {
            panic!("generic extern signature")
        };
        assert_eq!(outputs.outputs.len(), 2);
    }

    #[test]
    fn derives_all_generic_extern_functions_in_declaration_order() {
        let result = validate_text(
            "%%start\nenv = extern wasm \"helper\" { unsafe i32(i32) read; unit() flush; };\ni32 value = unsafe { env.read(7) };\nenv.flush();\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Extern(extern_decl) = &result.program.items[0] else {
            panic!("extern item");
        };
        assert_eq!(extern_decl.functions.len(), 2);
        assert_eq!(extern_decl.functions[0].name, "read");
        assert!(extern_decl.functions[0].unsafe_marker);
        assert_eq!(extern_decl.functions[1].name, "flush");
        assert!(!extern_decl.functions[1].unsafe_marker);
    }

    #[test]
    fn ordinary_unsafe_extern_requires_structured_unsafe_block() {
        let safe_text = "%%start\nraw = extern wasm \"env\" { unsafe i32(i32) read; };\ni32 value = raw.read(1);\n%%end";
        let safe = validate_text(safe_text);
        let diagnostic = safe
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0012")
            .expect("unsafe-call diagnostic");
        assert_eq!(
            diagnostic.message,
            "unsafe extern call requires an unsafe block"
        );
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(71, 82));

        let unsafe_text = "%%start\nraw = extern wasm \"env\" { unsafe i32(i32) read; };\ni32 value = unsafe { raw.read(1) };\n%%end";
        let accepted = validate_text(unsafe_text);
        assert!(
            accepted.diagnostics.is_empty(),
            "{:?}",
            accepted.diagnostics
        );
        let ScalarItem::Binding(binding) = &accepted.program.items[1] else {
            panic!("binding item");
        };
        let ScalarExpression::Block(block) = &binding.value else {
            panic!("unsafe block expression");
        };
        assert!(block.unsafe_context);
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
    fn validates_unary_nesting_bitwise_types_and_short_circuit_booleans() {
        let result = validate_text(
            "%%start
i32 complemented = ~~~1;
bool inverted = !!!true;
i32 bit_and = 7 & 3;
i32 bit_or = 7 | 3;
i32 bit_xor = 7 ^ 3;
i32 shifted_left = 7 << 1;
i32 shifted_right = 7 >> 1;
bool conjunction = true && false;
bool disjunction = false || true;
%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("complement binding");
        };
        assert!(matches!(
            &binding.value,
            ScalarExpression::Unary {
                operator: UnaryOperator::BitwiseNot,
                operand,
                ..
            } if matches!(operand.as_ref(), ScalarExpression::Unary { .. })
        ));
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("inversion binding");
        };
        assert!(matches!(
            &binding.value,
            ScalarExpression::Unary {
                operator: UnaryOperator::LogicalNot,
                operand,
                ..
            } if matches!(operand.as_ref(), ScalarExpression::Unary { .. })
        ));
    }

    #[test]
    fn derives_structural_negation_and_contextually_validates_integer_ranges() {
        let text = "%%start\ni32 minimum = -2147483648;\ni32 quotient = -7 / 3;\ni32 remainder = -7 % 3;\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("minimum binding");
        };
        assert!(matches!(
            &binding.value,
            ScalarExpression::Unary {
                operator: UnaryOperator::Negate,
                span,
                operand,
            } if *span == ByteSpan::new(22, 33)
                && matches!(operand.as_ref(), ScalarExpression::Integer { value, .. } if value == &BigInt::from(2147483648u32))
        ));

        for name in ["quotient", "remainder"] {
            let ScalarItem::Binding(binding) = result
                .program
                .items
                .iter()
                .find(|item| matches!(item, ScalarItem::Binding(binding) if binding.name == name))
                .expect("arithmetic binding")
            else {
                panic!("arithmetic binding");
            };
            assert!(matches!(binding.declared_type, ScalarType::I32));
            assert!(matches!(binding.value, ScalarExpression::Binary { .. }));
        }

        let invalid = validate_text("%%start\ni32 value = -2147483649;\n%%end");
        let diagnostic = invalid
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0010")
            .expect("negative range diagnostic");
        assert_eq!(
            diagnostic.message,
            "integer literal is outside the resolved target type range"
        );
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(20, 31));

        let unsigned = validate_text("%%start\nu32 value = -1;\n%%end");
        assert!(unsigned.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0010"
                && diagnostic.message == "integer literal is outside the resolved target type range"
                && diagnostic.labels[0].span.range == ByteSpan::new(20, 22)
        }));
    }

    #[test]
    fn rejects_invalid_integer_boolean_and_equal_type_operands_at_expression_spans() {
        let text = "%%start
i32 signed = 1;
u32 unsigned = 1;
i32 mismatch = signed & unsigned;
i32 bool_bitwise = true | false;
i32 bool_complement = ~true;
bool integer_inversion = !1;
%%end";
        let result = validate_text(text);
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .collect();
        assert!(diagnostics.len() >= 4, "{:?}", result.diagnostics);
        for expression in ["signed & unsigned", "true | false", "~true", "!1"] {
            let start = text.find(expression).expect("expression") as u32;
            let end = start + expression.len() as u32;
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.labels[0].span.range == ByteSpan::new(start, end)),
                "missing diagnostic span for {expression}: {:?}",
                diagnostics
            );
        }
    }

    #[test]
    fn preserves_scalar_precedence_and_left_associativity() {
        let result = validate_text(
            "%%start\ni32 value = 1 | 2 ^ 3 & 4 << 1 + 2 * 3;\ni32 shifts = 8 >> 1 >> 1;\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("value binding");
        };
        let ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } = &binding.value
        else {
            panic!("outer binary expression");
        };
        assert_eq!(*operator, BinaryOperator::BitOr);
        assert!(
            matches!(
                right.as_ref(),
                ScalarExpression::Binary {
                    operator: BinaryOperator::BitXor,
                    ..
                }
            ),
            "{binding:?}"
        );
        assert!(matches!(left.as_ref(), ScalarExpression::Integer { .. }));
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("shift binding");
        };
        assert!(matches!(
            &binding.value,
            ScalarExpression::Binary {
                operator: BinaryOperator::ShiftRight,
                left,
                ..
            } if matches!(left.as_ref(), ScalarExpression::Binary {
                operator: BinaryOperator::ShiftRight,
                ..
            })
        ));
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
                &mut Vec::new(),
                false,
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
        let result = validate_text("%%start\ni32 value = 1;\nu32 other = value;\n%%end");
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
    fn derives_generic_call_type_arguments_with_unresolved_type_and_span() {
        let result =
            validate_text("%%start\nu32 result = core.cast<u32>(value, \"exact\");\n%%end");
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("result binding");
        };
        let ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            ..
        } = &binding.value
        else {
            panic!("generic call");
        };
        assert_eq!(receiver.as_deref(), Some("core"));
        assert_eq!(name, "cast");
        assert_eq!(arguments.len(), 2);
        assert_eq!(type_arguments.len(), 1);
        assert_eq!(type_arguments[0].ty, ScalarType::U32);
        assert_eq!(type_arguments[0].span, ByteSpan::new(31, 34));
        let serialized = serde_json::to_string(type_arguments).expect("serialize type arguments");
        let restored: Vec<ScalarTypeArgument> =
            serde_json::from_str(&serialized).expect("deserialize type arguments");
        assert_eq!(restored, *type_arguments);
    }

    #[test]
    fn validates_generic_exact_u64_to_u32_cast() {
        let result = validate_text(
            "%%start\nu64 source = 42;\nu32 result = core.cast<u32>(source, \"exact\");\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("cast binding");
        };
        assert_eq!(binding.declared_type, ScalarType::U32);
    }

    #[test]
    fn validates_generic_ordinary_two_output_call_in_order() {
        let result = validate_text(
            "%%start\n(u64, bool)() pair = fn { 1 };\nu64 first, bool second = pair<u32>();\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("generic binding");
        };
        assert_eq!(binding.output_sequence.outputs.len(), 2);
        assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::U64);
        assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::Bool);
        assert_eq!(binding.output_values[0].position, 0);
        assert_eq!(binding.output_values[1].position, 1);
        let call_span = expression_span(&binding.value);
        assert_eq!(binding.output_values[0].span, call_span);
        assert_eq!(binding.output_values[1].span, call_span);
    }

    #[test]
    fn reports_exact_cast_range_at_value_span() {
        let text = "%%start\nu32 result = core.cast<u32>(4294967296, \"exact\");\n%%end";
        let result = validate_text(text);
        let value_start = text.find("4294967296").expect("cast value") as u32;
        let value_end = value_start + 10;
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0010")
            .expect("exact cast range diagnostic");
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(value_start, value_end)
        );
    }

    #[test]
    fn rejects_non_static_exact_cast_modes_and_shapes() {
        let dynamic = validate_text(
            "%%start\nu64 source = 1;\nutf8 mode = \"exact\";\nu32 result = core.cast<u32>(source, mode);\n%%end",
        );
        assert!(dynamic
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
        let wrong_source = validate_text(
            "%%start\ni32 source = 1;\nu32 result = core.cast<u32>(source, \"exact\");\n%%end",
        );
        assert!(wrong_source
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
        let wrong_destination = validate_text(
            "%%start\nu64 source = 1;\ni32 result = core.cast<i32>(source, \"exact\");\n%%end",
        );
        assert!(wrong_destination
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
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
    fn decodes_all_scalar_string_escapes_and_unicode_values() {
        let result = validate_text(
            r##"%%start
std.utf8 value = "\\\"\'\n\r\t\0\u{0}\u{41}\u{1F600}";
%%end"##,
        );
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("binding item");
        };
        let ScalarExpression::Utf8 { value, .. } = &binding.value else {
            panic!("utf8 expression");
        };
        assert_eq!(
            value,
            &vec![b'\\', b'"', b'\'', b'\n', b'\r', b'\t', 0, 0, b'A', 0xF0, 0x9F, 0x98, 0x80]
        );
    }

    #[test]
    fn derives_char_and_artifact_id_with_exact_contextual_types() {
        let result = validate_text(
            r##"%%start
char(char) echo = fn(value) { value };
char initial = '\u{1F600}';
char copied = echo(initial);
artifact_id current = core.this_artifact_id();
bool same = current == current;
%%end"##,
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(initial) = &result.program.items[1] else {
            panic!("char binding");
        };
        assert_eq!(initial.declared_type, ScalarType::Char);
        assert!(matches!(
            initial.value,
            ScalarExpression::Char { value: '😀', .. }
        ));
        let ScalarItem::Binding(current) = &result.program.items[3] else {
            panic!("artifact binding");
        };
        assert_eq!(current.declared_type, ScalarType::ArtifactId);
    }

    #[test]
    fn rejects_invalid_character_shapes_at_literal_spans() {
        for literal in ["''", "'ab'", "'\\q'", "'\\u{D800}'", "'\\u{110000}'"] {
            let text = format!("%%start\nchar value = {literal};\n%%end");
            let literal_start = text.find(literal).expect("literal") as u32;
            let literal_end = literal_start + literal.len() as u32;
            let result = validate_text(&text);
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message == "invalid character literal")
                .expect("character diagnostic");
            assert!(diagnostic.labels[0].span.range.start >= literal_start);
            assert!(diagnostic.labels[0].span.range.end <= literal_end);
        }
    }

    #[test]
    fn reports_malformed_string_escapes_at_their_exact_spans() {
        let cases = [
            (r##"\q"##, 1, 3),
            (r##"\u{}"##, 1, 5),
            (r##"\u{12"##, 1, 6),
            (r##"\u{D800}"##, 1, 9),
            (r##"\u{110000}"##, 1, 11),
        ];
        for (literal, relative_start, relative_end) in cases {
            let text = format!("%%start\nstd.utf8 value = \"{}\";\n%%end", literal);
            let parsed = parse_source(source(), text.clone(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let result = derive_scalar_program(&parsed.result);
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message == "invalid string literal")
                .expect("string diagnostic");
            let ScalarItem::Binding(binding) = &result.program.items[0] else {
                panic!("binding item");
            };
            let (literal_start, error_span) = match &binding.value {
                ScalarExpression::InvalidInteger {
                    span,
                    error_span: Some(error_span),
                } => (span.start, *error_span),
                _ => panic!("invalid string expression"),
            };
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(literal_start + relative_start, literal_start + relative_end)
            );
            assert_eq!(error_span, diagnostic.labels[0].span.range);
        }
    }

    #[test]
    fn reports_malformed_extern_module_string_without_panicking() {
        let text = r##"%%start
wasi = extern wasm "\q" { i32(i32, i32) fd_write; };
%%end"##;
        let parsed = parse_source(source(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let result = derive_scalar_program(&parsed.result);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "invalid string literal")
            .expect("extern string diagnostic");
        let ScalarItem::Extern(extern_decl) = &result.program.items[0] else {
            panic!("extern item");
        };
        let start = extern_decl.module_span.start;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start + 1, start + 3)
        );
        assert_eq!(
            extern_decl.actual_module,
            ScalarExternModule::Invalid {
                span: ByteSpan::new(start, start + 4),
                error_span: ByteSpan::new(start + 1, start + 3),
            }
        );
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
    fn validates_two_output_namespace_call_and_receiver_order() {
        let math_source = module_source("src/math.w");
        let math = module_from_text(
            math_source.clone(),
            "%%start\n(u64, bool)() pair = fn { 1 };\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\nu64 first, bool second = math.pair();\n%%end",
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
            vec![main_source.clone(), math_source],
        );
        let result = validate_scalar_project(project);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.project.modules[0].items[1] else {
            panic!("namespace binding")
        };
        assert_eq!(binding.output_sequence.outputs.len(), 2);
        let call_span = expression_span(&binding.value);
        assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::U64);
        assert_eq!(binding.output_sequence.outputs[0].span, call_span);
        assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::Bool);
        assert_eq!(binding.output_sequence.outputs[1].span, call_span);
        assert_eq!(binding.output_values[0].position, 0);
        assert_eq!(binding.output_values[1].position, 1);
        assert_eq!(binding.output_values[0].ty, ScalarType::U64);
        assert_eq!(binding.output_values[1].ty, ScalarType::Bool);
        assert_eq!(expression_span(&binding.output_values[0].value), call_span);
        assert_eq!(expression_span(&binding.output_values[1].value), call_span);
        assert_eq!(binding.receivers[0].ty, ScalarType::U64);
        assert_eq!(binding.receivers[1].ty, ScalarType::Bool);
    }

    #[test]
    fn resolves_direct_root_extern_with_local_namespace_present() {
        let local_source = module_source("src/local.w");
        let local = module_from_text(local_source.clone(), "%%start\ni32 value = 7;\n%%end");
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\nlocal = namespace app \"src/local.w\";\ni32 value = local.value;\ni32 out = wasi.fd_write(1, 1);\n%%end",
        );
        let namespace_span = match &main.items[1] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let project = ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source.clone(),
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "local".to_owned(),
                        target: local_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(local_source.clone(), local.items, Vec::new()),
            ],
            vec![main_source, local_source],
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

    #[test]
    fn derives_struct_layout_fields_and_raw_address() {
        let result = validate_text(
            "%%start\nstruct WasiIovec {\n\t*?u8 buf;\n\tu32 len;\n}\nunsafe {\n\tWasiIovec item = {\n\t\t.buf = null;\n\t\t.len = 0;\n\t};\n\t*?WasiIovec address = &?item;\n};\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.program.structs[0].fields[0].offset, 0);
        assert_eq!(result.program.structs[0].fields[1].offset, 4);
        assert_eq!(result.program.structs[0].layout.size, 8);
        assert_eq!(result.program.target_layout, ScalarTargetLayout::WASM32);
        assert_eq!(
            result.program.structs[0].id,
            ScalarStructId {
                source: source(),
                index: 0
            }
        );
        assert_eq!(
            result.program.structs[0].fields[0].id,
            ScalarStructFieldId {
                structure: ScalarStructId {
                    source: source(),
                    index: 0
                },
                index: 0,
            }
        );
        let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            &result.program.items[0]
        else {
            panic!("unsafe block")
        };
        let ScalarBlockItem::LocalBinding(address) = &block.items[1] else {
            panic!("address binding")
        };
        assert!(matches!(address.value, ScalarExpression::RawAddress { .. }));
    }

    #[test]
    fn rejects_string_literal_without_std_context() {
        let result = validate_text("%%start\ni32 value = \"text\";\n%%end");
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("std.utf8 context")));
    }

    #[test]
    fn validates_multiple_output_calls_and_output_arity() {
        let valid = validate_text(
            "%%start\n(u64, bool)() pair = fn { 1 };\nu64 first, bool second = pair<u32>();\n%%end",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let ScalarItem::Binding(binding) = &valid.program.items[1] else {
            panic!("multi-output binding")
        };
        assert_eq!(binding.output_sequence.outputs.len(), 2);
        assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::U64);
        assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::Bool);
        let call_span = expression_span(&binding.value);
        assert_eq!(binding.output_sequence.outputs[0].span, call_span);
        assert_eq!(binding.output_sequence.outputs[1].span, call_span);
        assert_eq!(binding.output_values[0].position, 0);
        assert_eq!(binding.output_values[1].position, 1);
        assert_eq!(binding.output_values[0].ty, ScalarType::U64);
        assert_eq!(binding.output_values[1].ty, ScalarType::Bool);
        let wrong_type_text =
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first, i32 second = pair();\n%%end";
        let wrong_type = validate_text(wrong_type_text);
        let second_receiver_start =
            wrong_type_text.find("i32 second").expect("second receiver") as u32;
        let second_receiver_end = second_receiver_start + "i32 second".len() as u32;
        assert!(wrong_type.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "expression type does not match expected type"
                && diagnostic.labels.iter().any(|label| {
                    label.span.range == ByteSpan::new(second_receiver_start, second_receiver_end)
                })
        }));

        let wrong_arity =
            validate_text("%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = pair();\n%%end");
        assert!(wrong_arity
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "call has more outputs than receivers"));

        let too_many_text =
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first, u64 second, bool third = pair();\n%%end";
        let too_many = validate_text(too_many_text);
        let too_many_start = too_many_text.find("i32 first").expect("binding start") as u32;
        let too_many_end = too_many_text.find(";\n%%end").expect("binding end") as u32 + 1;
        assert!(too_many.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "call has fewer outputs than receivers"
                && diagnostic.labels[0].span.range == ByteSpan::new(too_many_start, too_many_end)
        }));

        let scalar = validate_text("%%start\n(i32, u64)() pair = fn { 1 };\npair();\n%%end");
        let output_span = ByteSpan::new(8, 20);
        assert!(scalar.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "multi-output call requires output receivers"
                && diagnostic.labels[0].span.range == output_span
        }));
    }

    #[test]
    fn preserves_independent_output_expression_spans() {
        let result = validate_text("%%start\nu64 first, bool second = 1, true;\n%%end");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("output binding");
        };
        assert_eq!(binding.output_values[0].position, 0);
        assert_eq!(binding.output_values[1].position, 1);
        assert_ne!(
            expression_span(&binding.output_values[0].value),
            expression_span(&binding.output_values[1].value)
        );
        assert_eq!(
            binding.output_sequence.outputs[0].span,
            binding.output_values[0].span
        );
        assert_eq!(
            binding.output_sequence.outputs[1].span,
            binding.output_values[1].span
        );
    }

    #[test]
    fn preserves_one_output_call_contexts_for_strict_integers_and_null() {
        let result = validate_text(
            "%%start\nu32(u32) identity = fn(value) { value };\nu32 number = identity(1);\nu32(*?u8) keep = fn(value) { if (value == null) { 1 } else { 1 } };\nu32 result = keep(null);\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn rejects_typed_i32_for_u32_and_contextually_types_u32_literals() {
        let declaration = validate_text("%%start\ni32 value = 1;\nu32 result = value;\n%%end");
        assert_eq!(
            declaration
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message
                    == "expression type does not match expected type")
                .count(),
            1
        );

        let assignment =
            validate_text("%%start\ni32 value = 1;\nu32 target = 2;\ntarget = value;\n%%end");
        assert!(
            assignment
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message
                    == "expression type does not match expected type")
        );

        let callable = validate_text(
            "%%start\nu32(u32) identity = fn(value) { value };\ni32 value = 1;\nu32 result = identity(value);\n%%end",
        );
        assert!(
            callable
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message
                    == "expression type does not match expected type")
        );

        let literal = validate_text("%%start\nu32 value = 1 + 2;\n%%end");
        assert!(literal.diagnostics.is_empty(), "{:?}", literal.diagnostics);
        let out_of_range = validate_text("%%start\nu32 value = 4294967296;\n%%end");
        assert!(out_of_range
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0010"));
    }

    #[test]
    fn resolves_contextual_null_in_declarations_assignments_calls_and_nested_expressions() {
        let text = "%%start\nu32(*?u8) keep = fn(value) { if (value == null) { 1 } else { 1 } };\n*?u8 pointer = null;\npointer = null;\nu32 result = keep(null);\n*?u8 nested = if (true) { if (true) { null } else { null } } else { null };\nbool same = pointer == null;\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("pointer declaration");
        };
        assert_eq!(
            binding.declared_type,
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        );
        let ScalarItem::Binding(nested) = &result.program.items[4] else {
            panic!("nested pointer declaration");
        };
        assert_eq!(
            nested.declared_type,
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        );
    }

    #[test]
    fn reports_unconstrained_null_once_at_the_null_token_span() {
        let cases = [
            "%%start\nnull;\n%%end",
            "%%start\ni32 value = 1;\nvalue = null;\n%%end",
            "%%start\ni32(i32) identity = fn(value) { value };\ni32 result = identity(null);\n%%end",
            "%%start\ni32 value = if (true) { null } else { 1 };\n%%end",
        ];
        for text in cases {
            let result = validate_text(text);
            let diagnostics: Vec<_> = result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message == "null requires a pointer context")
                .collect();
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            let start = text.find("null").expect("null token") as u32;
            assert_eq!(diagnostics[0].labels.len(), 1);
            assert_eq!(
                diagnostics[0].labels[0].span.range,
                ByteSpan::new(start, start + 4)
            );
        }
    }

    #[test]
    fn project_validation_applies_exact_types_and_contextual_null() {
        let dependency_source = module_source("src/dependency.w");
        let dependency = module_from_text(
            dependency_source.clone(),
            "%%start\nu32(*?u8) accept = fn(value) { 1 };\ni32 value = 1;\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\ndependency = namespace app \"src/dependency.w\";\ni32 value = 1;\nu32 rejected = dependency.accept(value);\nu32 nested = 1 + (2 + 3);\n*?u8 pointer = null;\n*?u8 selected = if (true) { null } else { null };\nbool same = pointer == (if (true) { null } else { null });\nbool nested_comparison = (pointer == null) == true;\n%%end",
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
                        binding: "dependency".to_owned(),
                        target: dependency_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(dependency_source, dependency.items, Vec::new()),
            ],
            vec![main_source],
        ));
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message
                    == "expression type does not match expected type")
        );
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "null requires a pointer context"));
    }

    #[test]
    fn resolves_forward_struct_types_and_canonical_field_places() {
        let result = validate_text(
            "%%start\nstruct Outer {\n\tInner inner;\n\t*?Inner pointer;\n}\nstruct Inner {\n\tu32 value;\n}\nunsafe {\n\tOuter item = { .inner = { .value = 1; }; .pointer = null; };\n\t*?Outer address = &?item;\n\t*?u32 value_address = &?(*item.pointer).value;\n\t*?Inner pointer_value_address = &?(*address).inner;\n};\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result.program.structs[0].id,
            ScalarStructId {
                source: source(),
                index: 0
            }
        );
        assert_eq!(
            result.program.structs[1].id,
            ScalarStructId {
                source: source(),
                index: 1
            }
        );
        assert_eq!(
            result.program.structs[0].fields[0].ty,
            ScalarType::Struct(ScalarStructId {
                source: source(),
                index: 1
            })
        );
        let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            &result.program.items[0]
        else {
            panic!("unsafe block")
        };
        let ScalarBlockItem::LocalBinding(value_address) = &block.items[2] else {
            panic!("value address binding")
        };
        let ScalarExpression::RawAddress { place, .. } = &value_address.value else {
            panic!("raw value address")
        };
        let ScalarPlace::Field { field, base, .. } = place else {
            panic!("nested field place")
        };
        assert_eq!(
            *field,
            ScalarFieldReference::Resolved(ScalarStructFieldId {
                structure: ScalarStructId {
                    source: source(),
                    index: 1
                },
                index: 0,
            })
        );
        let ScalarPlace::Dereference { .. } = base.as_ref() else {
            panic!("pointer dereference place")
        };
    }

    #[test]
    fn validates_struct_literals_in_nested_positions_and_pointer_null_fields() {
        let result = validate_text(
            "%%start\nstruct Pair {\n\tu32 left;\n\t*?u8 right;\n}\nPair value = if (true) { { .left = 1; .right = null; } } else { { .left = 0; .right = null; } };\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("struct binding")
        };
        assert!(matches!(binding.value, ScalarExpression::If { .. }));
        assert_eq!(
            binding.declared_type,
            ScalarType::Struct(ScalarStructId {
                source: source(),
                index: 0
            })
        );
    }

    #[test]
    fn reports_struct_literal_field_shape_errors_at_field_spans() {
        let cases = [
            (
                ".left = 1; .left = 2; .right = null;",
                "duplicate struct literal field",
                ".left = 2",
                4,
            ),
            (
                ".left = 1; .unknown = 2;",
                "unknown struct field",
                ".unknown",
                7,
            ),
            (
                ".left = 1;",
                "struct literal must initialize every field",
                "{ .left",
                0,
            ),
        ];
        for (fields, message, span_text, field_length) in cases {
            let text = format!(
                "%%start\nstruct Pair {{\n\tu32 left;\n\tu32 right;\n}}\nPair value = {{ {fields} }};\n%%end"
            );
            let result = validate_text(&text);
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message == message)
                .expect("struct literal diagnostic");
            assert_eq!(diagnostic.labels[0].message, message);
            if message == "struct literal must initialize every field" {
                let ScalarItem::Binding(binding) = &result.program.items[0] else {
                    panic!("struct binding")
                };
                let ScalarExpression::StructLiteral { span, .. } = &binding.value else {
                    panic!("struct literal")
                };
                assert_eq!(diagnostic.labels[0].span.range, *span);
            } else {
                let start = text.find(span_text).expect("diagnostic span text") as u32 + 1;
                let end = start + field_length;
                assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, end));
            }
        }
    }

    #[test]
    fn enforces_unsafe_raw_addresses_and_rejects_invalid_field_receivers() {
        let safe = validate_text(
            "%%start\nstruct Pair {\n\tu32 value;\n}\nPair item = { .value = 1; };\n*?Pair address = &?item;\n%%end",
        );
        let diagnostic = safe
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0012")
            .expect("raw address safety diagnostic");
        assert_eq!(diagnostic.message, "raw address requires an unsafe block");
        let invalid = validate_text(
            "%%start\ni32 value = 1;\nunsafe {\n\t*?i32 address = &?value.missing;\n};\n%%end",
        );
        let diagnostic = invalid
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "value has no field")
            .expect("invalid field receiver diagnostic");
        let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            &invalid.program.items[1]
        else {
            panic!("unsafe block")
        };
        let ScalarBlockItem::LocalBinding(binding) = &block.items[0] else {
            panic!("address binding")
        };
        let ScalarExpression::RawAddress { place, .. } = &binding.value else {
            panic!("raw address")
        };
        assert_eq!(
            diagnostic.labels[0].span.range,
            match place {
                ScalarPlace::Field { span, .. } => *span,
                _ => panic!("field place"),
            }
        );
    }

    #[test]
    fn validates_structs_through_project_modules() {
        let source = module_source("src/main.w");
        let program = module_from_text(
            source.clone(),
            "%%start\nstruct Outer {\n\tPair pair;\n}\nstruct Pair {\n\tu32 value;\n}\nraw = extern wasm \"env\" { unit(*?Pair) touch; };\nunit(Outer) consume = fn(value) { unsafe { *?Outer pointer = &?value; *?Pair field_address = &?(*pointer).pair; raw.touch(field_address); }; };\nOuter item = { .pair = { .value = 1; }; };\n%%end",
        );
        let struct_id = program.structs[0].id.clone();
        let pair_id = program.structs[1].id.clone();
        let struct_layout = program.structs[0].layout.clone();
        let result = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.project.modules[0].structs[0].id, struct_id.clone());
        assert_eq!(result.project.modules[0].structs[0].layout, struct_layout);
        let ScalarItem::Extern(extern_decl) = &result.project.modules[0].items[0] else {
            panic!("extern declaration");
        };
        let ScalarType::Callable { parameters, .. } = &extern_decl.functions[0].signature else {
            panic!("extern signature");
        };
        assert!(
            matches!(parameters[0], ScalarType::RawPointer(ref inner) if inner.as_ref() == &ScalarType::Struct(pair_id.clone()))
        );
        let ScalarItem::Function(function) = &result.project.modules[0].items[1] else {
            panic!("callable declaration");
        };
        let ScalarType::Callable { parameters, .. } = &function.signature else {
            panic!("callable signature");
        };
        assert_eq!(parameters, &[ScalarType::Struct(struct_id.clone())]);
        let ScalarBlockItem::Expression(ScalarExpression::Block(block)) = &function.body.items[0]
        else {
            panic!("unsafe block");
        };
        let ScalarBlockItem::LocalBinding(binding) = &block.items[1] else {
            panic!("field address binding");
        };
        let ScalarExpression::RawAddress {
            place: ScalarPlace::Field { field, .. },
            ..
        } = &binding.value
        else {
            panic!("field address");
        };
        assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == struct_id));
        let ScalarItem::Binding(binding) = &result.project.modules[0].items[2] else {
            panic!("struct binding");
        };
        assert_eq!(binding.declared_type, ScalarType::Struct(struct_id));
    }

    #[test]
    fn resolves_imported_struct_identity_for_literals_and_field_addresses() {
        let child_source = module_source("src/child.w");
        let child_program = module_from_text(
            child_source.clone(),
            "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main_program = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\nunsafe { *?child.Pair pointer = &?item; *?u32 address = &?(*pointer).second; };\n%%end",
        );
        let imported_id = child_program.structs[0].id.clone();
        let namespace = match &main_program.items[0] {
            ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
            _ => panic!("namespace item"),
        };
        let project = ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main_program,
                    vec![ScalarNamespaceBinding {
                        binding: namespace.0,
                        target: child_source.clone(),
                        span: namespace.1,
                    }],
                ),
                ScalarModule::from_program(child_program, Vec::new()),
            ],
            vec![main_source.clone(), child_source],
        );
        let validation = validate_scalar_project(project);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let ScalarItem::Binding(binding) = &validation.project.modules[0].items[1] else {
            panic!("struct binding")
        };
        assert_eq!(
            binding.declared_type,
            ScalarType::Struct(imported_id.clone())
        );
        let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            &validation.project.modules[0].items[2]
        else {
            panic!("unsafe block")
        };
        let ScalarBlockItem::LocalBinding(binding) = &block.items[1] else {
            panic!("address binding")
        };
        let ScalarExpression::RawAddress {
            place: ScalarPlace::Field { field, .. },
            ..
        } = &binding.value
        else {
            panic!("field address")
        };
        assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
    }

    #[test]
    fn resolves_imported_struct_field_reads_and_preserves_namespace_members() {
        let std_source = module_source("src/bootstrap.w");
        let std = module_from_text(
            std_source.clone(),
            "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\ni32 value = 80;\n%%end",
        );
        let imported_id = std.structs[0].id.clone();
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nstd = namespace std \"src/bootstrap.w\";\ni32 observed = std.value;\nstd.utf8 text = \"hé\";\n*?u8 data = text.data;\nu64 length = text.length;\nunsafe { *?std.utf8 pointer = &?text; *?u8 pointer_data = pointer.data; u64 pointer_length = pointer.length; };\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source,
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "std".to_owned(),
                        target: std_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::from_program(std, Vec::new()),
            ],
            Vec::new(),
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let ScalarItem::Binding(observed) = &validation.project.modules[0].items[1] else {
            panic!("namespace member binding")
        };
        assert!(matches!(
            observed.value,
            ScalarExpression::Member { ref receiver, ref name, .. }
                if receiver == "std" && name == "value"
        ));
        let ScalarItem::Binding(text) = &validation.project.modules[0].items[2] else {
            panic!("utf8 binding")
        };
        assert_eq!(text.declared_type, ScalarType::Struct(imported_id.clone()));
        assert!(matches!(
            text.value,
            ScalarExpression::Utf8 { ref value, .. } if value == b"h\xc3\xa9"
        ));
        let ScalarItem::Binding(data) = &validation.project.modules[0].items[3] else {
            panic!("data binding")
        };
        assert_eq!(
            data.declared_type,
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        );
        let ScalarItem::Binding(length) = &validation.project.modules[0].items[4] else {
            panic!("length binding")
        };
        assert_eq!(length.declared_type, ScalarType::U64);
    }

    #[test]
    fn keeps_same_index_structs_distinct_by_declaring_source() {
        let first_source = module_source("src/first.w");
        let first = module_from_text(
            first_source.clone(),
            "%%start\nstruct Pair { u32 value; }\n%%end",
        );
        let second_source = module_source("src/second.w");
        let second = module_from_text(
            second_source.clone(),
            "%%start\nstruct Pair { u32 value; }\n%%end",
        );
        assert_eq!(first.structs[0].id.index, second.structs[0].id.index);
        assert_ne!(first.structs[0].id, second.structs[0].id);
        assert_eq!(first.structs[0].id.source, first_source);
        assert_eq!(second.structs[0].id.source, second_source);
    }

    #[test]
    fn derives_the_complete_integer_surface_and_radix_values() {
        let program = module_from_text(
            module_source("src/integers.w"),
            r#"%%start
i8 a = 127;
i16 b = 0x7fff;
i32 c = 2_147_483_647;
i64 d = 0x7fff_ffff_ffff_ffff;
i128 e = 0x7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;
u8 f = 255;
u16 g = 0xffff;
u32 h = 0xffff_ffff;
u64 i = 0xffff_ffff_ffff_ffff;
u128 j = 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;
%%end"#,
        );
        assert!(program.items.iter().all(|item| match item {
            ScalarItem::Binding(binding) =>
                !matches!(binding.declared_type, ScalarType::Named { .. }),
            _ => true,
        }));
        assert!(program.items.iter().all(|item| match item {
            ScalarItem::Binding(binding) => {
                matches!(binding.value, ScalarExpression::Integer { .. })
            }
            _ => true,
        }));
    }

    #[test]
    fn derives_contextual_finite_float_literals_across_scalar_positions() {
        let text = "%%start\nf32(f32) narrow = fn(value) { value };\nf64(f64) wide = fn(value) { value };\nenv = extern wasm \"env\" { unit(f32, f64) consume; };\nf32 first = 1_2.5e-1;\nf64 second = 2E+3;\nf32 sum = first + 2.5;\nbool ordered = second < 3e3;\nf32 echoed = narrow(4e-1);\nf64 widened = wide(5.0);\nenv.consume(6.0, 7e1);\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(first) = &result.program.items[3] else {
            panic!("first float binding")
        };
        let ScalarExpression::Float { spelling, span, .. } = &first.value else {
            panic!("first float literal")
        };
        assert_eq!(spelling, "1_2.5e-1");
        let start = text.find(spelling).expect("float literal") as u32;
        assert_eq!(*span, ByteSpan::new(start, start + spelling.len() as u32));
        assert_eq!(first.declared_type, ScalarType::F32);
        let ScalarItem::Binding(second) = &result.program.items[4] else {
            panic!("second float binding")
        };
        assert_eq!(second.declared_type, ScalarType::F64);
    }

    #[test]
    fn rejects_invalid_and_out_of_range_float_literals_at_literal_spans() {
        for literal in ["1__2.0", "1._2", "1e_2", "1e400"] {
            let text = format!("%%start\nf64 value = {literal};\n%%end");
            let result = validate_text(&text);
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "B0010")
                .expect("float diagnostic");
            let start = text.find(literal).expect("float literal") as u32;
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + literal.len() as u32)
            );
        }
    }

    #[test]
    fn rejects_float_width_and_integer_float_mismatches() {
        for text in [
            "%%start\nf32 narrow = 1.0;\nf64 wide = narrow;\n%%end",
            "%%start\nf64 wide = 1.0;\nf32 narrow = wide;\n%%end",
            "%%start\ni32 integer = 1;\nf32 decimal = integer;\n%%end",
            "%%start\nf64 decimal = 1.0;\ni32 integer = decimal;\n%%end",
        ] {
            let result = validate_text(text);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "B0003"),
                "{text}: {:?}",
                result.diagnostics
            );
        }
    }
}
