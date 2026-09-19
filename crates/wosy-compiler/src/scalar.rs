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
    CheckedReference {
        mutability: ScalarReferenceMutability,
        inner: Box<ScalarType>,
    },
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
    Enum(ScalarEnumId),
    Array {
        element: Box<ScalarType>,
        length: u64,
        length_span: ByteSpan,
        span: ByteSpan,
    },
    RuntimeArray {
        element: Box<ScalarType>,
        span: ByteSpan,
    },
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarReferenceMutability {
    Shared,
    Mutable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructId {
    pub source: SourceIdentity,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ScalarAllocationIdentity {
    pub binding: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarAutomaticReturnResultState {
    Live,
    Transferred,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAutomaticReturnResult {
    pub aggregate_type: ScalarStructId,
    pub allocation_identity: ScalarAllocationIdentity,
    pub state: ScalarAutomaticReturnResultState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarEnumId {
    pub source: SourceIdentity,
    pub index: usize,
}

impl Ord for ScalarEnumId {
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

impl PartialOrd for ScalarEnumId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
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

    pub const NATIVE64: Self = Self {
        pointer_size: 8,
        pointer_alignment: 8,
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
    Index {
        base: Box<ScalarPlace>,
        index: Box<ScalarExpression>,
        span: ByteSpan,
        index_span: ByteSpan,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarPlaceIdentity {
    Name(String),
    Field(Box<ScalarPlaceIdentity>, ScalarFieldIdentity),
    Dereference(Box<ScalarExpressionIdentity>),
    Index(Box<ScalarPlaceIdentity>, Box<ScalarExpressionIdentity>),
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarFieldIdentity {
    Unresolved(String),
    Resolved(ScalarStructFieldId),
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarExpressionIdentity {
    Name(String),
    Member(String, String),
    Integer(BigInt),
    InvalidInteger,
    Float(f64),
    InvalidFloat,
    Boolean(bool),
    Char(char),
    Utf8(Vec<u8>),
    RawAddress(ScalarPlaceIdentity),
    CheckedAddress(ScalarReferenceMutability, ScalarPlaceIdentity),
    Dereference(ScalarPlaceIdentity),
    IndexedRead(ScalarPlaceIdentity),
    StructLiteral(Vec<(String, ScalarExpressionIdentity)>),
    ArrayLiteral(Vec<ScalarExpressionIdentity>),
    Binary(
        BinaryOperator,
        Box<ScalarExpressionIdentity>,
        Box<ScalarExpressionIdentity>,
    ),
    Unary(UnaryOperator, Box<ScalarExpressionIdentity>),
    Call(
        Option<String>,
        String,
        Vec<ScalarTypeIdentity>,
        Vec<ScalarExpressionIdentity>,
    ),
    If(
        Box<ScalarExpressionIdentity>,
        ScalarBlockIdentity,
        ScalarBlockIdentity,
    ),
    UnitIf(Box<ScalarExpressionIdentity>, ScalarBlockIdentity),
    Block(ScalarBlockIdentity),
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarTypeIdentity {
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
    RawPointer(Box<ScalarTypeIdentity>),
    CheckedReference(ScalarReferenceMutability, Box<ScalarTypeIdentity>),
    Callable(Vec<ScalarTypeIdentity>, Vec<ScalarTypeIdentity>),
    Named(String),
    Qualified(String, String),
    Struct(ScalarStructId),
    Enum(ScalarEnumId),
    Array(Box<ScalarTypeIdentity>, u64),
    RuntimeArray(Box<ScalarTypeIdentity>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
struct ScalarBlockIdentity {
    items: Vec<ScalarBlockItemIdentity>,
    expressions: Vec<ScalarExpressionIdentity>,
    final_output_values: Vec<(usize, ScalarTypeIdentity, ScalarExpressionIdentity)>,
    unsafe_context: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarBlockItemIdentity {
    LocalBinding(
        String,
        ScalarTypeIdentity,
        ScalarExpressionIdentity,
        Vec<(String, ScalarTypeIdentity)>,
    ),
    Expression(ScalarExpressionIdentity),
    Assignment(Vec<ScalarPlaceIdentity>, Vec<ScalarExpressionIdentity>),
    While(ScalarExpressionIdentity, ScalarBlockIdentity),
}

fn scalar_place_identity(place: &ScalarPlace) -> ScalarPlaceIdentity {
    match place {
        ScalarPlace::Name { name, .. } => ScalarPlaceIdentity::Name(name.clone()),
        ScalarPlace::Field { base, field, .. } => ScalarPlaceIdentity::Field(
            Box::new(scalar_place_identity(base)),
            match field {
                ScalarFieldReference::Unresolved { name, .. } => {
                    ScalarFieldIdentity::Unresolved(name.clone())
                }
                ScalarFieldReference::Resolved(id) => ScalarFieldIdentity::Resolved(id.clone()),
            },
        ),
        ScalarPlace::Dereference { pointer, .. } => {
            ScalarPlaceIdentity::Dereference(Box::new(scalar_expression_identity(pointer)))
        }
        ScalarPlace::Index { base, index, .. } => ScalarPlaceIdentity::Index(
            Box::new(scalar_place_identity(base)),
            Box::new(scalar_expression_identity(index)),
        ),
    }
}

fn scalar_place_target_span(place: &ScalarPlace) -> ByteSpan {
    match place {
        ScalarPlace::Name { span, .. } | ScalarPlace::Dereference { span, .. } => *span,
        ScalarPlace::Field { base, span, .. } | ScalarPlace::Index { base, span, .. } => {
            ByteSpan::new(scalar_place_target_span(base).start, span.end)
        }
    }
}

fn scalar_expression_identity(expression: &ScalarExpression) -> ScalarExpressionIdentity {
    match expression {
        ScalarExpression::Name { name, .. } => ScalarExpressionIdentity::Name(name.clone()),
        ScalarExpression::Member { receiver, name, .. } => {
            ScalarExpressionIdentity::Member(receiver.clone(), name.clone())
        }
        ScalarExpression::Integer { value, .. } => ScalarExpressionIdentity::Integer(value.clone()),
        ScalarExpression::InvalidInteger { .. } => ScalarExpressionIdentity::InvalidInteger,
        ScalarExpression::Float { value, .. } => ScalarExpressionIdentity::Float(*value),
        ScalarExpression::InvalidFloat { .. } => ScalarExpressionIdentity::InvalidFloat,
        ScalarExpression::Boolean { value, .. } => ScalarExpressionIdentity::Boolean(*value),
        ScalarExpression::Char { value, .. } => ScalarExpressionIdentity::Char(*value),
        ScalarExpression::Utf8 { value, .. } => ScalarExpressionIdentity::Utf8(value.clone()),
        ScalarExpression::RawAddress { place, .. } => {
            ScalarExpressionIdentity::RawAddress(scalar_place_identity(place))
        }
        ScalarExpression::CheckedAddress {
            mutability, place, ..
        } => ScalarExpressionIdentity::CheckedAddress(*mutability, scalar_place_identity(place)),
        ScalarExpression::Dereference { place, .. } => {
            ScalarExpressionIdentity::Dereference(scalar_place_identity(place))
        }
        ScalarExpression::IndexedRead { place, .. } => {
            ScalarExpressionIdentity::IndexedRead(scalar_place_identity(place))
        }
        ScalarExpression::StructLiteral { fields, .. } => ScalarExpressionIdentity::StructLiteral(
            fields
                .iter()
                .map(|field| (field.name.clone(), scalar_expression_identity(&field.value)))
                .collect(),
        ),
        ScalarExpression::ArrayLiteral { elements, .. } => ScalarExpressionIdentity::ArrayLiteral(
            elements.iter().map(scalar_expression_identity).collect(),
        ),
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => ScalarExpressionIdentity::Binary(
            operator.clone(),
            Box::new(scalar_expression_identity(left)),
            Box::new(scalar_expression_identity(right)),
        ),
        ScalarExpression::Unary {
            operator, operand, ..
        } => ScalarExpressionIdentity::Unary(
            operator.clone(),
            Box::new(scalar_expression_identity(operand)),
        ),
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            ..
        } => ScalarExpressionIdentity::Call(
            receiver.clone(),
            name.clone(),
            type_arguments
                .iter()
                .map(|argument| scalar_type_identity(&argument.ty))
                .collect(),
            arguments.iter().map(scalar_expression_identity).collect(),
        ),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => ScalarExpressionIdentity::If(
            Box::new(scalar_expression_identity(condition)),
            scalar_block_identity(then_branch),
            scalar_block_identity(else_branch),
        ),
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => ScalarExpressionIdentity::UnitIf(
            Box::new(scalar_expression_identity(condition)),
            scalar_block_identity(then_branch),
        ),
        ScalarExpression::Block(block) => {
            ScalarExpressionIdentity::Block(scalar_block_identity(block))
        }
    }
}

fn scalar_type_identity(ty: &ScalarType) -> ScalarTypeIdentity {
    match ty {
        ScalarType::Unit => ScalarTypeIdentity::Unit,
        ScalarType::Bool => ScalarTypeIdentity::Bool,
        ScalarType::I8 => ScalarTypeIdentity::I8,
        ScalarType::I16 => ScalarTypeIdentity::I16,
        ScalarType::I32 => ScalarTypeIdentity::I32,
        ScalarType::I64 => ScalarTypeIdentity::I64,
        ScalarType::I128 => ScalarTypeIdentity::I128,
        ScalarType::U8 => ScalarTypeIdentity::U8,
        ScalarType::U16 => ScalarTypeIdentity::U16,
        ScalarType::U32 => ScalarTypeIdentity::U32,
        ScalarType::U64 => ScalarTypeIdentity::U64,
        ScalarType::U128 => ScalarTypeIdentity::U128,
        ScalarType::F32 => ScalarTypeIdentity::F32,
        ScalarType::F64 => ScalarTypeIdentity::F64,
        ScalarType::Char => ScalarTypeIdentity::Char,
        ScalarType::ArtifactId => ScalarTypeIdentity::ArtifactId,
        ScalarType::RawPointer(inner) => {
            ScalarTypeIdentity::RawPointer(Box::new(scalar_type_identity(inner)))
        }
        ScalarType::CheckedReference { mutability, inner } => {
            ScalarTypeIdentity::CheckedReference(*mutability, Box::new(scalar_type_identity(inner)))
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarTypeIdentity::Callable(
            outputs
                .outputs
                .iter()
                .map(|output| scalar_type_identity(&output.ty))
                .collect(),
            parameters.iter().map(scalar_type_identity).collect(),
        ),
        ScalarType::Named { name, .. } => ScalarTypeIdentity::Named(name.clone()),
        ScalarType::Qualified {
            receiver, member, ..
        } => ScalarTypeIdentity::Qualified(receiver.clone(), member.clone()),
        ScalarType::Struct(id) => ScalarTypeIdentity::Struct(id.clone()),
        ScalarType::Enum(id) => ScalarTypeIdentity::Enum(id.clone()),
        ScalarType::Array {
            element, length, ..
        } => ScalarTypeIdentity::Array(Box::new(scalar_type_identity(element)), *length),
        ScalarType::RuntimeArray { element, .. } => {
            ScalarTypeIdentity::RuntimeArray(Box::new(scalar_type_identity(element)))
        }
        ScalarType::Error => ScalarTypeIdentity::Error,
    }
}

fn scalar_block_identity(block: &ScalarBlock) -> ScalarBlockIdentity {
    ScalarBlockIdentity {
        items: block
            .items
            .iter()
            .map(|item| match item {
                ScalarBlockItem::LocalBinding(binding) => ScalarBlockItemIdentity::LocalBinding(
                    binding.name.clone(),
                    scalar_type_identity(&binding.declared_type),
                    scalar_expression_identity(&binding.value),
                    binding
                        .receivers
                        .iter()
                        .map(|receiver| (receiver.name.clone(), scalar_type_identity(&receiver.ty)))
                        .collect(),
                ),
                ScalarBlockItem::Expression(expression) => {
                    ScalarBlockItemIdentity::Expression(scalar_expression_identity(expression))
                }
                ScalarBlockItem::Assignment(assignment) => ScalarBlockItemIdentity::Assignment(
                    assignment
                        .targets
                        .iter()
                        .map(|target| scalar_place_identity(&target.place))
                        .collect(),
                    assignment
                        .values
                        .iter()
                        .map(scalar_expression_identity)
                        .collect(),
                ),
                ScalarBlockItem::While(while_expression) => ScalarBlockItemIdentity::While(
                    scalar_expression_identity(&while_expression.condition),
                    scalar_block_identity(&while_expression.body),
                ),
            })
            .collect(),
        expressions: block
            .expressions
            .iter()
            .map(scalar_expression_identity)
            .collect(),
        final_output_values: block
            .final_output_values
            .iter()
            .map(|value| {
                (
                    value.position,
                    scalar_type_identity(&value.ty),
                    scalar_expression_identity(&value.value),
                )
            })
            .collect(),
        unsafe_context: block.unsafe_context,
    }
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
    pub offset: Option<u64>,
    pub layout: Option<ScalarLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStruct {
    pub id: ScalarStructId,
    pub name: String,
    pub name_span: ByteSpan,
    pub fields: Vec<ScalarStructField>,
    pub span: ByteSpan,
    pub layout: Option<ScalarLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarEnumVariant {
    pub name: String,
    pub name_span: ByteSpan,
    pub tag: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarEnum {
    pub id: ScalarEnumId,
    pub name: String,
    pub name_span: ByteSpan,
    pub variants: Vec<ScalarEnumVariant>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputReceiver {
    pub name: String,
    pub name_span: ByteSpan,
    pub ty: ScalarType,
    pub allocation_length: Option<ScalarExpression>,
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
pub enum ScalarBindingOutputOrigin {
    SingleExpression,
    IndependentExpressions,
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
    if is_error_type(left) || is_error_type(right) {
        return false;
    }
    match (left, right) {
        (ScalarType::RawPointer(left), ScalarType::RawPointer(right)) => {
            scalar_type_equal(left, right)
        }
        (
            ScalarType::CheckedReference {
                mutability: left_mutability,
                inner: left_inner,
            },
            ScalarType::CheckedReference {
                mutability: right_mutability,
                inner: right_inner,
            },
        ) => left_mutability == right_mutability && scalar_type_equal(left_inner, right_inner),
        (
            ScalarType::Array {
                element: left_element,
                length: left_length,
                ..
            },
            ScalarType::Array {
                element: right_element,
                length: right_length,
                ..
            },
        ) => left_length == right_length && scalar_type_equal(left_element, right_element),
        (
            ScalarType::RuntimeArray { element: left, .. },
            ScalarType::RuntimeArray { element: right, .. },
        ) => scalar_type_equal(left, right),
        (
            ScalarType::RuntimeArray { element: left, .. },
            ScalarType::Array { element: right, .. },
        )
        | (
            ScalarType::Array { element: left, .. },
            ScalarType::RuntimeArray { element: right, .. },
        ) => scalar_type_equal(left, right),
        (ScalarType::Named { name: left, .. }, ScalarType::Named { name: right, .. }) => {
            left == right
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
        (ScalarType::CheckedReference { .. }, _) | (_, ScalarType::CheckedReference { .. }) => {
            false
        }
        (ScalarType::Callable { .. }, _) | (_, ScalarType::Callable { .. }) => false,
        _ => left == right,
    }
}

fn substitute_type(ty: &ScalarType, substitutions: &BTreeMap<String, ScalarType>) -> ScalarType {
    match ty {
        ScalarType::Named { name, .. } => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(substitute_type(inner, substitutions)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(substitute_type(inner, substitutions)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(substitute_type(element, substitutions)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: substitute_type(&output.ty, substitutions),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| substitute_type(parameter, substitutions))
                .collect(),
        },
        _ => ty.clone(),
    }
}

fn substitute_generic_callable(
    signature: &ScalarType,
    parameters: &[ScalarGenericParameter],
    type_arguments: &[ScalarTypeArgument],
) -> Option<ScalarType> {
    if parameters.is_empty() {
        return Some(signature.clone());
    }
    if parameters.len() != type_arguments.len() {
        return None;
    }
    let substitutions = parameters
        .iter()
        .zip(type_arguments)
        .map(|(parameter, argument)| (parameter.name.clone(), argument.ty.clone()))
        .collect();
    Some(substitute_type(signature, &substitutions))
}

fn infer_overload_substitutions(
    pattern: &ScalarType,
    actual: &ScalarType,
    parameters: &[ScalarGenericParameter],
    substitutions: &mut BTreeMap<String, ScalarType>,
) -> bool {
    if let ScalarType::Named { name, .. } = pattern {
        if parameters.iter().any(|parameter| parameter.name == *name) {
            if let Some(previous) = substitutions.get(name) {
                return scalar_type_equal(previous, actual);
            }
            substitutions.insert(name.clone(), actual.clone());
            return true;
        }
    }
    match (pattern, actual) {
        (ScalarType::RawPointer(pattern), ScalarType::RawPointer(actual)) => {
            infer_overload_substitutions(pattern, actual, parameters, substitutions)
        }
        (
            ScalarType::CheckedReference {
                mutability: pattern_mutability,
                inner: pattern_inner,
            },
            ScalarType::CheckedReference {
                mutability: actual_mutability,
                inner: actual_inner,
            },
        ) if pattern_mutability == actual_mutability => {
            infer_overload_substitutions(pattern_inner, actual_inner, parameters, substitutions)
        }
        (
            ScalarType::Array {
                element: pattern_element,
                length: pattern_length,
                ..
            },
            ScalarType::Array {
                element: actual_element,
                length: actual_length,
                ..
            },
        ) if pattern_length == actual_length => {
            infer_overload_substitutions(pattern_element, actual_element, parameters, substitutions)
        }
        (
            ScalarType::Callable {
                outputs: pattern_outputs,
                parameters: pattern_parameters,
            },
            ScalarType::Callable {
                outputs: actual_outputs,
                parameters: actual_parameters,
            },
        ) => {
            pattern_outputs.outputs.len() == actual_outputs.outputs.len()
                && pattern_parameters.len() == actual_parameters.len()
                && pattern_outputs
                    .outputs
                    .iter()
                    .zip(&actual_outputs.outputs)
                    .all(|(pattern, actual)| {
                        infer_overload_substitutions(
                            &pattern.ty,
                            &actual.ty,
                            parameters,
                            substitutions,
                        )
                    })
                && pattern_parameters
                    .iter()
                    .zip(actual_parameters)
                    .all(|(pattern, actual)| {
                        infer_overload_substitutions(pattern, actual, parameters, substitutions)
                    })
        }
        _ => scalar_type_equal(pattern, actual),
    }
}

#[derive(Clone, Debug)]
enum ScalarOverloadArgument {
    IntegerLiteral,
    Typed(ScalarType),
}

fn infer_overload_argument(
    pattern: &ScalarType,
    argument: &ScalarOverloadArgument,
    parameters: &[ScalarGenericParameter],
    substitutions: &mut BTreeMap<String, ScalarType>,
) -> bool {
    match argument {
        ScalarOverloadArgument::Typed(actual) => {
            infer_overload_substitutions(pattern, actual, parameters, substitutions)
        }
        ScalarOverloadArgument::IntegerLiteral => {
            matches!(pattern, ScalarType::Named { name, .. } if parameters.iter().any(|parameter| parameter.name == *name))
                || is_integer_type(pattern)
        }
    }
}

fn resolve_overload_candidate(
    overload: &ScalarFunction,
    arguments: &[ScalarOverloadArgument],
    expected: Option<&ScalarType>,
) -> Result<(ScalarType, ScalarOverloadSelection), &'static str> {
    let mut matches = Vec::new();
    let mut inconsistent = false;
    let mut output_mismatch = false;
    let mut unresolved_integer_literal = false;
    for (arm_index, arm) in overload.overload_arms.iter().enumerate() {
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &arm.signature
        else {
            continue;
        };
        if parameters.len() != arguments.len() {
            continue;
        }
        let mut substitutions = BTreeMap::new();
        let inputs_match = parameters.iter().zip(arguments).all(|(pattern, argument)| {
            infer_overload_argument(
                pattern,
                argument,
                &arm.generic_parameters,
                &mut substitutions,
            )
        });
        if !inputs_match {
            if !arm.generic_parameters.is_empty() {
                inconsistent = true;
            }
            continue;
        }
        if let Some(expected) = expected {
            let Some(output) = outputs.outputs.first() else {
                continue;
            };
            if !infer_overload_substitutions(
                &output.ty,
                expected,
                &arm.generic_parameters,
                &mut substitutions,
            ) {
                output_mismatch = true;
                continue;
            }
        }
        if arm
            .generic_parameters
            .iter()
            .any(|parameter| !substitutions.contains_key(&parameter.name))
        {
            unresolved_integer_literal |= arguments
                .iter()
                .any(|argument| matches!(argument, ScalarOverloadArgument::IntegerLiteral));
            continue;
        }
        let callable = substitute_type(&arm.signature, &substitutions);
        if let ScalarType::Callable { outputs, .. } = &callable {
            if expected.is_some_and(|expected| {
                outputs
                    .outputs
                    .first()
                    .is_some_and(|output| !scalar_type_equal(&output.ty, expected))
            }) {
                output_mismatch = true;
                continue;
            }
        }
        let ScalarType::Callable { parameters, .. } = &callable else {
            unreachable!("overload arms are callable")
        };
        if !parameters
            .iter()
            .zip(arguments)
            .all(|(parameter, argument)| {
                !matches!(argument, ScalarOverloadArgument::IntegerLiteral)
                    || is_integer_type(parameter)
            })
        {
            continue;
        }
        matches.push((
            callable,
            ScalarOverloadSelection {
                arm_index,
                substitutions,
            },
        ));
    }
    match matches.len() {
        1 => Ok(matches.pop().expect("one overload candidate")),
        0 if output_mismatch => Err("overload output does not match the expected receiver"),
        0 if inconsistent => Err("overload generic substitution is inconsistent"),
        0 if unresolved_integer_literal => {
            Err("integer literal requires a unique overload parameter or output context")
        }
        0 => Err("no overload candidate matches this call"),
        _ => Err("overload call is ambiguous"),
    }
}

fn overload_argument_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
) -> Option<ScalarOverloadArgument> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            scope.get(name).cloned().map(ScalarOverloadArgument::Typed)
        }
        ScalarExpression::Integer { .. } | ScalarExpression::InvalidInteger { .. } => {
            Some(ScalarOverloadArgument::IntegerLiteral)
        }
        ScalarExpression::Boolean { .. } => Some(ScalarOverloadArgument::Typed(ScalarType::Bool)),
        ScalarExpression::Char { .. } => Some(ScalarOverloadArgument::Typed(ScalarType::Char)),
        _ => None,
    }
}

fn overload_argument_from_actual(
    expression: &ScalarExpression,
    actual: &ScalarType,
) -> ScalarOverloadArgument {
    if matches!(
        expression,
        ScalarExpression::Integer { .. } | ScalarExpression::InvalidInteger { .. }
    ) {
        ScalarOverloadArgument::IntegerLiteral
    } else {
        ScalarOverloadArgument::Typed(actual.clone())
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
        enum_tag: Option<u32>,
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
    CheckedAddress {
        mutability: ScalarReferenceMutability,
        place: ScalarPlace,
        span: ByteSpan,
    },
    Dereference {
        place: ScalarPlace,
        span: ByteSpan,
    },
    IndexedRead {
        place: ScalarPlace,
        span: ByteSpan,
    },
    StructLiteral {
        fields: Vec<ScalarStructLiteralField>,
        span: ByteSpan,
    },
    ArrayLiteral {
        elements: Vec<ScalarExpression>,
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
        overload_selection: Option<ScalarOverloadSelection>,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOverloadSelection {
    pub arm_index: usize,
    pub substitutions: BTreeMap<String, ScalarType>,
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
    pub final_output_values: Vec<ScalarOutputValue>,
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
    pub values: Vec<ScalarExpression>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAssignmentTarget {
    pub receiver: Option<String>,
    pub target: String,
    pub receiver_span: Option<ByteSpan>,
    pub target_span: ByteSpan,
    pub span: ByteSpan,
    pub place: ScalarPlace,
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
    pub name_span: ByteSpan,
    pub declared_type: ScalarType,
    pub value: ScalarExpression,
    pub receivers: Vec<ScalarOutputReceiver>,
    pub output_sequence: ScalarOutputSequence,
    pub output_values: Vec<ScalarOutputValue>,
    pub output_origin: ScalarBindingOutputOrigin,
    pub is_allocation: bool,
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
    pub name_span: ByteSpan,
    pub signature: ScalarType,
    pub parameters: Vec<String>,
    pub parameter_spans: Vec<ByteSpan>,
    pub body: ScalarBlock,
    pub span: ByteSpan,
    pub generic_parameters: Vec<ScalarGenericParameter>,
    pub overload_arms: Vec<ScalarFunction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarGenericParameter {
    pub name: String,
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
    pub enums: Vec<ScalarEnum>,
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
    pub enums: Vec<ScalarEnum>,
    pub target_layout: ScalarTargetLayout,
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
                        if !is_module_private_name(&function.name) {
                            members.insert(function.name.clone(), function.signature.clone());
                        }
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
            if !is_module_private_name(name) {
                members.entry(name.clone()).or_insert_with(|| ty.clone());
            }
            initialization_nodes.push(ScalarInitializationNode { item_index, span });
        }
        Self {
            source,
            items,
            namespace_bindings,
            members,
            initialization_nodes,
            structs: Vec::new(),
            enums: Vec::new(),
            target_layout: ScalarTargetLayout::WASM32,
        }
    }

    pub fn from_program(
        program: ScalarProgram,
        namespace_bindings: Vec<ScalarNamespaceBinding>,
    ) -> Self {
        let mut module = Self::new(program.source, program.items, namespace_bindings);
        module.structs = program.structs;
        module.enums = program.enums;
        module.target_layout = program.target_layout;
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
    derive_scalar_program_with_layout(parse, ScalarTargetLayout::WASM32)
}

pub fn derive_scalar_program_with_layout(
    parse: &ParseResult,
    target_layout: ScalarTargetLayout,
) -> ScalarValidation {
    derive_scalar_program_from_cst_with_layout(parse.canonical_cst(), target_layout)
}

pub fn derive_scalar_program_from_cst(canonical: &CanonicalCstRoot) -> ScalarValidation {
    derive_scalar_program_from_cst_with_layout(canonical, ScalarTargetLayout::WASM32)
}

pub fn derive_scalar_program_from_cst_with_layout(
    canonical: &CanonicalCstRoot,
    target_layout: ScalarTargetLayout,
) -> ScalarValidation {
    let mut items = Vec::new();
    let mut diagnostics = string_diagnostics(canonical);
    diagnostics.extend(char_diagnostics(canonical));
    diagnostics.extend(integer_diagnostics(canonical));
    diagnostics.extend(float_diagnostics(canonical));
    diagnostics.extend(fixed_array_length_diagnostics(canonical));
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
                target_layout,
            )
        })
        .collect();
    let enums = source_root
        .children()
        .chain(source_root.descendants())
        .filter(|node| node.kind() == SyntaxKind::EnumDecl)
        .enumerate()
        .map(|(index, node)| {
            derive_enum(
                node,
                ScalarEnumId {
                    source: canonical.source.clone(),
                    index,
                },
            )
        })
        .collect();
    let mut generic_function = false;
    for node in source_root.children() {
        match node.kind() {
            SyntaxKind::NamespaceDecl => items.push(ScalarItem::Namespace(derive_namespace(&node))),
            SyntaxKind::ExternDecl => items.push(ScalarItem::Extern(derive_extern(&node))),
            SyntaxKind::BindingDecl => items.push(ScalarItem::Binding(derive_binding(&node))),
            SyntaxKind::FunctionDecl if !generic_function => {
                items.push(ScalarItem::Function(derive_function(&node)))
            }
            SyntaxKind::OverloadDecl => items.push(ScalarItem::Function(derive_overload(&node))),
            SyntaxKind::GenericFunctionDecl => {
                items.push(ScalarItem::Function(derive_generic_function(&node)));
                generic_function = true;
            }
            SyntaxKind::Item => {
                let item_children = node.children().collect::<Vec<_>>();
                let has_generic_function = item_children
                    .iter()
                    .any(|item| item.kind() == SyntaxKind::GenericFunctionDecl);
                for item in item_children {
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
                        SyntaxKind::FunctionDecl if !has_generic_function => {
                            items.push(ScalarItem::Function(derive_function(&item)))
                        }
                        SyntaxKind::OverloadDecl => {
                            items.push(ScalarItem::Function(derive_overload(&item)))
                        }
                        SyntaxKind::GenericFunctionDecl => {
                            items.push(ScalarItem::Function(derive_generic_function(&item)))
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
        if node.kind() != SyntaxKind::GenericFunctionDecl {
            generic_function = false;
        }
    }
    let mut program = ScalarProgram {
        source: canonical.source.clone(),
        items,
        structs,
        enums,
        target_layout,
    };
    resolve_program_types(&mut program, &mut diagnostics);
    resolve_program_places(&mut program);
    let mut validation_diagnostics = validate(&program);
    let mut diagnostics = diagnostics;
    diagnostics.append(&mut validation_diagnostics);
    if diagnostics.is_empty() {
        record_overload_selections(&mut program);
    }
    ScalarValidation {
        program,
        diagnostics,
    }
}

fn record_overload_selections(program: &mut ScalarProgram) {
    let scope = program
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
    let overloads = program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                Some((function.name.clone(), function.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for item in &mut program.items {
        if let ScalarItem::Binding(binding) = item {
            let expected = binding
                .receivers
                .first()
                .map(|receiver| &receiver.ty)
                .unwrap_or(&binding.declared_type);
            record_expression_overload_selection(
                &mut binding.value,
                Some(expected),
                &scope,
                &overloads,
            );
        }
    }
}

fn record_expression_overload_selection(
    expression: &mut ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    match expression {
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            overload_selection,
            ..
        } => {
            for argument in arguments.iter_mut() {
                record_expression_overload_selection(argument, None, scope, overloads);
            }
            if let (Some(overload), Some(argument_types)) = (
                overloads.get(
                    &receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}")),
                ),
                arguments
                    .iter()
                    .map(|argument| overload_argument_type(argument, scope))
                    .collect::<Option<Vec<_>>>(),
            ) {
                if let Ok((_, selection)) =
                    resolve_overload_candidate(overload, &argument_types, expected)
                {
                    *overload_selection = Some(selection);
                }
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            record_expression_overload_selection(left, None, scope, overloads);
            record_expression_overload_selection(right, None, scope, overloads);
        }
        ScalarExpression::Unary { operand, .. } => {
            record_expression_overload_selection(operand, expected, scope, overloads)
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            record_expression_overload_selection(
                condition,
                Some(&ScalarType::Bool),
                scope,
                overloads,
            );
            record_block_overload_selections(then_branch, scope, overloads);
            record_block_overload_selections(else_branch, scope, overloads);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            record_expression_overload_selection(
                condition,
                Some(&ScalarType::Bool),
                scope,
                overloads,
            );
            record_block_overload_selections(then_branch, scope, overloads);
        }
        ScalarExpression::Block(block) => record_block_overload_selections(block, scope, overloads),
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => match place {
            ScalarPlace::Dereference { pointer, .. } => {
                record_expression_overload_selection(pointer, None, scope, overloads)
            }
            ScalarPlace::Field { base, .. } => {
                if let ScalarPlace::Dereference { pointer, .. } = base.as_mut() {
                    record_expression_overload_selection(pointer, None, scope, overloads);
                }
            }
            ScalarPlace::Index { base, index, .. } => {
                record_place_overload_selections(base, scope, overloads);
                record_expression_overload_selection(
                    index,
                    Some(&ScalarType::U64),
                    scope,
                    overloads,
                );
            }
            ScalarPlace::Name { .. } => {}
        },
        ScalarExpression::IndexedRead { place, .. } => {
            record_place_overload_selections(place, scope, overloads)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                record_expression_overload_selection(&mut field.value, None, scope, overloads);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                record_expression_overload_selection(element, None, scope, overloads);
            }
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

fn record_place_overload_selections(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            record_expression_overload_selection(pointer, None, scope, overloads)
        }
        ScalarPlace::Field { base, .. } => record_place_overload_selections(base, scope, overloads),
        ScalarPlace::Index { base, index, .. } => {
            record_place_overload_selections(base, scope, overloads);
            record_expression_overload_selection(index, Some(&ScalarType::U64), scope, overloads);
        }
    }
}

fn record_block_overload_selections(
    block: &mut ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    let mut scope = scope.clone();
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                let expected = binding
                    .receivers
                    .first()
                    .map(|receiver| &receiver.ty)
                    .unwrap_or(&binding.declared_type);
                record_expression_overload_selection(
                    &mut binding.value,
                    Some(expected),
                    &scope,
                    overloads,
                );
                scope.insert(binding.name.clone(), binding.declared_type.clone());
                for receiver in &binding.receivers {
                    scope.insert(receiver.name.clone(), receiver.ty.clone());
                }
            }
            ScalarBlockItem::Expression(expression) => {
                record_expression_overload_selection(expression, None, &scope, overloads)
            }
            ScalarBlockItem::Assignment(assignment) => {
                record_expression_overload_selection(
                    &mut assignment.value,
                    scope.get(&assignment.target),
                    &scope,
                    overloads,
                );
                for (value, target) in assignment.values.iter_mut().zip(&assignment.targets) {
                    record_expression_overload_selection(
                        value,
                        scope.get(&target.target),
                        &scope,
                        overloads,
                    );
                }
            }
            ScalarBlockItem::While(while_expression) => {
                record_block_overload_selections(&mut while_expression.body, &scope, overloads)
            }
        }
    }
}

fn resolve_program_types(program: &mut ScalarProgram, diagnostics: &mut Vec<super::Diagnostic>) {
    let source = program.source.clone();
    let names: BTreeMap<String, ScalarType> = program
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(program.enums.iter().map(|enumeration| {
            (
                enumeration.name.clone(),
                ScalarType::Enum(enumeration.id.clone()),
            )
        }))
        .collect();
    for structure in &mut program.structs {
        for field in &mut structure.fields {
            field.ty = resolve_type(&field.ty, &names);
        }
        compute_initial_struct_layout(structure, program.target_layout);
    }
    resolve_program_struct_layouts(program, &source, diagnostics);
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
                if function.generic_parameters.is_empty() {
                    function.signature = resolve_type(&function.signature, &names);
                }
                for arm in &mut function.overload_arms {
                    if arm.generic_parameters.is_empty() {
                        arm.signature = resolve_type(&arm.signature, &names);
                    }
                }
                if let ScalarType::Callable { outputs, .. } = &function.signature {
                    for (value, output) in function
                        .body
                        .final_output_values
                        .iter_mut()
                        .zip(&outputs.outputs)
                    {
                        value.ty = output.ty.clone();
                    }
                }
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

fn resolve_item_types(item: &mut ScalarBlockItem, names: &BTreeMap<String, ScalarType>) {
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

fn resolve_block_types(block: &mut ScalarBlock, names: &BTreeMap<String, ScalarType>) {
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                binding.declared_type = resolve_type(&binding.declared_type, names);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type(&receiver.ty, names);
                }
                resolve_expression_types(&mut binding.value, names);
            }
            ScalarBlockItem::While(while_expression) => {
                resolve_expression_types(&mut while_expression.condition, names);
                resolve_block_types(&mut while_expression.body, names);
            }
            ScalarBlockItem::Expression(expression) => resolve_expression_types(expression, names),
            ScalarBlockItem::Assignment(assignment) => {
                resolve_expression_types(&mut assignment.value, names);
                for value in &mut assignment.values {
                    resolve_expression_types(value, names);
                }
            }
        }
    }
    for output in &mut block.final_output_values {
        resolve_expression_types(&mut output.value, names);
    }
}

fn resolve_expression_types(
    expression: &mut ScalarExpression,
    names: &BTreeMap<String, ScalarType>,
) {
    match expression {
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_types(condition, names);
            resolve_block_types(then_branch, names);
            resolve_block_types(else_branch, names);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_types(condition, names);
            resolve_block_types(then_branch, names);
        }
        ScalarExpression::Block(block) => resolve_block_types(block, names),
        _ => {}
    }
}

fn resolve_type(ty: &ScalarType, names: &BTreeMap<String, ScalarType>) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => names.get(name).cloned().map_or_else(
            || ScalarType::Named {
                name: name.clone(),
                span: *span,
            },
            |ty| ty,
        ),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(resolve_type(inner, names)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_type(inner, names)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_type(element, names)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
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
            for target in &mut assignment.targets {
                resolve_place(&mut target.place, scope, program);
            }
            resolve_expression_places(&mut assignment.value, scope, program);
            for value in &mut assignment.values {
                resolve_expression_places(value, scope, program);
            }
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
    for output in &mut block.final_output_values {
        resolve_expression_places(&mut output.value, scope, program);
    }
}

fn resolve_expression_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) {
    match expression {
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => resolve_place(place, scope, program),
        ScalarExpression::IndexedRead { place, .. } => resolve_place(place, scope, program),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_places(&mut field.value, scope, program);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                resolve_expression_places(element, scope, program);
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
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            *enum_tag = program
                .enums
                .iter()
                .find(|enumeration| enumeration.name == *receiver)
                .and_then(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .find(|variant| variant.name == *name)
                })
                .map(|variant| variant.tag);
        }
        ScalarExpression::Name { .. }
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
            let base_type = match base.as_ref() {
                ScalarPlace::Dereference { pointer, .. } => {
                    match expression_type(
                        pointer,
                        scope,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        program,
                        &mut Vec::new(),
                        false,
                    ) {
                        ScalarType::RawPointer(inner)
                        | ScalarType::CheckedReference { inner, .. } => *inner,
                        _ => ScalarType::Error,
                    }
                }
                ScalarPlace::Name { .. }
                | ScalarPlace::Field { .. }
                | ScalarPlace::Index { .. } => place_type(base, scope, program, &mut Vec::new()),
            };
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
        ScalarPlace::Index { base, index, .. } => {
            resolve_place(base, scope, program);
            resolve_expression_places(index, scope, program);
        }
    }
}

pub fn derive_scalar_diagnostics_from_cst(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    let mut diagnostics = string_diagnostics(canonical);
    diagnostics.extend(char_diagnostics(canonical));
    diagnostics.extend(integer_diagnostics(canonical));
    diagnostics.extend(float_diagnostics(canonical));
    diagnostics.extend(fixed_array_length_diagnostics(canonical));
    diagnostics
}

pub fn validate_scalar_project(project: ScalarProject) -> ScalarProjectValidation {
    let mut project = project;
    let mut diagnostics = Vec::new();
    let struct_lookup = project.modules.clone();
    for module in &mut project.modules {
        resolve_module_types_in_project(module, &struct_lookup);
    }
    resolve_project_struct_layouts(&mut project, &mut diagnostics);
    let struct_lookup = project.modules.clone();
    for module in &mut project.modules {
        resolve_module_places(module, &struct_lookup);
    }
    for module in &project.modules {
        validate_unit_if_positions_in_module(module, &mut diagnostics);
        let mut declarations = BTreeMap::new();
        let mut declaration_names = BTreeSet::new();
        let mut folded_declarations = BTreeMap::new();
        for item in &module.items {
            let (name, span, ty) = match item {
                ScalarItem::Namespace(namespace) => {
                    validate_identifier_style(
                        module,
                        &namespace.binding,
                        namespace.binding_span,
                        &mut diagnostics,
                    );
                    (&namespace.binding, namespace.span, None)
                }
                ScalarItem::Extern(extern_decl) => {
                    validate_identifier_style(
                        module,
                        &extern_decl.binding,
                        extern_decl.binding_span,
                        &mut diagnostics,
                    );
                    (&extern_decl.binding, extern_decl.span, None)
                }
                ScalarItem::Binding(binding) => {
                    for receiver in &binding.receivers {
                        validate_identifier_style(
                            module,
                            &receiver.name,
                            receiver.name_span,
                            &mut diagnostics,
                        );
                    }
                    (&binding.name, binding.span, Some(&binding.declared_type))
                }
                ScalarItem::Function(function) => {
                    validate_identifier_style(
                        module,
                        &function.name,
                        function.name_span,
                        &mut diagnostics,
                    );
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
                    if let ScalarItem::Function(function) = item {
                        if !is_module_private_name(&function.name)
                            && !function.overload_arms.is_empty()
                        {
                            for arm in &function.overload_arms {
                                if arm.generic_parameters.is_empty() {
                                    validate_module_type(
                                        module,
                                        &arm.signature,
                                        arm.span,
                                        &mut diagnostics,
                                    );
                                } else {
                                    validate_module_generic_type(
                                        module,
                                        &arm.signature,
                                        arm.span,
                                        &arm.generic_parameters,
                                        &mut diagnostics,
                                    );
                                }
                            }
                        } else if function.generic_parameters.is_empty() {
                            validate_module_type(module, ty, span, &mut diagnostics);
                        } else {
                            validate_module_generic_type(
                                module,
                                ty,
                                span,
                                &function.generic_parameters,
                                &mut diagnostics,
                            );
                        }
                    } else {
                        validate_module_type(module, ty, span, &mut diagnostics);
                    }
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
                    if binding.is_allocation {
                        for receiver in &binding.receivers {
                            let Some(length) = &receiver.allocation_length else {
                                continue;
                            };
                            let actual = expression_type_in_module_expected(
                                length,
                                Some(&ScalarType::U64),
                                &declarations,
                                &BTreeSet::new(),
                                &BTreeMap::new(),
                                module,
                                &project.modules,
                                &mut diagnostics,
                                false,
                            );
                            expect_module_type(
                                module,
                                &ScalarType::U64,
                                &actual,
                                receiver.span,
                                &mut diagnostics,
                            );
                        }
                        continue;
                    }
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
                    if !function.overload_arms.is_empty() {
                        continue;
                    }
                    let ScalarType::Callable {
                        outputs,
                        parameters,
                    } = &function.signature
                    else {
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
                        validate_identifier_style(
                            module,
                            name,
                            function.parameter_spans[index],
                            &mut diagnostics,
                        );
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
                    if outputs.outputs.len() > 1 && !function.body.final_output_values.is_empty() {
                        let final_start =
                            function.body.items.len() - function.body.final_output_values.len();
                        for item in &function.body.items[..final_start] {
                            let _ = match item {
                                ScalarBlockItem::Expression(
                                    expression @ (ScalarExpression::If { .. }
                                    | ScalarExpression::UnitIf { .. }),
                                ) => expression_type_in_module_expected(
                                    expression,
                                    Some(&ScalarType::Unit),
                                    &scope,
                                    &visible_names,
                                    &folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                ),
                                _ => block_item_type_in_module(
                                    item,
                                    &mut scope,
                                    &mut visible_names,
                                    &mut folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                ),
                            };
                        }
                        if function.body.final_output_values.len() == 1
                            && matches!(
                                &function.body.final_output_values[0].value,
                                ScalarExpression::Call { .. }
                            )
                        {
                            let value = &function.body.final_output_values[0];
                            if let Some(actual_outputs) = call_output_sequence_in_module(
                                &value.value,
                                &scope,
                                module,
                                &project.modules,
                            ) {
                                expression_type_in_module_expected(
                                    &value.value,
                                    actual_outputs.outputs.first().map(|output| &output.ty),
                                    &scope,
                                    &visible_names,
                                    &folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                );
                                if actual_outputs.outputs.len() != outputs.outputs.len() {
                                    diagnostics.push(module_diagnostic(
                                        module,
                                        "B0004",
                                        "function output arity does not match its final output list",
                                        value.span,
                                    ));
                                }
                                for (actual, expected) in
                                    actual_outputs.outputs.iter().zip(&outputs.outputs)
                                {
                                    expect_module_type(
                                        module,
                                        &expected.ty,
                                        &actual.ty,
                                        value.span,
                                        &mut diagnostics,
                                    );
                                }
                            }
                        } else {
                            if function.body.final_output_values.len() > 1
                                && function.body.final_output_values.len() != outputs.outputs.len()
                            {
                                diagnostics.push(module_diagnostic(
                                    module,
                                    "B0004",
                                    "function output arity does not match its final output list",
                                    function.body.span,
                                ));
                            }
                            for (value, output) in function
                                .body
                                .final_output_values
                                .iter()
                                .zip(&outputs.outputs)
                            {
                                let actual = expression_type_in_module_expected(
                                    &value.value,
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
                                    value.span,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else if let Some(output) = outputs.outputs.first() {
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
                    } else {
                        let actual = block_type_in_module(
                            &function.body,
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
                            &ScalarType::Unit,
                            &actual,
                            function.body.span,
                            &mut diagnostics,
                        );
                    }
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
    if diagnostics.is_empty() {
        record_project_overload_selections(&mut project);
    }
    ScalarProjectValidation {
        project,
        diagnostics,
    }
}

fn record_project_overload_selections(project: &mut ScalarProject) {
    let modules = project.modules.clone();
    for module in &mut project.modules {
        let scope = module.members.clone();
        let mut overloads = module
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                    Some((function.name.clone(), function.clone()))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        for namespace in &module.namespace_bindings {
            if let Some(target) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            {
                for item in &target.items {
                    if let ScalarItem::Function(function) = item {
                        if target.members.contains_key(&function.name)
                            && !function.overload_arms.is_empty()
                        {
                            overloads.insert(
                                format!("{}.{}", namespace.binding, function.name),
                                function.clone(),
                            );
                        }
                    }
                }
            }
        }
        for item in &mut module.items {
            if let ScalarItem::Binding(binding) = item {
                let expected = binding
                    .receivers
                    .first()
                    .map(|receiver| &receiver.ty)
                    .unwrap_or(&binding.declared_type);
                record_expression_overload_selection(
                    &mut binding.value,
                    Some(expected),
                    &scope,
                    &overloads,
                );
            }
        }
    }
}

fn resolve_module_types_in_project(module: &mut ScalarModule, modules: &[ScalarModule]) {
    let context = module.clone();
    let names = module
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(module.enums.iter().map(|enumeration| {
            (
                enumeration.name.clone(),
                ScalarType::Enum(enumeration.id.clone()),
            )
        }))
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
                if function.generic_parameters.is_empty() {
                    function.signature =
                        resolve_type_in_project(&function.signature, &names, &context, modules);
                }
                for arm in &mut function.overload_arms {
                    if arm.generic_parameters.is_empty() {
                        arm.signature =
                            resolve_type_in_project(&arm.signature, &names, &context, modules);
                    }
                }
                if let ScalarType::Callable { outputs, .. } = &function.signature {
                    for (value, output) in function
                        .body
                        .final_output_values
                        .iter_mut()
                        .zip(&outputs.outputs)
                    {
                        value.ty = output.ty.clone();
                    }
                }
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
            ScalarItem::Binding(binding) if !is_module_private_name(&binding.name) => {
                module
                    .members
                    .insert(binding.name.clone(), binding.declared_type.clone());
            }
            ScalarItem::Function(function) if !is_module_private_name(&function.name) => {
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
    names: &BTreeMap<String, ScalarType>,
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
    names: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    for item in &mut block.items {
        resolve_item_types_project(item, names, module, modules);
    }
}

fn resolve_type_in_project(
    ty: &ScalarType,
    names: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => {
            if let Some(id) = names.get(name) {
                return id.clone();
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
                target.structs.iter().find(|structure| {
                    structure.name == *member && !is_module_private_name(&structure.name)
                })
            }) {
            Some(structure) => ScalarType::Struct(structure.id.clone()),
            None => module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
                .and_then(|namespace| {
                    modules
                        .iter()
                        .find(|candidate| candidate.source == namespace.target)
                })
                .and_then(|target| {
                    target.enums.iter().find(|enumeration| {
                        enumeration.name == *member && !is_module_private_name(&enumeration.name)
                    })
                })
                .map_or_else(
                    || ScalarType::Qualified {
                        receiver: receiver.clone(),
                        receiver_span: *receiver_span,
                        member: member.clone(),
                        member_span: *member_span,
                        span: *span,
                    },
                    |enumeration| ScalarType::Enum(enumeration.id.clone()),
                ),
        },
        ScalarType::RawPointer(inner) => ScalarType::RawPointer(Box::new(resolve_type_in_project(
            inner, names, module, modules,
        ))),
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_type_in_project(inner, names, module, modules)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_type_in_project(element, names, module, modules)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
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

fn resolve_program_struct_layouts(
    program: &mut ScalarProgram,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (cycles, order) = aggregate_layout_order(&program.structs);
    for cycle in cycles {
        let (structure_index, field_index) = cycle_participant(&program.structs, &cycle);
        let field = &program.structs[structure_index].fields[field_index];
        recursive_layout_diagnostic(source, field.name_span, diagnostics);
        for index in cycle {
            clear_struct_layout(&mut program.structs[index]);
        }
    }
    for index in order {
        let structs = program.structs.clone();
        recompute_struct_layout(
            &mut program.structs[index],
            &structs,
            program.target_layout,
            source,
            diagnostics,
        );
    }
}

fn resolve_project_struct_layouts(
    project: &mut ScalarProject,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let locations = project
        .modules
        .iter()
        .enumerate()
        .flat_map(|(module_index, module)| {
            module
                .structs
                .iter()
                .enumerate()
                .map(move |(structure_index, structure)| {
                    (module_index, structure_index, structure.clone())
                })
        })
        .collect::<Vec<_>>();
    let structures = locations
        .iter()
        .map(|(_, _, structure)| structure.clone())
        .collect::<Vec<_>>();
    let (cycles, order) = aggregate_layout_order(&structures);
    for cycle in cycles {
        let (participant, field_index) = cycle_participant(&structures, &cycle);
        let (module_index, structure_index, _) = locations[participant];
        let field = &project.modules[module_index].structs[structure_index].fields[field_index];
        let source = project.modules[module_index].source.clone();
        recursive_layout_diagnostic(&source, field.name_span, diagnostics);
        for index in cycle {
            let (module_index, structure_index, _) = locations[index];
            clear_struct_layout(&mut project.modules[module_index].structs[structure_index]);
        }
    }
    for index in order {
        let (module_index, structure_index, _) = locations[index];
        let modules = project.modules.clone();
        let target_layout = project.modules[module_index].target_layout;
        let source = project.modules[module_index].source.clone();
        recompute_struct_layout_project(
            &mut project.modules[module_index].structs[structure_index],
            &modules,
            target_layout,
            &source,
            diagnostics,
        );
    }
}

fn aggregate_layout_order(structures: &[ScalarStruct]) -> (Vec<Vec<usize>>, Vec<usize>) {
    let indices = structures
        .iter()
        .enumerate()
        .map(|(index, structure)| (structure.id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let dependencies = structures
        .iter()
        .map(|structure| {
            let mut ids = Vec::new();
            for field in &structure.fields {
                by_value_struct_dependencies(&field.ty, &mut ids);
            }
            ids.into_iter()
                .filter_map(|id| indices.get(&id).copied())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let components = aggregate_layout_components(&dependencies);
    let mut cyclic = vec![false; structures.len()];
    let mut cycles = Vec::new();
    for component in components {
        let self_edge = component.len() == 1 && dependencies[component[0]].contains(&component[0]);
        if component.len() > 1 || self_edge {
            for &index in &component {
                cyclic[index] = true;
            }
            cycles.push(component);
        }
    }
    cycles.sort_unstable_by_key(|cycle| cycle_participant(structures, cycle));
    let mut visited = vec![false; structures.len()];
    let mut order = Vec::new();
    for index in 0..structures.len() {
        aggregate_layout_visit(index, &dependencies, &cyclic, &mut visited, &mut order);
    }
    (cycles, order)
}

fn by_value_struct_dependencies(ty: &ScalarType, dependencies: &mut Vec<ScalarStructId>) {
    match ty {
        ScalarType::Struct(id) => dependencies.push(id.clone()),
        ScalarType::Array { element, .. } => by_value_struct_dependencies(element, dependencies),
        _ => {}
    }
}

fn aggregate_layout_components(dependencies: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut index = 0;
    let mut indices = vec![None; dependencies.len()];
    let mut lowlinks = vec![0; dependencies.len()];
    let mut stack = Vec::new();
    let mut on_stack = vec![false; dependencies.len()];
    let mut components = Vec::new();
    for node in 0..dependencies.len() {
        if indices[node].is_none() {
            aggregate_layout_component_visit(
                node,
                dependencies,
                &mut index,
                &mut indices,
                &mut lowlinks,
                &mut stack,
                &mut on_stack,
                &mut components,
            );
        }
    }
    components
}

fn aggregate_layout_component_visit(
    node: usize,
    dependencies: &[Vec<usize>],
    index: &mut usize,
    indices: &mut [Option<usize>],
    lowlinks: &mut [usize],
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    components: &mut Vec<Vec<usize>>,
) {
    indices[node] = Some(*index);
    lowlinks[node] = *index;
    *index += 1;
    stack.push(node);
    on_stack[node] = true;
    for &dependency in &dependencies[node] {
        if indices[dependency].is_none() {
            aggregate_layout_component_visit(
                dependency,
                dependencies,
                index,
                indices,
                lowlinks,
                stack,
                on_stack,
                components,
            );
            lowlinks[node] = lowlinks[node].min(lowlinks[dependency]);
        } else if on_stack[dependency] {
            lowlinks[node] = lowlinks[node].min(indices[dependency].expect("component index"));
        }
    }
    if lowlinks[node] == indices[node].expect("component index") {
        let mut component = Vec::new();
        loop {
            let member = stack.pop().expect("component member");
            on_stack[member] = false;
            component.push(member);
            if member == node {
                break;
            }
        }
        component.sort_unstable();
        components.push(component);
    }
}

fn aggregate_layout_visit(
    node: usize,
    dependencies: &[Vec<usize>],
    cyclic: &[bool],
    visited: &mut [bool],
    order: &mut Vec<usize>,
) {
    if visited[node] || cyclic[node] {
        return;
    }
    visited[node] = true;
    for &dependency in &dependencies[node] {
        aggregate_layout_visit(dependency, dependencies, cyclic, visited, order);
    }
    order.push(node);
}

fn cycle_participant(structures: &[ScalarStruct], cycle: &[usize]) -> (usize, usize) {
    for &structure_index in cycle {
        for (field_index, field) in structures[structure_index].fields.iter().enumerate() {
            let mut dependencies = Vec::new();
            by_value_struct_dependencies(&field.ty, &mut dependencies);
            if dependencies.iter().any(|dependency| {
                cycle
                    .iter()
                    .any(|&candidate| structures[candidate].id == *dependency)
            }) {
                return (structure_index, field_index);
            }
        }
    }
    panic!("recursive aggregate component has a participating field")
}

fn recursive_layout_diagnostic(
    source: &SourceIdentity,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    diagnostics.push(super::Diagnostic {
        code: "B0003".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: "recursive by-value struct layout is unsupported".to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span),
            message: "field participates in a recursive struct layout".to_owned(),
        }],
        notes: Vec::new(),
    });
}

fn recompute_struct_layout(
    structure: &mut ScalarStruct,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let mut offset = 0;
    let mut alignment = 1;
    clear_struct_layout(structure);
    for field in &mut structure.fields {
        let layout = match layout_for_type_in_structs(&field.ty, structs, target_layout) {
            Ok(layout) => layout,
            Err(error) => {
                layout_error_source(source, error, diagnostics);
                return;
            }
        };
        alignment = alignment.max(layout.alignment);
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            layout_error_source(
                source,
                LayoutError::AggregateSizeOverflow(field.name_span),
                diagnostics,
            );
            clear_struct_layout(structure);
            return;
        };
        field.offset = Some(field_offset);
        field.layout = Some(layout.clone());
        offset = next_offset;
    }
    let Some(size) = checked_align_offset(offset, alignment) else {
        layout_error_source(
            source,
            LayoutError::AggregateSizeOverflow(structure.name_span),
            diagnostics,
        );
        clear_struct_layout(structure);
        return;
    };
    structure.layout = Some(ScalarLayout { size, alignment });
}

fn recompute_struct_layout_project(
    structure: &mut ScalarStruct,
    modules: &[ScalarModule],
    target_layout: ScalarTargetLayout,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let mut offset = 0;
    let mut alignment = 1;
    clear_struct_layout(structure);
    for field in &mut structure.fields {
        let layout = match layout_for_type_in_modules(&field.ty, modules, target_layout) {
            Ok(layout) => layout,
            Err(error) => {
                layout_error_source(source, error, diagnostics);
                return;
            }
        };
        alignment = alignment.max(layout.alignment);
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            layout_error_source(
                source,
                LayoutError::AggregateSizeOverflow(field.name_span),
                diagnostics,
            );
            clear_struct_layout(structure);
            return;
        };
        field.offset = Some(field_offset);
        field.layout = Some(layout.clone());
        offset = next_offset;
    }
    let Some(size) = checked_align_offset(offset, alignment) else {
        layout_error_source(
            source,
            LayoutError::AggregateSizeOverflow(structure.name_span),
            diagnostics,
        );
        clear_struct_layout(structure);
        return;
    };
    structure.layout = Some(ScalarLayout { size, alignment });
}

fn layout_for_type_in_structs(
    ty: &ScalarType,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    match ty {
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type_in_structs(element, structs, target_layout)?;
            Ok(ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            })
        }
        ScalarType::Struct(id) => structs
            .iter()
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.layout.clone())
            .ok_or(LayoutError::InvalidType),
        _ => layout_for_type(ty, target_layout),
    }
}

fn layout_for_type_in_modules(
    ty: &ScalarType,
    modules: &[ScalarModule],
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    match ty {
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type_in_modules(element, modules, target_layout)?;
            Ok(ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            })
        }
        ScalarType::Struct(id) => modules
            .iter()
            .flat_map(|module| module.structs.iter())
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.layout.clone())
            .ok_or(LayoutError::InvalidType),
        _ => layout_for_type(ty, target_layout),
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
            for target in &mut assignment.targets {
                resolve_module_place(&mut target.place, scope, module, modules);
            }
            resolve_expression_module_places(&mut assignment.value, scope, module, modules);
            for value in &mut assignment.values {
                resolve_expression_module_places(value, scope, module, modules);
            }
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
    for output in &mut block.final_output_values {
        resolve_expression_module_places(&mut output.value, scope, module, modules);
    }
}

fn resolve_expression_module_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    match expression {
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => {
            resolve_module_place(place, scope, module, modules)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            resolve_module_place(place, scope, module, modules)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_module_places(&mut field.value, scope, module, modules);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                resolve_expression_module_places(element, scope, module, modules);
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
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            *enum_tag = enum_member_in_module(module, modules, receiver, name).map(|(_, tag)| tag);
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

fn enum_member_in_module(
    module: &ScalarModule,
    modules: &[ScalarModule],
    receiver: &str,
    name: &str,
) -> Option<(ScalarEnumId, u32)> {
    let enumeration = module
        .enums
        .iter()
        .find(|enumeration| enumeration.name == receiver)
        .or_else(|| {
            receiver.split_once('.').and_then(|(namespace, enum_name)| {
                module
                    .namespace_bindings
                    .iter()
                    .find(|binding| binding.binding == namespace)
                    .and_then(|binding| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == binding.target)
                    })
                    .and_then(|target| {
                        target.enums.iter().find(|enumeration| {
                            enumeration.name == enum_name
                                && !is_module_private_name(&enumeration.name)
                        })
                    })
            })
        })?;
    enumeration
        .variants
        .iter()
        .find(|variant| variant.name == name)
        .map(|variant| (enumeration.id.clone(), variant.tag))
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
            let base_type = match base.as_ref() {
                ScalarPlace::Dereference { pointer, .. } => match expression_type_in_module(
                    pointer,
                    scope,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    module,
                    modules,
                    &mut Vec::new(),
                    false,
                ) {
                    ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                        *inner
                    }
                    _ => ScalarType::Error,
                },
                ScalarPlace::Name { .. }
                | ScalarPlace::Field { .. }
                | ScalarPlace::Index { .. } => {
                    place_type_in_module(base, scope, module, modules, &mut Vec::new())
                }
            };
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
        ScalarPlace::Index { base, index, .. } => {
            resolve_module_place(base, scope, module, modules);
            resolve_expression_module_places(index, scope, module, modules);
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
        ScalarType::CheckedReference { inner, .. } => {
            validate_module_type(module, inner, span, diagnostics)
        }
        ScalarType::RawPointer(inner)
            if matches!(inner.as_ref(), ScalarType::U8 | ScalarType::Struct(_)) => {}
        ScalarType::RawPointer(_) => diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "unsupported raw pointer type",
            span,
        )),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
            validate_module_type(module, element, span, diagnostics)
        }
        ScalarType::Struct(_) | ScalarType::Enum(_) => {}
        ScalarType::Error => {}
    }
}

fn validate_module_generic_type(
    module: &ScalarModule,
    ty: &ScalarType,
    span: ByteSpan,
    generic_parameters: &[ScalarGenericParameter],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { name, .. }
            if generic_parameters
                .iter()
                .any(|parameter| parameter.name == *name) => {}
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            validate_module_generic_type(module, inner, span, generic_parameters, diagnostics)
        }
        ScalarType::Array { element, .. } => {
            validate_module_generic_type(module, element, span, generic_parameters, diagnostics)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_module_generic_type(
                    module,
                    &output.ty,
                    output.span,
                    generic_parameters,
                    diagnostics,
                );
            }
            for parameter in parameters {
                validate_module_generic_type(
                    module,
                    parameter,
                    span,
                    generic_parameters,
                    diagnostics,
                );
            }
        }
        _ => validate_module_type(module, ty, span, diagnostics),
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
        validate_identifier_style(module, &function.name, function.name_span, diagnostics);
        validate_module_type(
            module,
            &function.signature,
            function.signature_span,
            diagnostics,
        );
        if contains_fixed_array(&function.signature) {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "direct FFI arrays are not supported",
                function.signature_span,
            ));
        }
        if contains_checked_reference(&function.signature) {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "direct FFI checked references are not supported",
                function.signature_span,
            ));
        }
    }
}

fn contains_checked_reference(ty: &ScalarType) -> bool {
    match ty {
        ScalarType::CheckedReference { .. } => true,
        ScalarType::RawPointer(inner) | ScalarType::Array { element: inner, .. } => {
            contains_checked_reference(inner)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            outputs
                .outputs
                .iter()
                .any(|output| contains_checked_reference(&output.ty))
                || parameters.iter().any(contains_checked_reference)
        }
        _ => false,
    }
}

fn contains_fixed_array(ty: &ScalarType) -> bool {
    match ty {
        ScalarType::Array { .. } => true,
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            contains_fixed_array(inner)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            outputs
                .outputs
                .iter()
                .any(|output| contains_fixed_array(&output.ty))
                || parameters.iter().any(contains_fixed_array)
        }
        _ => false,
    }
}

fn call_output_sequence_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) -> Option<ScalarOutputSequence> {
    let ScalarExpression::Call {
        receiver,
        name,
        type_arguments,
        arguments,
        ..
    } = expression
    else {
        return None;
    };
    let (callable, target, member_visible) = match receiver {
        None => (scope.get(name), module, true),
        Some(binding) => {
            let namespace = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding);
            if let Some(namespace) = namespace {
                let target = modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)?;
                (
                    target.members.get(name),
                    target,
                    target.members.contains_key(name),
                )
            } else {
                (
                    module.items.iter().find_map(|item| match item {
                        ScalarItem::Extern(extern_decl) if extern_decl.binding == *binding => {
                            extern_decl
                                .functions
                                .iter()
                                .find(|function| function.name == *name)
                                .map(|function| &function.signature)
                        }
                        _ => None,
                    }),
                    module,
                    true,
                )
            }
        }
    };
    if member_visible {
        if let Some(overload) = target.items.iter().find_map(|item| match item {
            ScalarItem::Function(function)
                if function.name == *name && !function.overload_arms.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        }) {
            let argument_types = arguments
                .iter()
                .map(|argument| overload_argument_type(argument, scope))
                .collect::<Option<Vec<_>>>()?;
            let ScalarType::Callable { outputs, .. } =
                resolve_overload_candidate(overload, &argument_types, None)
                    .ok()?
                    .0
            else {
                return None;
            };
            return Some(outputs);
        }
    }
    let callable = callable?;
    let extern_callable = match receiver {
        None => false,
        Some(binding) => {
            if let Some(namespace) = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding)
            {
                modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)
                    .is_some_and(|target| {
                        target.items.iter().any(|item| {
                            matches!(
                                item,
                                ScalarItem::Extern(extern_decl)
                                    if extern_decl.functions.iter().any(|function| function.name == *name)
                            )
                        })
                    })
            } else {
                module.items.iter().any(|item| {
                    matches!(
                        item,
                        ScalarItem::Extern(extern_decl)
                            if extern_decl.binding == *binding
                                && extern_decl.functions.iter().any(|function| function.name == *name)
                    )
                })
            }
        }
    };
    if extern_callable && !type_arguments.is_empty() {
        return None;
    }
    let generic_parameters = if member_visible {
        target.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == *name => {
                Some(&function.generic_parameters)
            }
            _ => None,
        })
    } else {
        None
    };
    let callable = if let Some(parameters) = generic_parameters {
        substitute_generic_callable(callable, parameters, type_arguments)?
    } else {
        callable.clone()
    };
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs)
}

fn call_output_sequence(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) -> Option<ScalarOutputSequence> {
    let ScalarExpression::Call {
        receiver,
        name,
        type_arguments,
        arguments,
        ..
    } = expression
    else {
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
    let extern_callable = program.items.iter().any(|item| {
        matches!(
            item,
            ScalarItem::Extern(extern_decl)
                if receiver.as_deref() == Some(extern_decl.binding.as_str())
                    && extern_decl.functions.iter().any(|function| function.name == *name)
        )
    });
    if extern_callable && !type_arguments.is_empty() {
        return None;
    }
    if let Some(overload) = receiver
        .is_none()
        .then(|| {
            program.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if function.name == *name && !function.overload_arms.is_empty() =>
                {
                    Some(function)
                }
                _ => None,
            })
        })
        .flatten()
    {
        let argument_types = arguments
            .iter()
            .map(|argument| overload_argument_type(argument, scope))
            .collect::<Option<Vec<_>>>()?;
        let ScalarType::Callable { outputs, .. } =
            resolve_overload_candidate(overload, &argument_types, None)
                .ok()?
                .0
        else {
            return None;
        };
        return Some(outputs);
    }
    let generic_parameters = receiver.is_none().then(|| {
        program.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == *name => {
                Some(&function.generic_parameters)
            }
            _ => None,
        })
    });
    let callable = if let Some(Some(parameters)) = generic_parameters {
        substitute_generic_callable(callable, parameters, type_arguments)?
    } else {
        callable.clone()
    };
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs)
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
        | ScalarExpression::CheckedAddress { span, .. }
        | ScalarExpression::Dereference { span, .. }
        | ScalarExpression::IndexedRead { span, .. }
        | ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::ArrayLiteral { span, .. }
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
    if receivers.len() > outputs.outputs.len() {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "call has fewer outputs than receivers",
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
    if receivers.len() > outputs.outputs.len() {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "call has fewer outputs than receivers",
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
            for receiver in &binding.receivers {
                validate_identifier_style(module, &receiver.name, receiver.name_span, diagnostics);
            }
            if binding.is_allocation {
                for receiver in &binding.receivers {
                    let Some(length) = &receiver.allocation_length else {
                        continue;
                    };
                    let actual = expression_type_in_module_expected(
                        length,
                        Some(&ScalarType::U64),
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
                        &ScalarType::U64,
                        &actual,
                        receiver.span,
                        diagnostics,
                    );
                }
                if declare_module_name(
                    module,
                    &binding.name,
                    binding.span,
                    visible_names,
                    folded_names,
                    diagnostics,
                ) {
                    for receiver in &binding.receivers {
                        scope.insert(receiver.name.clone(), receiver.ty.clone());
                    }
                }
                return ScalarType::Unit;
            }
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
        ScalarBlockItem::Expression(expression) => expression_type_in_module(
            expression,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
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
    let mut inferred_names = visible_names.clone();
    let mut inferred_folded_names = folded_names.clone();
    let expected = assignment
        .targets
        .iter()
        .map(|target| assignment_target_type_in_module(target, scope, module, modules, diagnostics))
        .collect::<Vec<_>>();
    validate_assignment_target_distinctness_in_module(assignment, module, diagnostics);
    let outputs = assignment
        .values
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let actual = expression_type_in_module_expected(
                value,
                expected.get(position).and_then(Option::as_ref),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            match call_output_sequence_in_module(value, scope, module, modules) {
                Some(outputs) => Some(outputs.outputs),
                None if matches!(value, ScalarExpression::Call { .. }) => None,
                None => Some(vec![ScalarOutput {
                    ty: actual,
                    span: expression_span(value),
                }]),
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|outputs| ScalarOutputSequence {
            outputs: outputs.into_iter().flatten().collect(),
            span: assignment.span,
        });
    if let Some(outputs) = outputs {
        if assignment.targets.len() > outputs.outputs.len() {
            diagnostics.push(module_diagnostic(
                module,
                "B0004",
                "call has fewer outputs than assignment targets",
                assignment.span,
            ));
        }
        for ((index, (target, expected)), output) in assignment
            .targets
            .iter()
            .zip(&expected)
            .enumerate()
            .zip(&outputs.outputs)
        {
            if let Some(expected) = expected {
                expect_module_type(
                    module,
                    expected,
                    &output.ty,
                    if index == 0 {
                        assignment.span
                    } else {
                        target.target_span
                    },
                    diagnostics,
                );
            } else if let ScalarPlace::Name { name, span } = &target.place {
                validate_identifier_style(module, name, *span, diagnostics);
                if declare_module_name(
                    module,
                    name,
                    *span,
                    &mut inferred_names,
                    &mut inferred_folded_names,
                    diagnostics,
                ) {
                    scope.insert(name.clone(), output.ty.clone());
                }
            }
        }
    }
    ScalarType::Unit
}

fn assignment_target_type_in_module(
    target: &ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> Option<ScalarType> {
    if let Some(receiver) = &target.receiver {
        if let Some(namespace) = module
            .namespace_bindings
            .iter()
            .find(|namespace| namespace.binding == *receiver)
        {
            let Some(target_module) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown assignment target",
                    target
                        .receiver_span
                        .expect("qualified assignment receiver span"),
                ));
                return Some(ScalarType::Error);
            };
            let Some(expected) = target_module.members.get(&target.target).cloned() else {
                diagnostics.push(unknown_member_diagnostic(
                    module,
                    target.target_span,
                    &target_module.source,
                ));
                return Some(ScalarType::Error);
            };
            if is_const_binding_name(&target.target) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    target.target_span,
                ));
            }
            return Some(expected);
        }
    }
    match &target.place {
        ScalarPlace::Name { name, span } => {
            let expected = scope.get(name).cloned();
            if expected.is_some() && is_const_binding_name(name) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    *span,
                ));
            }
            expected
        }
        place => Some(writable_place_type_in_module(
            place,
            target.span,
            scope,
            module,
            modules,
            diagnostics,
        )),
    }
}

fn validate_assignment_target_distinctness_in_module(
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (index, target) in assignment.targets.iter().enumerate() {
        for previous in &assignment.targets[..index] {
            if scalar_place_identity(&target.place) == scalar_place_identity(&previous.place) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "duplicate assignment target",
                    target.target_span,
                ));
            } else if place_contains_dynamic_index(&target.place)
                || place_contains_dynamic_index(&previous.place)
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "assignment targets require a distinctness proof",
                    target.target_span,
                ));
            }
        }
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

fn type_core_memory_in_module(
    operation: &str,
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
    if matches!(operation, "alloc" | "free") && !unsafe_context {
        diagnostics.push(module_diagnostic(
            module,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    match operation {
        "alloc" => {
            if arguments.len() != 2 {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "core.alloc requires size and alignment",
                    span,
                ));
                return ScalarType::Error;
            }
            for argument in arguments {
                let actual = expression_type_in_module_expected(
                    argument,
                    Some(&ScalarType::U64),
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
                    &ScalarType::U64,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        }
        "free" => {
            let [pointer] = arguments else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "core.free requires one pointer",
                    span,
                ));
                return ScalarType::Error;
            };
            let actual = expression_type_in_module(
                pointer,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            if !matches!(actual, ScalarType::RawPointer(_)) && !is_error_type(&actual) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "core.free requires a raw pointer",
                    expression_span(pointer),
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        "system_panic" => {
            if !arguments.is_empty() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "core.system_panic accepts no arguments",
                    span,
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        _ => unreachable!(),
    }
}

fn type_core_memory(
    operation: &str,
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if matches!(operation, "alloc" | "free") && !unsafe_context {
        diagnostics.push(diagnostic(
            program,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    match operation {
        "alloc" => {
            if arguments.len() != 2 {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "core.alloc requires size and alignment",
                    span,
                ));
                return ScalarType::Error;
            }
            for argument in arguments {
                let actual = expression_type_expected(
                    argument,
                    &ScalarType::U64,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    &ScalarType::U64,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        }
        "free" => {
            let [pointer] = arguments else {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "core.free requires one pointer",
                    span,
                ));
                return ScalarType::Error;
            };
            let actual = expression_type(
                pointer,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !matches!(actual, ScalarType::RawPointer(_)) && !is_error_type(&actual) {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "core.free requires a raw pointer",
                    expression_span(pointer),
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        "system_panic" => {
            if !arguments.is_empty() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "core.system_panic accepts no arguments",
                    span,
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        _ => unreachable!(),
    }
}

fn type_core_int_conversion_in_module(
    operation: &str,
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
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !is_integer_type(destination) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} has an invalid integer destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = expression_type_in_module_expected(
        &arguments[0],
        source_context.as_ref(),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    let source_width = integer_width(&actual);
    let destination_width = integer_width(destination);
    if !is_error_type(&actual) && (source_width.is_none() || destination_width.is_none()) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires integer source and destination types"),
            expression_span(&arguments[0]),
        ));
    }
    if source_width.is_some_and(|source| {
        destination_width.is_some_and(|destination| match operation {
            "int_trunc" => source <= destination,
            "int_extend" => source >= destination,
            _ => unreachable!(),
        })
    }) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate integer width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || source_width.is_none() {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn float_conversion_destination_matches(operation: &str, destination: &ScalarType) -> bool {
    match operation {
        "uint_to_float" | "sint_to_float" => is_float_type(destination),
        "float_to_sint_trunc" => matches!(
            destination,
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128
        ),
        "float_to_uint_trunc" => matches!(
            destination,
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128
        ),
        "float_trunc" => *destination == ScalarType::F32,
        "float_extend" => *destination == ScalarType::F64,
        _ => unreachable!(),
    }
}

fn float_conversion_source_matches(operation: &str, actual: &ScalarType) -> bool {
    match operation {
        "uint_to_float" => matches!(
            actual,
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128
        ),
        "sint_to_float" => matches!(
            actual,
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128
        ),
        "float_to_sint_trunc" | "float_to_uint_trunc" | "float_trunc" | "float_extend" => {
            is_float_type(actual)
        }
        _ => unreachable!(),
    }
}

fn float_conversion_direction_mismatched(
    operation: &str,
    destination: &ScalarType,
    actual: &ScalarType,
) -> bool {
    if !is_float_type(destination) || !is_float_type(actual) {
        return false;
    }
    match operation {
        "float_trunc" => !(*destination == ScalarType::F32 && *actual == ScalarType::F64),
        "float_extend" => !(*destination == ScalarType::F64 && *actual == ScalarType::F32),
        _ => false,
    }
}

fn type_core_float_conversion_in_module(
    operation: &str,
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
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !float_conversion_destination_matches(operation, destination) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} has an invalid destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = expression_type_in_module_expected(
        &arguments[0],
        source_context.as_ref(),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    let source_valid = float_conversion_source_matches(operation, &actual);
    if !is_error_type(&actual) && !source_valid {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a compatible source type"),
            expression_span(&arguments[0]),
        ));
    }
    if float_conversion_direction_mismatched(operation, destination, &actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate float width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || !source_valid {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn type_core_float_conversion(
    operation: &str,
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
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !float_conversion_destination_matches(operation, destination) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} has an invalid destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = match source_context {
        Some(source) => expression_type_expected(
            &arguments[0],
            &source,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        None => expression_type(
            &arguments[0],
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
    };
    let source_valid = float_conversion_source_matches(operation, &actual);
    if !is_error_type(&actual) && !source_valid {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a compatible source type"),
            expression_span(&arguments[0]),
        ));
    }
    if float_conversion_direction_mismatched(operation, destination, &actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate float width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || !source_valid {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn type_core_int_conversion(
    operation: &str,
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
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !is_integer_type(destination) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} has an invalid integer destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = match source_context {
        Some(source) => expression_type_expected(
            &arguments[0],
            &source,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        None => expression_type(
            &arguments[0],
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
    };
    let source_width = integer_width(&actual);
    let destination_width = integer_width(destination);
    if !is_error_type(&actual) && (source_width.is_none() || destination_width.is_none()) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires integer source and destination types"),
            expression_span(&arguments[0]),
        ));
    }
    if source_width.is_some_and(|source| {
        destination_width.is_some_and(|destination| match operation {
            "int_trunc" => source <= destination,
            "int_extend" => source >= destination,
            _ => unreachable!(),
        })
    }) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate integer width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || source_width.is_none() {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn type_core_raw_memory_in_module(
    operation: &str,
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
    if !unsafe_context {
        diagnostics.push(module_diagnostic(
            module,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one pointee type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let pointee = &type_arguments[0].ty;
    if operation == "load"
        && matches!(
            pointee,
            ScalarType::Unit | ScalarType::Callable { .. } | ScalarType::Error
        )
    {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "core.load has an invalid pointee type",
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if operation == "offset" && arguments.len() != 2 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "core.offset requires a pointer and an offset",
            span,
        ));
        return ScalarType::Error;
    }
    if operation == "load" && arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "core.load requires one pointer",
            span,
        ));
        return ScalarType::Error;
    }
    let pointer = expression_type_in_module(
        &arguments[0],
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    let mut valid = true;
    if !is_error_type(&pointer) {
        match &pointer {
            ScalarType::RawPointer(inner) => {
                if !is_error_type(pointee) && !scalar_type_equal(inner, pointee) {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        &format!("core.{operation} requires a raw pointer to the pointee type"),
                        expression_span(&arguments[0]),
                    ));
                    valid = false;
                }
            }
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    &format!("core.{operation} requires a raw pointer"),
                    expression_span(&arguments[0]),
                ));
                valid = false;
            }
        }
    }
    if operation == "offset" {
        let count = expression_type_in_module_expected(
            &arguments[1],
            Some(&ScalarType::I64),
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
            &ScalarType::I64,
            &count,
            expression_span(&arguments[1]),
            diagnostics,
        );
        if is_error_type(&count) {
            valid = false;
        }
    }
    if !valid || is_error_type(&pointer) || is_error_type(pointee) {
        return ScalarType::Error;
    }
    if operation == "offset" {
        ScalarType::RawPointer(Box::new(pointee.clone()))
    } else {
        pointee.clone()
    }
}

fn type_core_raw_memory(
    operation: &str,
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
    if !unsafe_context {
        diagnostics.push(diagnostic(
            program,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one pointee type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let pointee = &type_arguments[0].ty;
    if operation == "load"
        && matches!(
            pointee,
            ScalarType::Unit | ScalarType::Callable { .. } | ScalarType::Error
        )
    {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "core.load has an invalid pointee type",
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if operation == "offset" && arguments.len() != 2 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "core.offset requires a pointer and an offset",
            span,
        ));
        return ScalarType::Error;
    }
    if operation == "load" && arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "core.load requires one pointer",
            span,
        ));
        return ScalarType::Error;
    }
    let pointer = expression_type(
        &arguments[0],
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    let mut valid = true;
    if !is_error_type(&pointer) {
        match &pointer {
            ScalarType::RawPointer(inner) => {
                if !is_error_type(pointee) && !scalar_type_equal(inner, pointee) {
                    diagnostics.push(diagnostic(
                        program,
                        "B0003",
                        &format!("core.{operation} requires a raw pointer to the pointee type"),
                        expression_span(&arguments[0]),
                    ));
                    valid = false;
                }
            }
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    &format!("core.{operation} requires a raw pointer"),
                    expression_span(&arguments[0]),
                ));
                valid = false;
            }
        }
    }
    if operation == "offset" {
        let count = expression_type_expected(
            &arguments[1],
            &ScalarType::I64,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        );
        expect_type(
            program,
            &ScalarType::I64,
            &count,
            expression_span(&arguments[1]),
            diagnostics,
        );
        if is_error_type(&count) {
            valid = false;
        }
    }
    if !valid || is_error_type(&pointer) || is_error_type(pointee) {
        return ScalarType::Error;
    }
    if operation == "offset" {
        ScalarType::RawPointer(Box::new(pointee.clone()))
    } else {
        pointee.clone()
    }
}

fn integer_conversion_source_context(
    operation: &str,
    destination: &ScalarType,
    expression: &ScalarExpression,
) -> Option<ScalarType> {
    if matches!(
        operation,
        "float_to_sint_trunc" | "float_to_uint_trunc" | "float_trunc" | "float_extend"
    ) {
        return float_conversion_source_context(operation, expression);
    }
    let value = match expression {
        ScalarExpression::Integer { value, .. } => value.clone(),
        ScalarExpression::Unary {
            operator: UnaryOperator::Negate,
            operand,
            ..
        } => match operand.as_ref() {
            ScalarExpression::Integer { value, .. } => -value,
            _ => return None,
        },
        _ => return None,
    };
    let source_types: &[ScalarType] = match operation {
        "int_trunc" => &[
            ScalarType::I64,
            ScalarType::U64,
            ScalarType::I128,
            ScalarType::U128,
        ],
        "int_extend" => &[
            ScalarType::I32,
            ScalarType::U32,
            ScalarType::I64,
            ScalarType::U64,
            ScalarType::I128,
            ScalarType::U128,
        ],
        "uint_to_float" => &[ScalarType::U32, ScalarType::U64, ScalarType::U128],
        "sint_to_float" => &[ScalarType::I32, ScalarType::I64, ScalarType::I128],
        _ => unreachable!(),
    };
    source_types
        .iter()
        .find(|source| {
            let Some(source_width) = integer_width(source) else {
                return false;
            };
            let width_matches = match operation {
                "int_trunc" | "int_extend" => {
                    let Some(destination_width) = integer_width(destination) else {
                        return false;
                    };
                    match operation {
                        "int_trunc" => source_width > destination_width,
                        "int_extend" => source_width < destination_width,
                        _ => unreachable!(),
                    }
                }
                "uint_to_float" | "sint_to_float" => true,
                _ => unreachable!(),
            };
            width_matches && integer_literal_fits_type(&value, source)
        })
        .cloned()
}

fn float_conversion_source_context(
    operation: &str,
    expression: &ScalarExpression,
) -> Option<ScalarType> {
    let is_float_literal = match expression {
        ScalarExpression::Float { .. } => true,
        ScalarExpression::Unary {
            operator: UnaryOperator::Negate,
            operand,
            ..
        } => matches!(operand.as_ref(), ScalarExpression::Float { .. }),
        _ => false,
    };
    if !is_float_literal {
        return None;
    }
    match operation {
        "float_trunc" => Some(ScalarType::F64),
        "float_extend" => Some(ScalarType::F32),
        "float_to_sint_trunc" | "float_to_uint_trunc" => {
            let value = match expression {
                ScalarExpression::Float { value, .. } => *value,
                ScalarExpression::Unary {
                    operator: UnaryOperator::Negate,
                    operand,
                    ..
                } => match operand.as_ref() {
                    ScalarExpression::Float { value, .. } => -*value,
                    _ => return None,
                },
                _ => return None,
            };
            [ScalarType::F32, ScalarType::F64]
                .into_iter()
                .find(|source| float_literal_fits_type(value, source))
        }
        _ => unreachable!(),
    }
}

fn float_literal_fits_type(value: f64, ty: &ScalarType) -> bool {
    match ty {
        ScalarType::F32 => (value as f32).is_finite(),
        ScalarType::F64 => value.is_finite(),
        _ => false,
    }
}

fn integer_literal_fits_type(value: &BigInt, ty: &ScalarType) -> bool {
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
        _ => return false,
    };
    value >= &minimum && value <= &maximum
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
        ScalarExpression::CheckedAddress {
            mutability,
            place,
            span,
        } => checked_address_type_in_module(
            *mutability,
            place,
            *span,
            scope,
            module,
            modules,
            diagnostics,
        ),
        ScalarExpression::Dereference { place, span } => {
            checked_dereference_type_in_module(place, *span, scope, module, modules, diagnostics)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            place_type_in_module(place, scope, module, modules, diagnostics)
        }
        ScalarExpression::StructLiteral { .. } => ScalarType::Error,
        ScalarExpression::ArrayLiteral { span, .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "array literal requires a fixed array context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            name_span,
            span,
            ..
        } => {
            if let Some((enum_id, _)) = enum_member_in_module(module, modules, receiver, name) {
                if enum_tag.is_none() {
                    diagnostics.push(module_diagnostic(
                        module,
                        "M0002",
                        "unknown enum variant",
                        *name_span,
                    ));
                    return ScalarType::Error;
                }
                return ScalarType::Enum(enum_id);
            }
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
            let right_type = if matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            ) {
                expression_type_in_module_expected(
                    right,
                    Some(&left_type),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                )
            } else {
                expression_type_in_module(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                )
            };
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
                    Some(&utf8_type_from_namespace_target(
                        module,
                        modules,
                        *span,
                        diagnostics,
                    )),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                let expected = utf8_type_from_namespace_target(module, modules, *span, diagnostics);
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
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "alloc" | "free" | "system_panic")
            {
                return type_core_memory_in_module(
                    name,
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
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return type_core_int_conversion_in_module(
                    name,
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
                return type_core_float_conversion_in_module(
                    name,
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
            if receiver.as_deref() == Some("core") && matches!(name.as_str(), "offset" | "load") {
                return type_core_raw_memory_in_module(
                    name,
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
            let overload = match receiver {
                None => module.items.iter().find_map(|item| match item {
                    ScalarItem::Function(function)
                        if function.name == *name && !function.overload_arms.is_empty() =>
                    {
                        Some(function)
                    }
                    _ => None,
                }),
                Some(binding) => module
                    .namespace_bindings
                    .iter()
                    .find(|namespace| namespace.binding == *binding)
                    .and_then(|namespace| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == namespace.target)
                    })
                    .and_then(|target| {
                        target.members.get(name).and_then(|_| {
                            target.items.iter().find_map(|item| match item {
                                ScalarItem::Function(function)
                                    if function.name == *name
                                        && !function.overload_arms.is_empty() =>
                                {
                                    Some(function)
                                }
                                _ => None,
                            })
                        })
                    }),
            };
            if overload.is_some() {
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
            let (callable, target, unsafe_callable, extern_callable) = match receiver {
                None => (scope.get(name), module, false, false),
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
                        (
                            Some(callable),
                            target,
                            false,
                            target.items.iter().any(|item| {
                                matches!(
                                    item,
                                    ScalarItem::Extern(extern_decl)
                                        if extern_decl.functions.iter().any(|function| function.name == *name)
                                )
                            }),
                        )
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
                        (
                            Some(&function.signature),
                            module,
                            function.unsafe_marker,
                            true,
                        )
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
            if extern_callable && !type_arguments.is_empty() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "generic argument arity does not match callable declaration",
                    *span,
                ));
                return ScalarType::Error;
            }
            let generic_parameters = target.items.iter().find_map(|item| match item {
                ScalarItem::Function(function) if function.name == *name => {
                    Some(&function.generic_parameters)
                }
                _ => None,
            });
            let callable = if let Some(generic_parameters) = generic_parameters {
                let Some(callable) =
                    substitute_generic_callable(&callable, generic_parameters, type_arguments)
                else {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0004",
                        "generic argument arity does not match callable declaration",
                        *span,
                    ));
                    return ScalarType::Error;
                };
                callable
            } else {
                callable.clone()
            };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = &callable
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
    if expected.is_some_and(is_error_type) {
        return ScalarType::Error;
    }
    if let ScalarExpression::Call {
        receiver,
        name,
        name_span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        let overload = match receiver {
            None => module.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if function.name == *name && !function.overload_arms.is_empty() =>
                {
                    Some(function)
                }
                _ => None,
            }),
            Some(binding) => module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding)
                .and_then(|namespace| {
                    modules
                        .iter()
                        .find(|candidate| candidate.source == namespace.target)
                })
                .and_then(|target| {
                    target.members.get(name).and_then(|_| {
                        target.items.iter().find_map(|item| match item {
                            ScalarItem::Function(function)
                                if function.name == *name && !function.overload_arms.is_empty() =>
                            {
                                Some(function)
                            }
                            _ => None,
                        })
                    })
                }),
        };
        if let Some(overload) = overload {
            if let Some(type_argument) = type_arguments.first() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "overload calls do not accept explicit type arguments",
                    type_argument.span,
                ));
                return ScalarType::Error;
            }
            let argument_types = arguments
                .iter()
                .map(|argument| {
                    expression_type_in_module(
                        argument,
                        scope,
                        visible_names,
                        folded_names,
                        module,
                        modules,
                        diagnostics,
                        unsafe_context,
                    )
                })
                .collect::<Vec<_>>();
            let overload_arguments = arguments
                .iter()
                .zip(&argument_types)
                .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                .collect::<Vec<_>>();
            let callable = match resolve_overload_candidate(overload, &overload_arguments, expected)
            {
                Ok((callable, _)) => callable,
                Err(message) => {
                    diagnostics.push(module_diagnostic(module, "B0004", message, *name_span));
                    return ScalarType::Error;
                }
            };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
            else {
                unreachable!("overload arms are callable")
            };
            for (argument, parameter) in arguments.iter().zip(&parameters) {
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
                expect_module_type(
                    module,
                    parameter,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            return scalar_call_result_in_module(&outputs);
        }
    }
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
            if let Some(
                pointer @ ScalarType::RawPointer(_) | pointer @ ScalarType::CheckedReference { .. },
            ) = expected
            {
                return pointer.clone();
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
        if let Some(utf8) = utf8_type_from_context(module, modules, expected) {
            return utf8;
        }
        let utf8 = utf8_type_from_namespace_target(module, modules, *span, diagnostics);
        expect_module_type(module, &utf8, expected, *span, diagnostics);
        return ScalarType::Error;
    }
    if let ScalarExpression::ArrayLiteral { elements, span } = expression {
        let Some(ScalarType::Array {
            element, length, ..
        }) = expected
        else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "array literal requires a fixed array context",
                *span,
            ));
            return ScalarType::Error;
        };
        if elements.len() as u64 != *length {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "array literal element count does not match fixed array length",
                *span,
            ));
        }
        for value in elements {
            let actual = expression_type_in_module_expected(
                value,
                Some(element),
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
                element,
                &actual,
                expression_span(value),
                diagnostics,
            );
        }
        return expected.cloned().expect("array context");
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
                } else if comparison {
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

fn utf8_type_from_namespace_target(
    module: &ScalarModule,
    modules: &[ScalarModule],
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let structure = module
        .namespace_bindings
        .iter()
        .filter_map(|binding| {
            modules
                .iter()
                .find(|candidate| candidate.source == binding.target)
        })
        .flat_map(|target| target.structs.iter())
        .find(|structure| structure.name == "utf8");
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

fn utf8_type_from_context(
    module: &ScalarModule,
    modules: &[ScalarModule],
    expected: &ScalarType,
) -> Option<ScalarType> {
    let ScalarType::Struct(expected_id) = expected else {
        return None;
    };
    module
        .namespace_bindings
        .iter()
        .filter(|binding| binding.target == expected_id.source)
        .filter_map(|binding| {
            modules
                .iter()
                .find(|candidate| candidate.source == binding.target)
        })
        .flat_map(|target| target.structs.iter())
        .find(|structure| structure.id == *expected_id && structure.name == "utf8")
        .map(|structure| ScalarType::Struct(structure.id.clone()))
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

fn is_supported_checked_address_place(place: &ScalarPlace) -> bool {
    match place {
        ScalarPlace::Name { .. } => true,
        ScalarPlace::Field { base, .. } => matches!(base.as_ref(), ScalarPlace::Name { .. }),
        ScalarPlace::Dereference { .. } => false,
        ScalarPlace::Index { base, .. } => is_supported_checked_address_place(base),
    }
}

fn is_addressable_checked_reference_type(ty: &ScalarType) -> bool {
    !matches!(
        ty,
        ScalarType::Unit | ScalarType::Callable { .. } | ScalarType::Error
    )
}

fn checked_address_type(
    mutability: ScalarReferenceMutability,
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if !is_supported_checked_address_place(place) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    let inner = place_type(place, scope, program, diagnostics);
    if !is_addressable_checked_reference_type(&inner) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    ScalarType::CheckedReference {
        mutability,
        inner: Box::new(inner),
    }
}

fn checked_address_type_in_module(
    mutability: ScalarReferenceMutability,
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if !is_supported_checked_address_place(place) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    let inner = place_type_in_module(place, scope, module, modules, diagnostics);
    if !is_addressable_checked_reference_type(&inner) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    ScalarType::CheckedReference {
        mutability,
        inner: Box::new(inner),
    }
}

fn writable_place_type(
    place: &ScalarPlace,
    target_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if writable_place_contains_raw_dereference(place, scope, program, diagnostics) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "assignment through raw pointer dereference is not supported",
            target_span,
        ));
        return ScalarType::Error;
    }
    let ty = writable_path_type(place, scope, program, diagnostics);
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "assignment place requires a scalar leaf type",
            target_span,
        ));
        return ScalarType::Error;
    }
    ty
}

fn writable_place_type_in_module(
    place: &ScalarPlace,
    target_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if writable_place_contains_raw_dereference_in_module(place, scope, module, modules, diagnostics)
    {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "assignment through raw pointer dereference is not supported",
            target_span,
        ));
        return ScalarType::Error;
    }
    let ty = writable_path_type_in_module(place, scope, module, modules, diagnostics);
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "assignment place requires a scalar leaf type",
            target_span,
        ));
        return ScalarType::Error;
    }
    ty
}

fn writable_place_contains_raw_dereference(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    match place {
        ScalarPlace::Name { .. } => false,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => {
            writable_place_contains_raw_dereference(base, scope, program, diagnostics)
        }
        ScalarPlace::Dereference { pointer, .. } => matches!(
            expression_type(
                pointer,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                true,
            ),
            ScalarType::RawPointer(_)
        ),
    }
}

fn writable_place_contains_raw_dereference_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    match place {
        ScalarPlace::Name { .. } => false,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => {
            writable_place_contains_raw_dereference_in_module(
                base,
                scope,
                module,
                modules,
                diagnostics,
            )
        }
        ScalarPlace::Dereference { pointer, .. } => matches!(
            expression_type_in_module(
                pointer,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                true,
            ),
            ScalarType::RawPointer(_)
        ),
    }
}

fn writable_path_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { .. } => place_type(place, scope, program, diagnostics),
        ScalarPlace::Dereference { pointer, span } => match expression_type(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            false,
        ) {
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable,
                inner,
            } => *inner,
            ScalarType::Error => ScalarType::Error,
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "assignment dereference requires a mutable checked reference",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Field { base, field, span } => {
            let ScalarType::Struct(structure) =
                writable_path_type(base, scope, program, diagnostics)
            else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let Some(field) = program
                .structs
                .iter()
                .find(|candidate| candidate.id == structure && field.structure == structure)
                .and_then(|candidate| candidate.fields.get(field.index))
                .filter(|candidate| candidate.id == *field)
            else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            field.ty.clone()
        }
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = writable_path_type(base, scope, program, diagnostics);
            let index_type = expression_type_expected(
                index,
                &ScalarType::U64,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                false,
            );
            expect_type(
                program,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(diagnostic(
                        program,
                        "B0003",
                        "indexed place requires a fixed array",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
        }
    }
}

fn writable_path_type_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { .. } => {
            place_type_in_module(place, scope, module, modules, diagnostics)
        }
        ScalarPlace::Dereference { pointer, span } => match expression_type_in_module(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            module,
            modules,
            diagnostics,
            false,
        ) {
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable,
                inner,
            } => *inner,
            ScalarType::Error => ScalarType::Error,
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "assignment dereference requires a mutable checked reference",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Field { base, field, span } => {
            let ScalarType::Struct(structure) =
                writable_path_type_in_module(base, scope, module, modules, diagnostics)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
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
            let Some(field) = modules
                .iter()
                .flat_map(|candidate| candidate.structs.iter())
                .find(|candidate| candidate.id == structure && field.structure == structure)
                .and_then(|candidate| candidate.fields.get(field.index))
                .filter(|candidate| candidate.id == *field)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            };
            field.ty.clone()
        }
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = writable_path_type_in_module(base, scope, module, modules, diagnostics);
            let index_type = expression_type_in_module_expected(
                index,
                Some(&ScalarType::U64),
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                false,
            );
            expect_module_type(
                module,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "indexed place requires a fixed array",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
        }
    }
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
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = place_type(base, scope, program, diagnostics);
            let index_type = expression_type_expected(
                index,
                &ScalarType::U64,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                false,
            );
            expect_type(
                program,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(diagnostic(
                        program,
                        "B0003",
                        "indexed place requires a fixed array",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
        }
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
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = place_type_in_module(base, scope, module, modules, diagnostics);
            let index_type = expression_type_in_module_expected(
                index,
                Some(&ScalarType::U64),
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                false,
            );
            expect_module_type(
                module,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "indexed place requires a fixed array",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
        }
    }
}

fn checked_dereference_target_type(
    pointer: &ScalarExpression,
    span: ByteSpan,
    allow_aggregate: bool,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match expression_type(
        pointer,
        scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        program,
        diagnostics,
        false,
    ) {
        ScalarType::CheckedReference { inner, .. } => match inner.as_ref() {
            ScalarType::Unit => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "cannot read unit through checked reference",
                    span,
                ));
                ScalarType::Error
            }
            ScalarType::Struct(_) | ScalarType::Array { .. } if !allow_aggregate => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "whole aggregate checked dereference read is not supported",
                    span,
                ));
                ScalarType::Error
            }
            _ => *inner,
        },
        ScalarType::RawPointer(_) => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "checked dereference requires a checked reference",
                span,
            ));
            ScalarType::Error
        }
        _ => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "cannot dereference value",
                span,
            ));
            ScalarType::Error
        }
    }
}

fn checked_dereference_type(
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Dereference { pointer, span } => {
            checked_dereference_target_type(pointer, *span, false, scope, program, diagnostics)
        }
        ScalarPlace::Field {
            base,
            field,
            span: field_span,
        } => {
            let ScalarPlace::Dereference { pointer, span } = base.as_ref() else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "checked dereference requires a direct struct field",
                    span,
                ));
                return ScalarType::Error;
            };
            let base_type =
                checked_dereference_target_type(pointer, *span, true, scope, program, diagnostics);
            let ScalarFieldReference::Resolved(field) = field else {
                let ScalarFieldReference::Unresolved { span, .. } = field else {
                    unreachable!("field reference")
                };
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let ScalarType::Struct(structure) = base_type else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "value has no field",
                    *field_span,
                ));
                return ScalarType::Error;
            };
            let candidate = program
                .structs
                .iter()
                .find(|candidate| candidate.id == structure)
                .and_then(|candidate| candidate.fields.get(field.index))
                .filter(|candidate| candidate.id == *field)
                .cloned();
            let Some(candidate) = candidate else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "value has no field",
                    *field_span,
                ));
                return ScalarType::Error;
            };
            if candidate.ty == ScalarType::Unit {
                let field_name_span =
                    ByteSpan::new(field_span.end - candidate.name.len() as u32, field_span.end);
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "cannot read unit through checked reference",
                    field_name_span,
                ));
                ScalarType::Error
            } else {
                candidate.ty
            }
        }
        ScalarPlace::Name { .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            ScalarType::Error
        }
        ScalarPlace::Index { .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            ScalarType::Error
        }
    }
}

fn checked_dereference_type_in_module(
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let (pointer, dereference_span, field) = match place {
        ScalarPlace::Dereference { pointer, span } => (pointer, *span, None),
        ScalarPlace::Field {
            base,
            field,
            span: field_span,
        } => {
            let ScalarPlace::Dereference {
                pointer,
                span: dereference_span,
            } = base.as_ref()
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "checked dereference requires a direct struct field",
                    *field_span,
                ));
                return ScalarType::Error;
            };
            (pointer, *dereference_span, Some((field, *field_span)))
        }
        ScalarPlace::Name { .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            return ScalarType::Error;
        }
        ScalarPlace::Index { .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            return ScalarType::Error;
        }
    };
    let allow_aggregate = field.is_some();
    let inner = match expression_type_in_module(
        pointer,
        scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        module,
        modules,
        diagnostics,
        false,
    ) {
        ScalarType::CheckedReference { inner, .. } => match inner.as_ref() {
            ScalarType::Unit => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "cannot read unit through checked reference",
                    dereference_span,
                ));
                return ScalarType::Error;
            }
            ScalarType::Struct(_) | ScalarType::Array { .. } if !allow_aggregate => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "whole aggregate checked dereference read is not supported",
                    dereference_span,
                ));
                return ScalarType::Error;
            }
            _ => *inner,
        },
        ScalarType::RawPointer(_) => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "checked dereference requires a checked reference",
                dereference_span,
            ));
            return ScalarType::Error;
        }
        _ => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "cannot dereference value",
                dereference_span,
            ));
            return ScalarType::Error;
        }
    };
    let (field, field_span) = match field {
        None => return inner,
        Some((ScalarFieldReference::Resolved(field), field_span)) => (field, field_span),
        Some((ScalarFieldReference::Unresolved { span, .. }, _)) => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "value has no field",
                *span,
            ));
            return ScalarType::Error;
        }
    };
    let ScalarType::Struct(structure) = inner else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            field_span,
        ));
        return ScalarType::Error;
    };
    let candidate = modules
        .iter()
        .flat_map(|candidate| candidate.structs.iter())
        .find(|candidate| candidate.id == structure)
        .and_then(|candidate| candidate.fields.get(field.index))
        .filter(|candidate| candidate.id == *field)
        .cloned();
    let Some(candidate) = candidate else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            field_span,
        ));
        return ScalarType::Error;
    };
    if candidate.ty == ScalarType::Unit {
        let field_name_span =
            ByteSpan::new(field_span.end - candidate.name.len() as u32, field_span.end);
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "cannot read unit through checked reference",
            field_name_span,
        ));
        ScalarType::Error
    } else {
        candidate.ty
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

fn is_float_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::F32 | ScalarType::F64)
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

fn validate_identifier_style(
    module: &ScalarModule,
    name: &str,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_declared_identifier_style(name) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE",
            span,
        ));
    }
}

fn validate_program_identifier_style(
    program: &ScalarProgram,
    name: &str,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_declared_identifier_style(name) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE",
            span,
        ));
    }
}

fn is_declared_identifier_style(name: &str) -> bool {
    if name == "_" {
        return true;
    }
    let bytes = name.as_bytes();
    let snake_case = bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_');
    let pascal_case =
        bytes[0].is_ascii_uppercase() && bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric());
    let screaming_snake_case = bytes.iter().any(|byte| byte.is_ascii_uppercase())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_');
    snake_case || pascal_case || screaming_snake_case
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

fn is_module_private_name(name: &str) -> bool {
    name.starts_with('_')
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
            let name = direct_token(&receiver, SyntaxKind::Identifier).expect("receiver name");
            let allocation_length = receiver
                .children()
                .find(|child| child.kind() == SyntaxKind::Expression)
                .map(|child| derive_expression(&child));
            let ty = allocation_length.as_ref().map_or_else(
                || {
                    let ty_node = direct_nodes(&receiver)
                        .into_iter()
                        .next()
                        .expect("receiver type");
                    derive_type(&ty_node)
                },
                |_| {
                    let element = direct_token(&receiver, SyntaxKind::TypeName)
                        .expect("runtime array element type");
                    ScalarType::RuntimeArray {
                        element: Box::new(type_from_name(element.text(), token_span(&element))),
                        span: wosy_syntax::byte_span(&receiver),
                    }
                },
            );
            ScalarOutputReceiver {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                ty,
                allocation_length,
                span: wosy_syntax::byte_span(&receiver),
            }
        })
        .collect::<Vec<_>>();
    let declared_type = receivers[0].ty.clone();
    let output_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::OutputList);
    let outputs = output_list.as_ref().map_or_else(Vec::new, |output_list| {
        output_list
            .children()
            .filter(|child| child.kind() == SyntaxKind::Expression)
            .map(|child| derive_expression(&child))
            .collect::<Vec<_>>()
    });
    let value = outputs
        .first()
        .cloned()
        .unwrap_or(ScalarExpression::Integer {
            value: BigInt::from(0),
            span: wosy_syntax::byte_span(node),
        });
    let name = receivers[0].name.clone();
    let output_values = receivers
        .iter()
        .enumerate()
        .map(|(position, receiver)| ScalarOutputValue {
            position,
            ty: receiver.ty.clone(),
            span: if outputs.is_empty() {
                receiver.span
            } else {
                span_of(if outputs.len() == 1 {
                    &value
                } else {
                    &outputs[position]
                })
            },
            value: if outputs.is_empty() || outputs.len() == 1 {
                value.clone()
            } else {
                outputs[position].clone()
            },
        })
        .collect::<Vec<_>>();
    let output_origin = match outputs.len() {
        1 => ScalarBindingOutputOrigin::SingleExpression,
        _ => ScalarBindingOutputOrigin::IndependentExpressions,
    };
    let output_sequence = ScalarOutputSequence {
        outputs: receivers
            .iter()
            .enumerate()
            .map(|(position, receiver)| ScalarOutput {
                ty: receiver.ty.clone(),
                span: output_values[position].span,
            })
            .collect(),
        span: output_list
            .as_ref()
            .map_or_else(|| wosy_syntax::byte_span(node), wosy_syntax::byte_span),
    };
    ScalarBinding {
        name,
        name_span: receivers[0].name_span,
        declared_type,
        value,
        receivers,
        output_sequence,
        output_values,
        output_origin,
        is_allocation: outputs.is_empty(),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_struct(
    node: CstNode,
    id: ScalarStructId,
    target_layout: ScalarTargetLayout,
) -> ScalarStruct {
    let name = direct_token(&node, SyntaxKind::Identifier).expect("struct name");
    let mut fields = node
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
                layout: None,
                ty,
                declaration_index: index,
                offset: None,
            }
        })
        .collect::<Vec<_>>();
    compute_initial_struct_fields_layout(&mut fields, target_layout);
    let layout = initial_struct_layout(&fields);
    ScalarStruct {
        id,
        name: name.text().to_owned(),
        name_span: token_span(&name),
        fields,
        span: wosy_syntax::byte_span(&node),
        layout,
    }
}

fn derive_enum(node: CstNode, id: ScalarEnumId) -> ScalarEnum {
    let name = direct_token(&node, SyntaxKind::Identifier).expect("enum name");
    let variants = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::EnumVariant)
        .enumerate()
        .map(|(tag, variant)| {
            let name = direct_token(&variant, SyntaxKind::Identifier).expect("enum variant");
            ScalarEnumVariant {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                tag: u32::try_from(tag).expect("enum variant tag fits u32"),
            }
        })
        .collect();
    ScalarEnum {
        id,
        name: name.text().to_owned(),
        name_span: token_span(&name),
        variants,
        span: wosy_syntax::byte_span(&node),
    }
}

fn checked_align_offset(offset: u64, alignment: u64) -> Option<u64> {
    offset
        .checked_add(alignment - 1)
        .and_then(|value| value.checked_div(alignment))
        .and_then(|value| value.checked_mul(alignment))
}

fn checked_layout_offset(offset: u64, layout: &ScalarLayout) -> Option<(u64, u64)> {
    let field_offset = checked_align_offset(offset, layout.alignment)?;
    let next_offset = field_offset.checked_add(layout.size)?;
    Some((field_offset, next_offset))
}

fn clear_struct_layout(structure: &mut ScalarStruct) {
    structure.layout = None;
    for field in &mut structure.fields {
        field.layout = None;
        field.offset = None;
    }
}

fn compute_initial_struct_layout(structure: &mut ScalarStruct, target_layout: ScalarTargetLayout) {
    compute_initial_struct_fields_layout(&mut structure.fields, target_layout);
    structure.layout = initial_struct_layout(&structure.fields);
}

fn compute_initial_struct_fields_layout(
    fields: &mut [ScalarStructField],
    target_layout: ScalarTargetLayout,
) {
    for field in fields.iter_mut() {
        field.layout = None;
        field.offset = None;
    }
    let mut offset = 0;
    for index in 0..fields.len() {
        let Ok(layout) = layout_for_type(&fields[index].ty, target_layout) else {
            return;
        };
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            for field in fields.iter_mut() {
                field.layout = None;
                field.offset = None;
            }
            return;
        };
        fields[index].offset = Some(field_offset);
        fields[index].layout = Some(layout.clone());
        offset = next_offset;
    }
}

fn initial_struct_layout(fields: &[ScalarStructField]) -> Option<ScalarLayout> {
    let mut offset = 0;
    let mut alignment = 1;
    for field in fields {
        let layout = field.layout.as_ref()?;
        alignment = alignment.max(layout.alignment);
        let (_, next_offset) = checked_layout_offset(offset, layout)?;
        offset = next_offset;
    }
    Some(ScalarLayout {
        size: checked_align_offset(offset, alignment)?,
        alignment,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutError {
    ArraySizeOverflow(ByteSpan),
    AggregateSizeOverflow(ByteSpan),
    InvalidType,
}

fn layout_for_type(
    ty: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    Ok(match ty {
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
        ScalarType::RawPointer(_)
        | ScalarType::CheckedReference { .. }
        | ScalarType::ArtifactId => ScalarLayout {
            size: target_layout.pointer_size,
            alignment: target_layout.pointer_alignment,
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type(element, target_layout)?;
            ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            }
        }
        ScalarType::Error => return Err(LayoutError::InvalidType),
        _ => ScalarLayout {
            size: 0,
            alignment: 1,
        },
    })
}

fn layout_error_source(
    source: &SourceIdentity,
    error: LayoutError,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (message, span, label) = match error {
        LayoutError::ArraySizeOverflow(span) => (
            "fixed array layout exceeds u64",
            span,
            "fixed array length overflows its layout",
        ),
        LayoutError::AggregateSizeOverflow(span) => (
            "struct layout exceeds u64",
            span,
            "field overflows its enclosing struct layout",
        ),
        LayoutError::InvalidType => return,
    };
    diagnostics.push(super::Diagnostic {
        code: "B0003".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span),
            message: label.to_owned(),
        }],
        notes: Vec::new(),
    });
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
                                if matches!(
                                    node.kind(),
                                    SyntaxKind::RawPointerType | SyntaxKind::CheckedPointerType
                                ) =>
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

fn derive_function(node: &CstNode) -> ScalarFunction {
    let children = direct_nodes(node);
    let signature = derive_type(
        children
            .iter()
            .find(|child| child.kind() == SyntaxKind::CallableType)
            .expect("function signature"),
    );
    let name = direct_token(node, SyntaxKind::Identifier).expect("function name");
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
    let mut body = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Block)
        .map(|node| derive_block(node))
        .expect("function block");
    if let ScalarType::Callable { outputs, .. } = &signature {
        expand_conditional_final_outputs(&mut body, outputs.outputs.len());
        for (value, output) in body.final_output_values.iter_mut().zip(&outputs.outputs) {
            value.ty = output.ty.clone();
        }
    }
    ScalarFunction {
        name: name.text().to_owned(),
        name_span: token_span(&name),
        signature,
        parameters,
        parameter_spans,
        body,
        span: wosy_syntax::byte_span(node),
        generic_parameters: Vec::new(),
        overload_arms: Vec::new(),
    }
}

fn derive_generic_parameters(node: &CstNode) -> Vec<ScalarGenericParameter> {
    direct_token(node, SyntaxKind::Identifier)
        .map(|token| {
            vec![ScalarGenericParameter {
                name: token.text().to_owned(),
                span: token_span(&token),
            }]
        })
        .unwrap_or_default()
}

fn derive_generic_function(node: &CstNode) -> ScalarFunction {
    let generic = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::GenericDecl)
        .expect("generic declaration");
    let function = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::FunctionDecl)
        .expect("generic function declaration");
    let mut function = derive_function(&function);
    function.generic_parameters = derive_generic_parameters(&generic);
    function.span = wosy_syntax::byte_span(node);
    function
}

fn derive_overload(node: &CstNode) -> ScalarFunction {
    let name = direct_token(node, SyntaxKind::Identifier).expect("overload name");
    let arms: Vec<ScalarFunction> = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::OverloadArm)
        .map(|arm| {
            let children = direct_nodes(&arm);
            let signature = derive_type(
                children
                    .iter()
                    .find(|child| child.kind() == SyntaxKind::CallableType)
                    .expect("overload arm signature"),
            );
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
            let mut body = derive_block(
                children
                    .iter()
                    .find(|child| child.kind() == SyntaxKind::Block)
                    .expect("overload arm block"),
            );
            if let ScalarType::Callable { outputs, .. } = &signature {
                expand_conditional_final_outputs(&mut body, outputs.outputs.len());
                for (value, output) in body.final_output_values.iter_mut().zip(&outputs.outputs) {
                    value.ty = output.ty.clone();
                }
            }
            ScalarFunction {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                signature,
                parameters,
                parameter_spans,
                body,
                span: wosy_syntax::byte_span(&arm),
                generic_parameters: children
                    .iter()
                    .find(|child| child.kind() == SyntaxKind::GenericDecl)
                    .map_or_else(Vec::new, derive_generic_parameters),
                overload_arms: Vec::new(),
            }
        })
        .collect();
    let signature = arms.first().expect("overload arm").signature.clone();
    ScalarFunction {
        name: name.text().to_owned(),
        name_span: token_span(&name),
        signature,
        parameters: Vec::new(),
        parameter_spans: Vec::new(),
        body: ScalarBlock {
            items: Vec::new(),
            terminated_items: Vec::new(),
            expressions: Vec::new(),
            final_output_values: Vec::new(),
            span: wosy_syntax::byte_span(node),
            unsafe_context: false,
        },
        span: wosy_syntax::byte_span(node),
        generic_parameters: Vec::new(),
        overload_arms: arms,
    }
}

fn expand_conditional_final_outputs(block: &mut ScalarBlock, output_count: usize) {
    if block.final_output_values.len() != 1 || output_count < 2 {
        return;
    }
    let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        ..
    } = &block.final_output_values[0].value
    else {
        return;
    };
    let mut then_branch = then_branch.clone();
    let mut else_branch = else_branch.clone();
    expand_conditional_final_outputs(&mut then_branch, output_count);
    expand_conditional_final_outputs(&mut else_branch, output_count);
    if then_branch.final_output_values.len() != output_count
        || else_branch.final_output_values.len() != output_count
    {
        return;
    }
    let condition = condition.clone();
    let conditional_span = span_of(&block.final_output_values[0].value);
    block.final_output_values = (0..output_count)
        .map(|position| {
            let value = ScalarExpression::If {
                condition: condition.clone(),
                then_branch: select_final_output(&then_branch, position),
                else_branch: select_final_output(&else_branch, position),
                span: conditional_span,
            };
            ScalarOutputValue {
                position,
                ty: ScalarType::Error,
                span: span_of(&value),
                value,
            }
        })
        .collect();
    let final_items = block
        .final_output_values
        .iter()
        .map(|output| ScalarBlockItem::Expression(output.value.clone()));
    let final_start = block.items.len() - 1;
    block.items.truncate(final_start);
    block.terminated_items.truncate(final_start);
    block.items.extend(final_items);
    block
        .terminated_items
        .extend((0..output_count).map(|_| false));
    block.expressions = block
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
}

fn select_final_output(block: &ScalarBlock, position: usize) -> ScalarBlock {
    let mut selected = block.clone();
    let final_start = selected.items.len() - selected.final_output_values.len();
    selected.items.truncate(final_start);
    selected.terminated_items.truncate(final_start);
    selected.final_output_values = vec![block.final_output_values[position].clone()];
    selected.items.push(ScalarBlockItem::Expression(
        selected.final_output_values[0].value.clone(),
    ));
    selected.terminated_items.push(false);
    selected.expressions = selected
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    selected
}

fn derive_type(node: &CstNode) -> ScalarType {
    let actual = if matches!(node.kind(), SyntaxKind::TypeSpec | SyntaxKind::Punctuation) {
        node.descendants()
            .find(|child| {
                matches!(
                    child.kind(),
                    SyntaxKind::CallableType
                        | SyntaxKind::RawPointerType
                        | SyntaxKind::CheckedPointerType
                        | SyntaxKind::ParenthesizedType
                        | SyntaxKind::QualifiedType
                )
            })
            .unwrap_or_else(|| node.clone())
    } else {
        node.clone()
    };
    let mut ty = match actual.kind() {
        SyntaxKind::CheckedPointerType => {
            let inner = actual
                .children()
                .find(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::ParenthesizedType
                            | SyntaxKind::QualifiedType
                            | SyntaxKind::Punctuation
                    )
                })
                .map(|child| derive_type(&child))
                .unwrap_or_else(|| {
                    let token = direct_token(&actual, SyntaxKind::TypeName)
                        .expect("checked reference type");
                    type_from_name(token.text(), token_span(&token))
                });
            ScalarType::CheckedReference {
                mutability: if actual.children_with_tokens().any(
                    |element| matches!(element, NodeOrToken::Token(token) if token.text() == "!"),
                ) {
                    ScalarReferenceMutability::Mutable
                } else {
                    ScalarReferenceMutability::Shared
                },
                inner: Box::new(inner),
            }
        }
        SyntaxKind::ParenthesizedType => {
            derive_type(&actual.children().next().expect("parenthesized type inner"))
        }
        SyntaxKind::RawPointerType => {
            let inner = actual
                .children()
                .find(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::Punctuation
                            | SyntaxKind::QualifiedType
                            | SyntaxKind::RawPointerType
                    )
                })
                .map(|child| derive_type(&child))
                .unwrap_or_else(|| {
                    let token =
                        direct_token(&actual, SyntaxKind::TypeName).expect("raw pointer type");
                    type_from_name(token.text(), token_span(&token))
                });
            ScalarType::RawPointer(Box::new(inner))
        }
        SyntaxKind::QualifiedType => qualified_type(&actual),
        SyntaxKind::CallableType => {
            let output = actual
                .descendants()
                .find(|child| child.kind() == SyntaxKind::CallableOutput)
                .expect("callable output");
            let outputs = ScalarOutputSequence {
                outputs: output
                    .children()
                    .map(|child| ScalarOutput {
                        ty: derive_callable_array_type(&child),
                        span: wosy_syntax::byte_span(&child),
                    })
                    .collect(),
                span: wosy_syntax::byte_span(&actual),
            };
            let parameters = actual
                .children()
                .filter(|child| child.kind() != SyntaxKind::CallableOutput)
                .filter(|child| child.kind() == SyntaxKind::Punctuation)
                .filter(|child| child.text() != "(")
                .filter(|child| child.text() != ")")
                .map(|child| derive_callable_array_type(&child))
                .collect();
            ScalarType::Callable {
                outputs,
                parameters,
            }
        }
        _ => {
            let token = actual
                .descendants_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::TypeName => {
                        Some(token)
                    }
                    _ => None,
                })
                .next()
                .expect("type token");
            type_from_name(token.text(), token_span(&token))
        }
    };
    for suffix in node
        .children()
        .filter(|child| child.kind() == SyntaxKind::FixedArraySuffix)
    {
        let length = suffix
            .children()
            .find(|child| child.kind() == SyntaxKind::FixedArrayLength)
            .expect("fixed array length");
        let token = direct_token(&length, SyntaxKind::Integer).expect("fixed array length token");
        let length_span = wosy_syntax::byte_span(&length);
        let negative = length
            .children_with_tokens()
            .any(|element| matches!(element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::Punctuation && token.text() == "-"));
        if negative {
            return ScalarType::Error;
        }
        let Some(length) = parse_integer(token.text()).and_then(|value| u64::try_from(value).ok())
        else {
            return ScalarType::Error;
        };
        ty = ScalarType::Array {
            element: Box::new(ty),
            length,
            length_span,
            span: wosy_syntax::byte_span(node),
        };
    }
    for _ in node
        .children()
        .filter(|child| child.kind() == SyntaxKind::ErasedArraySuffix)
    {
        ty = ScalarType::RuntimeArray {
            element: Box::new(ty),
            span: wosy_syntax::byte_span(node),
        };
    }
    if !matches!(ty, ScalarType::Callable { .. }) {
        let erased_suffixes = node.text().to_string().match_indices("[]").count();
        for _ in runtime_array_depth(&ty)..erased_suffixes {
            ty = ScalarType::RuntimeArray {
                element: Box::new(ty),
                span: wosy_syntax::byte_span(node),
            };
        }
    }
    ty
}

fn runtime_array_depth(ty: &ScalarType) -> usize {
    match ty {
        ScalarType::RuntimeArray { element, .. } => 1 + runtime_array_depth(element),
        _ => 0,
    }
}

fn derive_callable_array_type(node: &CstNode) -> ScalarType {
    derive_type(node)
}

fn fixed_array_length_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::FixedArrayLength)
        .filter_map(|length| {
            let token = direct_token(&length, SyntaxKind::Integer)?;
            let negative = length
                .children_with_tokens()
                .any(|element| matches!(element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::Punctuation && token.text() == "-"));
            (!negative && parse_integer(token.text())
                .and_then(|value| u64::try_from(value).ok())
                .is_none()
                || negative)
                .then(|| super::Diagnostic {
                    code: "B0003".to_owned(),
                    severity: super::DiagnosticSeverity::Error,
                    message: "fixed array length must be an unsigned u64".to_owned(),
                    labels: vec![super::DiagnosticLabel {
                        kind: super::DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(canonical.source.clone(), wosy_syntax::byte_span(&length)),
                        message: "invalid fixed array length".to_owned(),
                    }],
                    notes: Vec::new(),
                })
        })
        .collect()
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
    let mut items: Vec<ScalarBlockItem> = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::BlockItem)
        .map(|item| derive_block_item(&item))
        .collect();
    let final_output_values = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::FinalOutputList)
        .map(|final_list| {
            let output_list = direct_nodes(&final_list)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::OutputList)
                .expect("final output list");
            output_list
                .children()
                .filter(|child| child.kind() == SyntaxKind::Expression)
                .enumerate()
                .map(|(position, child)| {
                    let value = derive_expression(&child);
                    ScalarOutputValue {
                        position,
                        ty: ScalarType::Error,
                        span: span_of(&value),
                        value,
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    items.extend(
        final_output_values
            .iter()
            .map(|output| ScalarBlockItem::Expression(output.value.clone())),
    );
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
        .chain(final_output_values.iter().map(|_| false))
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
        final_output_values,
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
            let place = derive_place(&target_node);
            ScalarAssignmentTarget {
                receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
                target: target.text().to_owned(),
                receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
                target_span: token_span(target),
                span: ByteSpan::new(
                    wosy_syntax::byte_span(&target_node).start,
                    scalar_place_target_span(&place).end,
                ),
                place,
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
    let output_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::OutputList)
        .expect("assignment output list");
    let values = output_list
        .children()
        .filter(|child| child.kind() == SyntaxKind::Expression)
        .map(|child| derive_expression(&child))
        .collect::<Vec<_>>();
    let value = values[0].clone();
    ScalarAssignment {
        receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
        target: target.text().to_owned(),
        receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
        target_span: token_span(&target),
        targets,
        value,
        values,
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
        SyntaxKind::ArrayLiteral => ScalarExpression::ArrayLiteral {
            elements: direct_nodes(&actual)
                .into_iter()
                .filter(|child| child.kind() == SyntaxKind::Expression)
                .map(|child| derive_expression(&child))
                .collect(),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Primary
            if actual.children_with_tokens().any(|element| {
                matches!(element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::ArrayLiteral)
            }) => ScalarExpression::ArrayLiteral {
            elements: Vec::new(),
            span: wosy_syntax::byte_span(&actual),
        },
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
                overload_selection: None,
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
                receiver: if identifiers.len() == 3 {
                    format!("{}.{}", identifiers[0].text(), identifiers[1].text())
                } else {
                    identifiers[0].text().to_owned()
                },
                name: identifiers.last().expect("qualified member name").text().to_owned(),
                enum_tag: None,
                receiver_span: token_span(&identifiers[0]),
                name_span: token_span(identifiers.last().expect("qualified member name")),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::FieldAccess => {
            if let Some(field) = actual
                .descendants()
                .find(|child| child.kind() == SyntaxKind::DereferencedField)
            {
                ScalarExpression::Dereference {
                    place: derive_place(&field),
                    span: wosy_syntax::byte_span(&actual),
                }
            } else {
                derive_element(&semantic_children(&actual)[0])
            }
        }
        SyntaxKind::IndexedPlace => ScalarExpression::IndexedRead {
            place: derive_place(&actual),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Dereference | SyntaxKind::DereferencedField => ScalarExpression::Dereference {
            place: derive_place(&actual),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::CheckedAddress => {
            let target = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::PlaceTarget)
                .expect("checked address target");
            ScalarExpression::CheckedAddress {
                mutability: if actual.children_with_tokens().any(
                    |element| matches!(element, NodeOrToken::Token(token) if token.text() == "!"),
                ) {
                    ScalarReferenceMutability::Mutable
                } else {
                    ScalarReferenceMutability::Shared
                },
                place: derive_place(&target),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::RawAddress => {
            let target = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::PlaceTarget)
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
        _ => {
            let children = semantic_children(&actual);
            children
                .first()
                .map(derive_element)
                .unwrap_or_else(|| panic!("expression grammar {:?}", actual.kind()))
        }
    }
}

fn derive_place(node: &CstNode) -> ScalarPlace {
    match node.kind() {
        SyntaxKind::PlaceTarget => {
            derive_place(&direct_nodes(node).into_iter().next().expect("place target"))
        }
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
            let dereference = parent
                .descendants()
                .find(|child| child.kind() == SyntaxKind::Dereference)
                .expect("dereference expression");
            ScalarPlace::Field {
                base: Box::new(derive_place(&dereference)),
                field: ScalarFieldReference::Unresolved {
                    name: field.text().to_owned(),
                    span: token_span(&field),
                },
                span: wosy_syntax::byte_span(node),
            }
        }
        SyntaxKind::Dereference => ScalarPlace::Dereference {
            pointer: Box::new(derive_expression(
                &direct_nodes(node)
                    .into_iter()
                    .next()
                    .expect("dereference pointer"),
            )),
            span: wosy_syntax::byte_span(node),
        },
        SyntaxKind::IndexedPlace => {
            let children = direct_nodes(node);
            let mut place = children
                .iter()
                .find(|child| child.kind() != SyntaxKind::IndexSuffix)
                .map(derive_place)
                .unwrap_or_else(|| {
                    let name =
                        direct_token(node, SyntaxKind::Identifier).expect("indexed place name");
                    ScalarPlace::Name {
                        name: name.text().to_owned(),
                        span: token_span(&name),
                    }
                });
            for suffix in children
                .iter()
                .filter(|child| child.kind() == SyntaxKind::IndexSuffix)
            {
                let index = suffix
                    .children()
                    .find(|child| child.kind() == SyntaxKind::Expression)
                    .expect("indexed place expression");
                let index_span = wosy_syntax::byte_span(&index);
                place = ScalarPlace::Index {
                    base: Box::new(place),
                    index: Box::new(derive_expression(&index)),
                    span: ByteSpan::new(
                        wosy_syntax::byte_span(node).start,
                        wosy_syntax::byte_span(&suffix).end,
                    ),
                    index_span,
                };
            }
            place
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
        | ScalarExpression::CheckedAddress { span, .. }
        | ScalarExpression::Dereference { span, .. }
        | ScalarExpression::IndexedRead { span, .. }
        | ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::ArrayLiteral { span, .. } => *span,
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
            for value in &assignment.values {
                validate_unit_if_position(value, false, source, diagnostics);
            }
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
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => {
            validate_unit_if_position_in_place(place, source, diagnostics)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            validate_unit_if_position_in_place(place, source, diagnostics)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                validate_unit_if_position(&field.value, false, source, diagnostics);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                validate_unit_if_position(element, false, source, diagnostics);
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
        ScalarPlace::Index { base, index, .. } => {
            validate_unit_if_position_in_place(base, source, diagnostics);
            validate_unit_if_position(index, false, source, diagnostics);
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
            ScalarItem::Namespace(namespace) => {
                validate_program_identifier_style(
                    program,
                    &namespace.binding,
                    namespace.binding_span,
                    &mut diagnostics,
                );
                (&namespace.binding, namespace.span, None)
            }
            ScalarItem::Extern(extern_decl) => {
                validate_program_identifier_style(
                    program,
                    &extern_decl.binding,
                    extern_decl.binding_span,
                    &mut diagnostics,
                );
                (&extern_decl.binding, extern_decl.span, None)
            }
            ScalarItem::Binding(binding) => {
                for receiver in &binding.receivers {
                    validate_program_identifier_style(
                        program,
                        &receiver.name,
                        receiver.name_span,
                        &mut diagnostics,
                    );
                }
                (&binding.name, binding.span, Some(&binding.declared_type))
            }
            ScalarItem::Function(function) => {
                validate_program_identifier_style(
                    program,
                    &function.name,
                    function.name_span,
                    &mut diagnostics,
                );
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
                if let ScalarItem::Function(function) = item {
                    if !function.overload_arms.is_empty() {
                        for arm in &function.overload_arms {
                            if arm.generic_parameters.is_empty() {
                                validate_type(program, &arm.signature, arm.span, &mut diagnostics);
                            } else {
                                validate_generic_type(
                                    program,
                                    &arm.signature,
                                    arm.span,
                                    &arm.generic_parameters,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else if function.generic_parameters.is_empty() {
                        validate_type(program, ty, span, &mut diagnostics);
                    } else {
                        validate_generic_type(
                            program,
                            ty,
                            span,
                            &function.generic_parameters,
                            &mut diagnostics,
                        );
                    }
                } else {
                    validate_type(program, ty, span, &mut diagnostics);
                }
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
                if binding.is_allocation {
                    for receiver in &binding.receivers {
                        let Some(length) = &receiver.allocation_length else {
                            continue;
                        };
                        let actual = expression_type_expected(
                            length,
                            &ScalarType::U64,
                            &declarations,
                            &BTreeSet::new(),
                            &BTreeMap::new(),
                            program,
                            &mut diagnostics,
                            false,
                        );
                        expect_type(
                            program,
                            &ScalarType::U64,
                            &actual,
                            receiver.span,
                            &mut diagnostics,
                        );
                    }
                    continue;
                }
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
                if !function.overload_arms.is_empty() {
                    continue;
                }
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
                    validate_program_identifier_style(
                        program,
                        name,
                        function.parameter_spans[index],
                        &mut diagnostics,
                    );
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
                if outputs.outputs.len() > 1 && !function.body.final_output_values.is_empty() {
                    let final_start =
                        function.body.items.len() - function.body.final_output_values.len();
                    for item in &function.body.items[..final_start] {
                        let _ = match item {
                            ScalarBlockItem::Expression(
                                expression @ (ScalarExpression::If { .. }
                                | ScalarExpression::UnitIf { .. }),
                            ) => expression_type_expected(
                                expression,
                                &ScalarType::Unit,
                                &scope,
                                &visible_names,
                                &folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            ),
                            _ => block_item_type(
                                item,
                                &mut scope,
                                &mut visible_names,
                                &mut folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            ),
                        };
                    }
                    if function.body.final_output_values.len() == 1
                        && matches!(
                            function.body.final_output_values[0].value,
                            ScalarExpression::If { .. }
                        )
                    {
                        validate_conditional_output_arity(
                            &function.body,
                            outputs.outputs.len(),
                            program,
                            &mut diagnostics,
                        );
                    }
                    if function.body.final_output_values.len() == 1
                        && matches!(
                            &function.body.final_output_values[0].value,
                            ScalarExpression::Call { .. }
                        )
                    {
                        let value = &function.body.final_output_values[0];
                        if let Some(actual_outputs) =
                            call_output_sequence(&value.value, &scope, program)
                        {
                            expression_type_expected(
                                &value.value,
                                &actual_outputs
                                    .outputs
                                    .first()
                                    .map(|output| &output.ty)
                                    .expect("multi-output final call has an output"),
                                &scope,
                                &visible_names,
                                &folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            );
                            if actual_outputs.outputs.len() != outputs.outputs.len() {
                                diagnostics.push(diagnostic(
                                    program,
                                    "B0004",
                                    "function output arity does not match its final output list",
                                    value.span,
                                ));
                            }
                            for (actual, expected) in
                                actual_outputs.outputs.iter().zip(&outputs.outputs)
                            {
                                expect_type(
                                    program,
                                    &expected.ty,
                                    &actual.ty,
                                    value.span,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else {
                        if function.body.final_output_values.len() > 1
                            && function.body.final_output_values.len() != outputs.outputs.len()
                        {
                            diagnostics.push(diagnostic(
                                program,
                                "B0004",
                                "function output arity does not match its final output list",
                                function.body.span,
                            ));
                        }
                        for (value, output) in function
                            .body
                            .final_output_values
                            .iter()
                            .zip(&outputs.outputs)
                        {
                            let actual = expression_type_expected(
                                &value.value,
                                &output.ty,
                                &scope,
                                &visible_names,
                                &folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            );
                            expect_type(program, &output.ty, &actual, value.span, &mut diagnostics);
                        }
                    }
                } else if let Some(output) = outputs.outputs.first() {
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
                } else {
                    let actual = block_type(
                        &function.body,
                        &scope,
                        &visible_names,
                        &folded_names,
                        program,
                        &mut diagnostics,
                        false,
                    );
                    expect_type(
                        program,
                        &ScalarType::Unit,
                        &actual,
                        function.body.span,
                        &mut diagnostics,
                    );
                }
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
                if binding.is_allocation {
                    for receiver in &binding.receivers {
                        if let Some(length) = &receiver.allocation_length {
                            self.expression(length, visible);
                        }
                    }
                } else {
                    self.expression(&binding.value, visible);
                }
                let id = self.declarations.len();
                visible.insert(binding.name.clone(), id);
                self.declarations.push(binding.span);
            }
            ScalarBlockItem::Expression(expression) => self.expression(expression, visible),
            ScalarBlockItem::Assignment(assignment) => {
                self.expression(&assignment.value, visible);
                for value in &assignment.values {
                    self.expression(value, visible);
                }
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
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. } => self.place(place, visible),
            ScalarExpression::IndexedRead { place, .. } => self.place(place, visible),
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, visible);
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element, visible);
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
            ScalarPlace::Index { base, index, .. } => {
                self.place(base, visible);
                self.expression(index, visible);
            }
        }
    }

    fn use_name(&mut self, name: &str, visible: &BTreeMap<String, usize>) {
        if let Some(id) = visible.get(name) {
            self.uses.insert(*id);
        }
    }

    fn unused(self) -> Vec<ByteSpan> {
        self.declarations
            .iter()
            .enumerate()
            .filter_map(|(id, span)| {
                let used = self.uses.contains(&id)
                    || self
                        .declarations
                        .iter()
                        .enumerate()
                        .any(|(other_id, other)| other == span && self.uses.contains(&other_id));
                (!used).then_some(*span)
            })
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
        ScalarType::Struct(_) | ScalarType::Enum(_) => {}
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
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            validate_type(program, inner, span, diagnostics)
        }
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
            validate_type(program, element, span, diagnostics)
        }
        ScalarType::Error => {}
    }
}

fn validate_generic_type(
    program: &ScalarProgram,
    ty: &ScalarType,
    span: ByteSpan,
    generic_parameters: &[ScalarGenericParameter],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { name, .. }
            if generic_parameters
                .iter()
                .any(|parameter| parameter.name == *name) => {}
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            validate_generic_type(program, inner, span, generic_parameters, diagnostics)
        }
        ScalarType::Array { element, .. } => {
            validate_generic_type(program, element, span, generic_parameters, diagnostics)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_generic_type(
                    program,
                    &output.ty,
                    output.span,
                    generic_parameters,
                    diagnostics,
                );
            }
            for parameter in parameters {
                validate_generic_type(program, parameter, span, generic_parameters, diagnostics);
            }
        }
        _ => validate_type(program, ty, span, diagnostics),
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
            for receiver in &binding.receivers {
                validate_program_identifier_style(
                    program,
                    &receiver.name,
                    receiver.name_span,
                    diagnostics,
                );
            }
            if binding.is_allocation {
                for receiver in &binding.receivers {
                    let Some(length) = &receiver.allocation_length else {
                        continue;
                    };
                    let actual = expression_type_expected(
                        length,
                        &ScalarType::U64,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                    expect_type(
                        program,
                        &ScalarType::U64,
                        &actual,
                        receiver.span,
                        diagnostics,
                    );
                }
                if declare_program_name(
                    program,
                    &binding.name,
                    binding.span,
                    visible_names,
                    folded_names,
                    diagnostics,
                ) {
                    for receiver in &binding.receivers {
                        scope.insert(receiver.name.clone(), receiver.ty.clone());
                    }
                }
                return ScalarType::Unit;
            }
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
        ScalarBlockItem::Expression(expression) => expression_type(
            expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
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
    let mut inferred_names = visible_names.clone();
    let mut inferred_folded_names = folded_names.clone();
    let expected = assignment
        .targets
        .iter()
        .map(|target| assignment_target_type(target, scope, program, diagnostics))
        .collect::<Vec<_>>();
    validate_assignment_target_distinctness(assignment, program, diagnostics);
    let outputs = assignment
        .values
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let actual = match expected.get(position).and_then(Option::as_ref) {
                Some(expected) => expression_type_expected(
                    value,
                    expected,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                ),
                None => expression_type(
                    value,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                ),
            };
            match call_output_sequence(value, scope, program) {
                Some(outputs) => Some(outputs.outputs),
                None if matches!(value, ScalarExpression::Call { .. }) => None,
                None => Some(vec![ScalarOutput {
                    ty: actual,
                    span: expression_span(value),
                }]),
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|outputs| ScalarOutputSequence {
            outputs: outputs.into_iter().flatten().collect(),
            span: assignment.span,
        });
    if let Some(outputs) = outputs {
        if assignment.targets.len() > outputs.outputs.len() {
            diagnostics.push(diagnostic(
                program,
                "B0004",
                "call has fewer outputs than assignment targets",
                assignment.span,
            ));
        }
        for ((index, (target, expected)), output) in assignment
            .targets
            .iter()
            .zip(&expected)
            .enumerate()
            .zip(&outputs.outputs)
        {
            if let Some(expected) = expected {
                expect_type(
                    program,
                    expected,
                    &output.ty,
                    if index == 0 {
                        assignment.span
                    } else {
                        target.target_span
                    },
                    diagnostics,
                );
            } else if let ScalarPlace::Name { name, span } = &target.place {
                validate_program_identifier_style(program, name, *span, diagnostics);
                if declare_program_name(
                    program,
                    name,
                    *span,
                    &mut inferred_names,
                    &mut inferred_folded_names,
                    diagnostics,
                ) {
                    scope.insert(name.clone(), output.ty.clone());
                }
            }
        }
    }
    ScalarType::Unit
}

fn assignment_target_type(
    target: &ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> Option<ScalarType> {
    match &target.place {
        ScalarPlace::Name { name, span } => {
            let expected = scope.get(name).cloned();
            if expected.is_some() && is_const_binding_name(name) {
                diagnostics.push(diagnostic(
                    program,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    *span,
                ));
            }
            expected
        }
        place => Some(writable_place_type(
            place,
            target.span,
            scope,
            program,
            diagnostics,
        )),
    }
}

fn validate_assignment_target_distinctness(
    assignment: &ScalarAssignment,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (index, target) in assignment.targets.iter().enumerate() {
        for previous in &assignment.targets[..index] {
            if scalar_place_identity(&target.place) == scalar_place_identity(&previous.place) {
                diagnostics.push(diagnostic(
                    program,
                    "B0002",
                    "duplicate assignment target",
                    target.target_span,
                ));
            } else if place_contains_dynamic_index(&target.place)
                || place_contains_dynamic_index(&previous.place)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0002",
                    "assignment targets require a distinctness proof",
                    target.target_span,
                ));
            }
        }
    }
}

fn place_contains_dynamic_index(place: &ScalarPlace) -> bool {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => false,
        ScalarPlace::Field { base, .. } => place_contains_dynamic_index(base),
        ScalarPlace::Index { base, index, .. } => {
            place_contains_dynamic_index(base)
                || !matches!(index.as_ref(), ScalarExpression::Integer { .. })
        }
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
        ScalarExpression::CheckedAddress {
            mutability,
            place,
            span,
        } => checked_address_type(*mutability, place, *span, scope, program, diagnostics),
        ScalarExpression::Dereference { place, span } => {
            checked_dereference_type(place, *span, scope, program, diagnostics)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            place_type(place, scope, program, diagnostics)
        }
        ScalarExpression::StructLiteral { .. } => ScalarType::Error,
        ScalarExpression::ArrayLiteral { span, .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "array literal requires a fixed array context",
                *span,
            ));
            ScalarType::Error
        }
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
            enum_tag,
            name_span,
            span,
            ..
        } => match program
            .enums
            .iter()
            .find(|enumeration| enumeration.name == *receiver)
        {
            Some(enumeration) => {
                if enum_tag.is_some() {
                    ScalarType::Enum(enumeration.id.clone())
                } else {
                    diagnostics.push(diagnostic(
                        program,
                        "M0002",
                        "unknown enum variant",
                        *name_span,
                    ));
                    ScalarType::Error
                }
            }
            None => field_type(
                scope.get(receiver),
                name,
                *name_span,
                *span,
                program,
                diagnostics,
            ),
        },
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
            let right_type = if comparison {
                expression_type_expected(
                    right,
                    &left_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                )
            } else {
                expression_type(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                )
            };
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
            name_span,
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
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "alloc" | "free" | "system_panic")
            {
                return type_core_memory(
                    name,
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
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return type_core_int_conversion(
                    name,
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
                return type_core_float_conversion(
                    name,
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
            if receiver.as_deref() == Some("core") && matches!(name.as_str(), "offset" | "load") {
                return type_core_raw_memory(
                    name,
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
            if let Some(overload) = program.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if receiver.is_none()
                        && function.name == *name
                        && !function.overload_arms.is_empty() =>
                {
                    Some(function)
                }
                _ => None,
            }) {
                let argument_types = arguments
                    .iter()
                    .map(|argument| {
                        expression_type(
                            argument,
                            scope,
                            visible_names,
                            folded_names,
                            program,
                            diagnostics,
                            unsafe_context,
                        )
                    })
                    .collect::<Vec<_>>();
                let overload_arguments = arguments
                    .iter()
                    .zip(&argument_types)
                    .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                    .collect::<Vec<_>>();
                let callable = match resolve_overload_candidate(overload, &overload_arguments, None)
                {
                    Ok((callable, _)) => callable,
                    Err(message) => {
                        diagnostics.push(diagnostic(program, "B0004", message, *name_span));
                        return ScalarType::Error;
                    }
                };
                let ScalarType::Callable {
                    outputs,
                    parameters,
                } = callable
                else {
                    unreachable!("overload arms are callable")
                };
                for (argument, parameter) in arguments.iter().zip(&parameters) {
                    let actual = expression_type_expected(
                        argument,
                        parameter,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                    expect_type(
                        program,
                        parameter,
                        &actual,
                        expression_span(argument),
                        diagnostics,
                    );
                }
                return scalar_call_result(&outputs);
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
            let Some(callable) = callable else {
                diagnostics.push(diagnostic(program, "B0001", "unknown callable name", *span));
                return ScalarType::Error;
            };
            let extern_callable = program.items.iter().any(|item| {
                matches!(
                    item,
                    ScalarItem::Extern(extern_decl)
                        if receiver.as_deref() == Some(extern_decl.binding.as_str())
                            && extern_decl.functions.iter().any(|function| function.name == *name)
                )
            });
            if extern_callable && !type_arguments.is_empty() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "generic argument arity does not match callable declaration",
                    *span,
                ));
                return ScalarType::Error;
            }
            let generic_parameters = (receiver.is_none()).then(|| {
                program.items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == *name => {
                        Some(&function.generic_parameters)
                    }
                    _ => None,
                })
            });
            let callable = if let Some(Some(generic_parameters)) = generic_parameters {
                let Some(callable) =
                    substitute_generic_callable(&callable, generic_parameters, type_arguments)
                else {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "generic argument arity does not match callable declaration",
                        *span,
                    ));
                    return ScalarType::Error;
                };
                callable
            } else {
                callable
            };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
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

fn validate_conditional_output_arity(
    block: &ScalarBlock,
    output_count: usize,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if block.final_output_values.len() == output_count {
        for output in &block.final_output_values {
            if let ScalarExpression::If {
                then_branch,
                else_branch,
                ..
            } = &output.value
            {
                validate_conditional_output_arity(then_branch, output_count, program, diagnostics);
                validate_conditional_output_arity(else_branch, output_count, program, diagnostics);
            }
        }
        return;
    }
    if let [output] = block.final_output_values.as_slice() {
        if let ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } = &output.value
        {
            validate_conditional_output_arity(then_branch, output_count, program, diagnostics);
            validate_conditional_output_arity(else_branch, output_count, program, diagnostics);
            return;
        }
    }
    diagnostics.push(diagnostic(
        program,
        "B0004",
        "function output arity does not match its final output list",
        block
            .final_output_values
            .first()
            .map_or(block.span, |output| output.span),
    ));
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
    if is_error_type(expected) {
        return ScalarType::Error;
    }
    if let ScalarExpression::Call {
        receiver: None,
        name,
        name_span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        if let Some(overload) = program.items.iter().find_map(|item| match item {
            ScalarItem::Function(function)
                if function.name == *name && !function.overload_arms.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        }) {
            if let Some(type_argument) = type_arguments.first() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "overload calls do not accept explicit type arguments",
                    type_argument.span,
                ));
                return ScalarType::Error;
            }
            let argument_types = arguments
                .iter()
                .map(|argument| {
                    expression_type(
                        argument,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                })
                .collect::<Vec<_>>();
            let overload_arguments = arguments
                .iter()
                .zip(&argument_types)
                .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                .collect::<Vec<_>>();
            let callable =
                match resolve_overload_candidate(overload, &overload_arguments, Some(expected)) {
                    Ok((callable, _)) => callable,
                    Err(message) => {
                        diagnostics.push(diagnostic(program, "B0004", message, *name_span));
                        return ScalarType::Error;
                    }
                };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
            else {
                unreachable!("overload arms are callable")
            };
            for (argument, parameter) in arguments.iter().zip(&parameters) {
                let actual = expression_type_expected(
                    argument,
                    parameter,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    parameter,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            return scalar_call_result(&outputs);
        }
    }
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
            && matches!(
                expected,
                ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. }
            )
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
                    } else if comparison {
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
        if let ScalarExpression::ArrayLiteral { elements, span } = expression {
            let ScalarType::Array {
                element, length, ..
            } = expected
            else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "array literal requires a fixed array context",
                    *span,
                ));
                return ScalarType::Error;
            };
            if elements.len() as u64 != *length {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "array literal element count does not match fixed array length",
                    *span,
                ));
            }
            for value in elements {
                let actual = expression_type_expected(
                    value,
                    element,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    element,
                    &actual,
                    expression_span(value),
                    diagnostics,
                );
            }
            return expected.clone();
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
    fn derives_zero_payload_enum_tags_and_equality() {
        let result = validate_text(
            "%%start\nenum Status { first; second; third; }\nStatus value = Status::second;\nbool equal = value == Status::second;\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.program.enums[0].variants[0].tag, 0);
        assert_eq!(result.program.enums[0].variants[1].tag, 1);
        assert_eq!(result.program.enums[0].variants[2].tag, 2);
    }

    #[test]
    fn validates_mutable_typed_place_writes_and_assignment_distinctness() {
        let valid = validate_text(
            "%%start\nstruct Record { u8 value; u8[3] bytes; }\nu8[3] values = [1, 2, 3];\nRecord holder = { .value = 4; .bytes = [5, 6, 7]; };\n*!u8 writer = &!values[1];\nvalues[0] = 8;\nholder.value = 9;\n*writer = 10;\nholder.bytes[2] = 11;\nvalues[0], values[2] = 12, 13;\n%%end",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);

        let dynamic = validate_text(
            "%%start\nu8[2] values = [1, 2];\nu64 index = 0;\nvalues[index], values[1] = 3, 4;\n%%end",
        );
        assert!(dynamic.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0002"
                && diagnostic.message == "assignment targets require a distinctness proof"
        }));

        let shared = validate_text(
            "%%start\nu8[2] values = [1, 2];\n*u8 shared = &values[0];\n*shared = 3;\n%%end",
        );
        assert!(shared.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message
                    == "assignment dereference requires a mutable checked reference"
        }));
    }

    #[test]
    fn validates_runtime_array_callable_transport() {
        let validation = validate_text(
            "%%start\nu8[]() make = fn { u64 length = 1; u8[length] bytes; bytes };\nu8[](u8[]) relay = fn(values) { values };\nu8[] value = make();\nu8[] alias = relay(value);\n%%end",
        );
        assert!(
            validation.diagnostics.is_empty(),
            "diagnostics: {:#?}\nprogram: {:#?}",
            validation.diagnostics,
            validation.program
        );
    }

    #[test]
    fn validates_runtime_array_binding_move_and_top_level_materialization() {
        let validation = validate_text(
            "%%start\nu64 length = 1;\nu8[length] first;\nu8[] second = first;\nsecond[0] = 7;\nu8 value = second[0];\n%%end",
        );
        assert!(
            validation.diagnostics.is_empty(),
            "diagnostics: {:#?}\nprogram: {:#?}",
            validation.diagnostics,
            validation.program
        );
    }

    #[test]
    fn enforces_canonical_assignment_targets_and_writable_place_paths() {
        for text in [
            "%%start\nu8 value = 0;\nvalue, value = 1, 2;\n%%end",
            "%%start\nstruct Record { u8 value; }\nRecord holder = { .value = 0; };\nholder.value, holder.value = 1, 2;\n%%end",
            "%%start\nu8[2] items = [0, 0];\nitems[0], items[0] = 1, 2;\n%%end",
        ] {
            let result = validate_text(text);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0002" && diagnostic.message == "duplicate assignment target"
            }));
        }
        for text in [
            "%%start\nstruct Record { u8 first; u8 second; }\nRecord holder = { .first = 0; .second = 0; };\nholder.first, holder.second = 1, 2;\n%%end",
            "%%start\nu8[2] items = [0, 0];\nitems[0], items[1] = 1, 2;\n%%end",
        ] {
            let result = validate_text(text);
            assert!(result.diagnostics.is_empty(), "{text}: {:?}", result.diagnostics);
        }
        for text in [
            "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[index], items[0] = 1, 2;\n%%end",
            "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[0], items[index] = 1, 2;\n%%end",
        ] {
            let result = validate_text(text);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0002"
                    && diagnostic.message == "assignment targets require a distinctness proof"
            }));
        }

        let valid = validate_text(
            "%%start\nstruct Record { u8 value; u8[2] bytes; }\nRecord holder = { .value = 0; .bytes = [0, 0]; };\n*!Record writer = &!holder;\n(*writer).value = 1;\n(*writer).bytes[0] = 2;\n%%end",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);

        for target in ["(*raw).value", "(*raw).bytes[0]"] {
            let text = format!(
                "%%start\nstruct Record {{ u8 value; u8[2] bytes; }}\nRecord holder = {{ .value = 0; .bytes = [0, 0]; }};\nunsafe {{ *?Record raw = &?holder; {target} = 1; }};\n%%end"
            );
            let result = validate_text(&text);
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.message
                        == "assignment through raw pointer dereference is not supported"
                })
                .expect("raw write diagnostic");
            let start = text.find(target).expect("raw assignment target") as u32;
            assert_eq!(diagnostic.code, "B0003");
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + target.len() as u32)
            );
        }

        for text in [
            "%%start\nu8[2] values = [1, 2];\n*!(u8[2]) writer = &!values;\n*writer = [3, 4];\n%%end",
            "%%start\nstruct Record { u8[2] bytes; }\nRecord holder = { .bytes = [1, 2]; };\nholder.bytes = [3, 4];\n%%end",
        ] {
            let result = validate_text(text);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "assignment place requires a scalar leaf type"
            }));
        }
    }

    #[test]
    fn enforces_assignment_target_and_writable_path_rules_in_projects() {
        let validate_project_text = |text: &str| {
            let main_source = module_source("src/main.w");
            let main = module_from_text(main_source.clone(), text);
            validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(main, Vec::new())],
                vec![main_source],
            ))
        };
        let duplicates = validate_project_text(
            "%%start\nstruct Record { u8 first; u8 second; }\nu8 value = 0;\nRecord record = { .first = 0; .second = 0; };\nu8[2] items = [0, 0];\nvalue, value = 1, 2;\nrecord.first, record.first = 1, 2;\nitems[0], items[0] = 1, 2;\n%%end",
        );
        assert_eq!(
            duplicates
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message == "duplicate assignment target")
                .count(),
            3,
            "{:?}",
            duplicates.diagnostics
        );
        let distinct = validate_project_text(
            "%%start\nstruct Record { u8 first; u8 second; }\nRecord record = { .first = 0; .second = 0; };\nu8[2] items = [0, 0];\nrecord.first, record.second = 1, 2;\nitems[0], items[1] = 1, 2;\n%%end",
        );
        assert!(
            distinct.diagnostics.is_empty(),
            "{:?}",
            distinct.diagnostics
        );
        for text in [
            "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[index], items[0] = 1, 2;\n%%end",
            "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[0], items[index] = 1, 2;\n%%end",
        ] {
            let result = validate_project_text(text);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0002"
                    && diagnostic.message == "assignment targets require a distinctness proof"
            }));
        }
        let valid = validate_project_text(
            "%%start\nstruct Record { u8 value; u8[2] bytes; }\nRecord holder = { .value = 0; .bytes = [0, 0]; };\n*!Record writer = &!holder;\n(*writer).value = 1;\n(*writer).bytes[0] = 2;\n%%end",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let raw_text =
            "%%start\nstruct Record { u8 value; u8[2] bytes; }\nRecord record = { .value = 0; .bytes = [0, 0]; };\nunsafe { *?Record raw = &?record; (*raw).value = 1; (*raw).bytes[0] = 2; };\n%%end";
        let raw = validate_project_text(raw_text);
        for target in ["(*raw).value", "(*raw).bytes[0]"] {
            let start = raw_text.find(target).expect("raw assignment target") as u32;
            let diagnostic = raw
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.message
                        == "assignment through raw pointer dereference is not supported"
                        && diagnostic.labels[0].span.range
                            == ByteSpan::new(start, start + target.len() as u32)
                })
                .expect("raw write diagnostic");
            assert_eq!(diagnostic.code, "B0003");
        }
        let aggregate = validate_project_text(
            "%%start\nu8[2] values = [1, 2];\n*!(u8[2]) writer = &!values;\n*writer = [3, 4];\n%%end",
        );
        assert!(aggregate.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "assignment place requires a scalar leaf type"
        }));
    }

    #[test]
    fn derives_and_types_fixed_array_index_places_and_addresses() {
        let text = "%%start\nstruct Record { u8[2] bytes; }\nu8[2][2] matrix = [[1, 2], [3, 4]];\nu64 row = 1;\nu64 column = 0;\nRecord holder = { .bytes = [5, 6]; };\nu8 nested = matrix[row][column];\nu8 field = holder.bytes[column];\n*u8 shared = &matrix[row][column];\n*!u8 mutable = &!holder.bytes[column];\nunsafe { *?u8 raw = &?matrix[row][column]; };\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(nested) = &result.program.items[4] else {
            panic!("nested indexed read")
        };
        let ScalarExpression::IndexedRead {
            place: ScalarPlace::Index {
                base, index_span, ..
            },
            ..
        } = &nested.value
        else {
            panic!("nested indexed read place")
        };
        let index_start = text.find("column];").expect("column") as u32;
        assert_eq!(*index_span, ByteSpan::new(index_start, index_start + 6));
        assert!(matches!(base.as_ref(), ScalarPlace::Index { .. }));
        for item in [6, 7] {
            let ScalarItem::Binding(binding) = &result.program.items[item] else {
                panic!("indexed address binding")
            };
            assert!(matches!(
                binding.value,
                ScalarExpression::CheckedAddress {
                    place: ScalarPlace::Index { .. },
                    ..
                }
            ));
        }
        let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            &result.program.items[8]
        else {
            panic!("unsafe indexed raw address")
        };
        assert!(matches!(
            block.items[0],
            ScalarBlockItem::LocalBinding(ScalarBinding {
                value: ScalarExpression::RawAddress {
                    place: ScalarPlace::Index { .. },
                    ..
                },
                ..
            })
        ));

        let invalid = validate_text("%%start\nu8[1] bytes = [1];\ni32 index = 0;\nu8 value = bytes[index];\nu8 wrong = index[0];\nu8[0] empty = [];\nu8 accepted = empty[0];\n%%end");
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message
                    == "expression type does not match expected type")
        );
        assert!(invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "indexed place requires a fixed array"));
        assert!(invalid
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.message != "index is outside the array"));
    }

    #[test]
    fn derives_pointer_struct_layouts_from_the_target() {
        let text = "%%start\nstruct Pair {\n\t*?u8 address;\n\tu32 count;\n}\n%%end";
        let parsed = parse_source(source(), text.to_owned(), &[]);

        let wasm = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
        let native =
            derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);

        assert_eq!(
            wasm.program.structs[0].layout,
            Some(ScalarLayout {
                size: 8,
                alignment: 4
            })
        );
        assert_eq!(wasm.program.structs[0].fields[1].offset, Some(4));
        assert_eq!(
            native.program.structs[0].layout,
            Some(ScalarLayout {
                size: 16,
                alignment: 8
            })
        );
        assert_eq!(native.program.structs[0].fields[1].offset, Some(8));
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
    fn validates_public_read_into_signature_and_hides_preview1_read_details() {
        let preview_source = module_source("src/wasi/preview1.w");
        let preview = module_from_text(
            preview_source.clone(),
            "%%start\nstruct _ReadIovec { *?u8 data; u32 length; }\nstruct _Nread { u32 value; }\n_wasi = extern wasm \"wasi_snapshot_preview1\" { unsafe i32(i32, *?_ReadIovec, i32, *?_Nread) _fd_read; };\n(u64, bool)(*?u8, u64) _fd_read_once = fn(destination, capacity) { _ReadIovec _iovec = { .data = destination; .length = core.int_trunc<u32>(capacity); }; _Nread _byte_count = { .value = 0; }; i32 _result = unsafe { _wasi._fd_read(0, &?_iovec, 1, &?_byte_count) }; u64 reported = 0; bool complete = false; if (_result == 0) { reported = core.int_extend<u64>(_byte_count.value); complete = true; } else { reported = 0; complete = false; }; reported, complete };\n(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) { _fd_read_once(destination, capacity) };\n%%end",
        );
        let bootstrap_source = module_source("src/bootstrap.w");
        let bootstrap = module_from_text(
            bootstrap_source.clone(),
            "%%start\npreview1 = namespace std \"wasi/preview1.w\";\n(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) { preview1.read_into(destination, capacity) };\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nstd = namespace std \"bootstrap.w\";\n*?u8 destination = null;\nu64 capacity = 4;\nu64 count = 0;\nbool complete = false;\ncount, complete = std.read_into(destination, capacity);\n%%end",
        );
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main,
                    vec![ScalarNamespaceBinding {
                        binding: "std".to_owned(),
                        target: bootstrap_source.clone(),
                        span: ByteSpan::new(0, 0),
                    }],
                ),
                ScalarModule::from_program(
                    bootstrap,
                    vec![ScalarNamespaceBinding {
                        binding: "preview1".to_owned(),
                        target: preview_source.clone(),
                        span: ByteSpan::new(0, 0),
                    }],
                ),
                ScalarModule::from_program(preview, Vec::new()),
            ],
            vec![main_source, bootstrap_source, preview_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let ScalarItem::Binding(count) = &validation.project.modules[0].items[3] else {
            panic!("read count binding")
        };
        assert_eq!(count.declared_type, ScalarType::U64);
        let ScalarItem::Binding(complete) = &validation.project.modules[0].items[4] else {
            panic!("read complete binding")
        };
        assert_eq!(complete.declared_type, ScalarType::Bool);

        assert!(validation.project.modules[1]
            .members
            .contains_key("read_into"));
        assert!(!validation.project.modules[1]
            .members
            .contains_key("fd_read_once"));
        assert!(!validation.project.modules[2]
            .members
            .contains_key("_fd_read"));
        assert!(!validation.project.modules[2]
            .members
            .contains_key("_fd_read_once"));
    }

    #[test]
    fn rejects_undeclared_extern_type_arguments_and_preserves_valid_calls() {
        let text = "%%start\nenv = extern wasm \"helper\" { (u64, bool)() read; };\nu64 first, bool second = env.read<u32>();\n%%end";
        let rejected = validate_text(text);
        assert_eq!(rejected.diagnostics.len(), 1, "{:?}", rejected.diagnostics);
        let diagnostic = &rejected.diagnostics[0];
        assert_eq!(diagnostic.code, "B0004");
        assert_eq!(
            diagnostic.message,
            "generic argument arity does not match callable declaration"
        );
        let start = text.find("env.read<u32>()").expect("extern call") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + 15)
        );

        let ordinary = validate_text(
            "%%start\nenv = extern wasm \"helper\" { u64() read; };\nu64 value = env.read();\n%%end",
        );
        assert!(
            ordinary.diagnostics.is_empty(),
            "{:?}",
            ordinary.diagnostics
        );

        let generic = validate_text(
            "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\nu64 result = identity<u64>(1);\n%%end",
        );
        assert!(generic.diagnostics.is_empty(), "{:?}", generic.diagnostics);
    }

    #[test]
    fn rejects_undeclared_project_extern_type_arguments_at_call_span() {
        let main_source = module_source("src/main.w");
        let library_source = module_source("src/library.w");
        let main_text = "%%start\nlibrary = namespace app \"src/library.w\";\nu64 first, bool second = library.read<u32>();\n%%end";
        let main = module_from_text(main_source.clone(), main_text);
        let library = module_from_text(
            library_source.clone(),
            "%%start\nenv = extern wasm \"helper\" { (u64, bool)() read; };\n%%end",
        );
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main,
                    vec![ScalarNamespaceBinding {
                        binding: "library".to_owned(),
                        target: library_source.clone(),
                        span: ByteSpan::new(8, 15),
                    }],
                ),
                ScalarModule::new(library_source, library.items, Vec::new()),
            ],
            vec![main_source.clone()],
        ));
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        let diagnostic = &validation.diagnostics[0];
        assert_eq!(diagnostic.code, "B0004");
        assert_eq!(
            diagnostic.message,
            "generic argument arity does not match callable declaration"
        );
        assert_eq!(diagnostic.labels[0].span.source, main_source);
        let start = main_text.find("library.read<u32>()").expect("extern call") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + 19)
        );
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
    fn infers_assignment_targets_and_rejects_mismatched_assignment_type() {
        let inferred =
            validate_text("%%start\ni32(i32) f = fn(value) { inferred = value; inferred };\n%%end");
        assert!(
            inferred.diagnostics.is_empty(),
            "{:?}",
            inferred.diagnostics
        );

        let mismatch = validate_text(
            "%%start\ni32(i32) f = fn(value) { i32 local = value; local = true; local };\n%%end",
        );
        assert!(mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
    }

    #[test]
    fn validates_ordered_assignment_outputs_and_inferred_project_targets() {
        let valid = validate_text(
            "%%start\n(i32, bool)() pair = fn { 1 };\ni32() result = fn { first, second = pair(); first };\n%%end",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let ScalarItem::Function(result) = &valid.program.items[1] else {
            panic!("ordered assignment")
        };
        let ScalarBlockItem::Assignment(assignment) = &result.body.items[0] else {
            panic!("ordered assignment")
        };
        assert_eq!(
            assignment
                .targets
                .iter()
                .map(|target| &target.target)
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );

        for text in [
            "%%start\n(i32, bool)() pair = fn { 1 };\nfirst, second, third = pair();\n%%end",
            "%%start\n(i32, bool)() pair = fn { 1 };\nbool first = false;\ni32 second = 0;\nfirst, second = pair();\n%%end",
            "%%start\ni32 value = 0;\nvalue, value = 1;\n%%end",
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid.diagnostics.iter().any(|diagnostic| {
                    matches!(diagnostic.code.as_str(), "B0002" | "B0003" | "B0004")
                }),
                "missing assignment diagnostic: {:?}",
                invalid.diagnostics
            );
        }

        let child_source = module_source("src/child.w");
        let child = module_from_text(
            child_source.clone(),
            "%%start\ni32() value = fn { 1 };\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\ni32() result = fn { inferred = child.value(); 0 };\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source,
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
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
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
    fn derives_u32_literal_context_for_generic_integer_extension() {
        let text = "%%start\nu64 result = core.int_extend<u64>(4294967295);\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarItem::Binding(binding) = &single.program.items[0] else {
            panic!("extension binding");
        };
        assert_eq!(binding.declared_type, ScalarType::U64);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }

    #[test]
    fn resolves_member_expression_sources_for_generic_integer_conversions() {
        let text = "%%start\nstruct utf8 {\n\tu64 length;\n}\nu32(utf8) truncate = fn(text) { core.int_trunc<u32>(text.length) };\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }

    #[test]
    fn validates_generic_integer_conversions_in_single_file_and_project() {
        let text = "%%start\ni8 signed_eight = 1;\ni16 signed_sixteen = core.int_extend<i16>(signed_eight);\ni32 signed_thirty_two = core.int_extend<i32>(signed_sixteen);\ni64 signed_sixty_four = core.int_extend<i64>(signed_thirty_two);\ni128 signed_full = core.int_extend<i128>(signed_sixty_four);\nu8 unsigned_eight = core.int_trunc<u8>(signed_full);\nu16 unsigned_sixteen = core.int_extend<u16>(unsigned_eight);\nu32 unsigned_thirty_two = core.int_extend<u32>(unsigned_sixteen);\nu64 unsigned_sixty_four = core.int_extend<u64>(unsigned_thirty_two);\nu128 unsigned_full = core.int_extend<u128>(unsigned_sixty_four);\nu8 narrow_unsigned = core.int_trunc<u8>(unsigned_full);\ni8 narrow_signed = core.int_trunc<i8>(unsigned_full);\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

        for text in [
            "%%start\nu32 source = 1;\nu32 value = core.int_trunc<u32>(source);\n%%end",
            "%%start\nu32 source = 1;\nu64 value = core.int_trunc<u64>(source);\n%%end",
            "%%start\nu64 source = 1;\nu32 value = core.int_extend<u32>(source);\n%%end",
            "%%start\nu32 source = 1;\nu32 value = core.int_extend<u32>(source);\n%%end",
            "%%start\nbool source = true;\nu32 value = core.int_extend<u32>(source);\n%%end",
            "%%start\nu64 source = 1;\nu32 value = core.int_trunc<bool>(source);\n%%end",
            "%%start\nu64 source = 1;\nu32 value = core.int_trunc(source);\n%%end",
            "%%start\nu64 source = 1;\nu32 value = core.int_trunc<u32, u16>(source);\n%%end",
            "%%start\nu64 source = 1;\nu32 value = core.int_trunc<u32>();\n%%end",
            "%%start\nu64 source = 1;\nu32 value = core.int_trunc<u32>(source, source);\n%%end",
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "B0003" || diagnostic.code == "B0004"),
                "{text}: {:?}",
                invalid.diagnostics
            );
        }
    }

    #[test]
    fn validates_float_conversions_in_single_file_and_project() {
        let text = "%%start\nu64 uint_source = 42;\ni64 sint_source = 42;\nf64 wide_source = 1.5;\nf32 narrow_source = 1.5;\nf64 from_uint = core.uint_to_float<f64>(uint_source);\nf32 from_sint = core.sint_to_float<f32>(sint_source);\nf64 from_uint_literal = core.uint_to_float<f64>(42);\nf32 from_sint_literal = core.sint_to_float<f32>(-3);\ni32 to_sint = core.float_to_sint_trunc<i32>(wide_source);\nu64 to_uint = core.float_to_uint_trunc<u64>(narrow_source);\ni32 to_sint_literal = core.float_to_sint_trunc<i32>(1.5);\nf32 narrowed = core.float_trunc<f32>(wide_source);\nf32 narrowed_literal = core.float_trunc<f32>(1.5);\nf64 widened = core.float_extend<f64>(narrow_source);\nf64 widened_literal = core.float_extend<f64>(1.5);\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

        for (text, code) in [
            (
                "%%start\nu64 source = 1;\nu32 value = core.uint_to_float<u32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\ni64 source = 1;\nf64 value = core.sint_to_float<i64>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf64 source = 1.5;\nf64 value = core.float_to_sint_trunc<f64>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 source = 1.5;\nf32 value = core.float_to_uint_trunc<f32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf64 source = 1.5;\nu32 value = core.float_trunc<u32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 source = 1.5;\ni32 value = core.float_extend<i32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\ni64 source = 1;\nf64 value = core.uint_to_float<f64>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nu64 source = 1;\nf32 value = core.sint_to_float<f32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf64 value = core.uint_to_float<f64>(-1);\n%%end",
                "B0003",
            ),
            (
                "%%start\ni64 source = 1;\ni32 value = core.float_to_sint_trunc<i32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nu64 source = 1;\nf32 value = core.float_trunc<f32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 source = 1.5;\nu32 value = core.float_to_sint_trunc<u32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 source = 1.5;\ni32 value = core.float_to_uint_trunc<i32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 source = 1.5;\nf32 value = core.float_trunc<f32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf64 source = 1.5;\nf64 value = core.float_extend<f64>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf64 source = 1.5;\nf64 value = core.float_trunc<f64>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 source = 1.5;\nf32 value = core.float_extend<f32>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nu64 source = 1;\nf64 value = core.uint_to_float(source);\n%%end",
                "B0004",
            ),
            (
                "%%start\nu64 source = 1;\nf64 value = core.uint_to_float<f64, f32>(source);\n%%end",
                "B0004",
            ),
            (
                "%%start\nu64 source = 1;\nf64 value = core.uint_to_float<f64>();\n%%end",
                "B0004",
            ),
            (
                "%%start\nu64 source = 1;\nf64 value = core.sint_to_float<f64>(source, source);\n%%end",
                "B0004",
            ),
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{text}: {:?}",
                invalid.diagnostics
            );
        }
    }

    #[test]
    fn validates_raw_offset_and_load_under_unsafe_in_single_file_and_project() {
        let text = "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index)) } };\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

        let missing_unsafe = validate_text(
            "%%start\nu8(*?u8, i64) read = fn(pointer, index) { core.load<u8>(core.offset<u8>(pointer, index)) };\n%%end",
        );
        assert!(
            missing_unsafe
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0012"),
            "{:?}",
            missing_unsafe.diagnostics
        );

        for (text, code) in [
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer)) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index, index)) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset(pointer, index)) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(pointer, index) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, u64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index)) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nu8(u8, i64) read = fn(value, index) { unsafe { core.load<u8>(core.offset<u8>(value, index)) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u16>(core.offset<u8>(pointer, index)) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nunit(*?u8) consume = fn(pointer) { unsafe { core.load<unit>(pointer) } };\n%%end",
                "B0003",
            ),
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{text}: {:?}",
                invalid.diagnostics
            );
        }
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
        assert_eq!(
            binding.output_origin,
            ScalarBindingOutputOrigin::SingleExpression
        );
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
    fn derives_and_substitutes_scoped_generic_function_parameters() {
        let result = validate_text(
            "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\ni64 result = identity<i64>(1);\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Function(function) = &result.program.items[0] else {
            panic!("generic function");
        };
        assert_eq!(function.generic_parameters.len(), 1);
        assert_eq!(function.generic_parameters[0].name, "T");
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("generic call binding");
        };
        assert_eq!(binding.declared_type, ScalarType::I64);
    }

    #[test]
    fn resolves_ordinary_and_generic_overload_arms_in_single_file_and_project() {
        let text = "%%start\ncast = overload {\n    i32(i64) => fn(value) { value };\n    generic T;\n    T(T) => fn(value) { value };\n};\npair = overload {\n    generic T;\n    (T, T)(T) => fn(value) { value };\n};\ni64 wide = 1;\ni32 narrow = cast(wide);\nbool flag = true;\nbool copied = cast(flag);\ni64 first, i64 second = pair(wide);\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarItem::Function(overload) = &single.program.items[0] else {
            panic!("overload declaration");
        };
        assert_eq!(overload.overload_arms.len(), 2);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }

    #[test]
    fn resolves_struct_typed_overload_arm_in_single_file_and_project() {
        let text = "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\nparse = overload {\n    *u64(utf8) => fn(text) {\n        *u64 result = null;\n        result\n    };\n};\nutf8 input = { .data = null; .length = 0; };\n*u64 parsed = parse(input);\n%%end";
        let single = validate_text(text);
        assert!(
            !single
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"),
            "{:?}",
            single.diagnostics
        );
        assert!(
            !single
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0004"),
            "{:?}",
            single.diagnostics
        );
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source],
        ));
        assert!(
            !project
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"),
            "{:?}",
            project.diagnostics
        );
        assert!(
            !project
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0004"),
            "{:?}",
            project.diagnostics
        );
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }

    #[test]
    fn keeps_generic_overload_arm_placeholders_when_resolving_struct_typed_arms() {
        let text = "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\nconvert = overload {\n    u64(utf8) => fn(text) { text.length };\n    generic T;\n    T(T) => fn(value) { value };\n};\nutf8 input = { .data = null; .length = 0; };\nu64 width = convert(input);\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarItem::Function(overload) = &single.program.items[0] else {
            panic!("overload declaration");
        };
        assert_eq!(overload.overload_arms.len(), 2);
        let ScalarType::Callable { parameters, .. } = &overload.overload_arms[1].signature else {
            panic!("generic arm signature");
        };
        assert!(
            matches!(&parameters[0], ScalarType::Named { name, .. } if name == "T"),
            "{:?}",
            overload.overload_arms[1].signature
        );

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }

    #[test]
    fn resolves_qualified_generic_overload_from_value_argument_and_records_selection() {
        let child_source = module_source("src/child.w");
        let child = module_from_text(
            child_source.clone(),
            "%%start\nidentity = overload {\n    i32(i64) => fn(value) { 7 };\n    generic T;\n    T(T) => fn(value) { value };\n};\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchar copied = child.identity('g');\n%%end",
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
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![main_source, child_source],
        ));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.project.modules[0].items[1] else {
            panic!("qualified overload binding");
        };
        let ScalarExpression::Call {
            overload_selection: Some(selection),
            ..
        } = &binding.value
        else {
            panic!("qualified overload selection");
        };
        assert_eq!(selection.arm_index, 1);
        assert_eq!(selection.substitutions.get("T"), Some(&ScalarType::Char));
    }

    #[test]
    fn keeps_private_overloads_local_and_reports_imported_private_overloads_at_member_tokens() {
        let child_source = module_source("src/child.w");
        let child = module_from_text(
            child_source.clone(),
            "%%start\n_ordinary = overload {\n    i32(i64) => fn(value) { value };\n};\n_identity = overload {\n    i32(i64) => fn(value) { 7 };\n    generic T;\n    T(T) => fn(value) { value };\n};\npublic_ordinary = overload {\n    i32(i64) => fn(value) { value };\n};\npublic_identity = overload {\n    i32(i64) => fn(value) { 7 };\n    generic T;\n    T(T) => fn(value) { value };\n};\ni32 local_ordinary = _ordinary(1);\nchar local_generic = _identity('l');\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main_text = "%%start\nchild = namespace generic_overloads \"src/child.w\";\ni32 public_ordinary = child.public_ordinary(1);\nchar public_value = child.public_identity('p');\ni32 ordinary = child._ordinary(1);\nchar value = child._identity('g');\n%%end";
        let main = module_from_text(main_source.clone(), main_text);
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
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![main_source.clone(), child_source],
        ));
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "M0002")
            .collect();
        assert_eq!(
            diagnostics.len(),
            result.diagnostics.len(),
            "{:?}",
            result.diagnostics
        );
        assert_eq!(diagnostics.len(), 2, "{:?}", result.diagnostics);
        for (diagnostic, member) in diagnostics.iter().zip(["_ordinary", "_identity"]) {
            let qualified = format!("child.{member}");
            let start = main_text.find(&qualified).expect("private member") + "child.".len();
            let start = u32::try_from(start).expect("source span");
            let end = start + u32::try_from(member.len()).expect("member span");
            assert_eq!(diagnostic.labels[0].span.source, main_source);
            assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, end));
        }
    }

    #[test]
    fn suppresses_derived_assignment_diagnostics_for_private_overload_output_sequences() {
        let child_source = module_source("src/child.w");
        let child = module_from_text(
            child_source.clone(),
            "%%start\n_private_pair = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n};\npublic_pair = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n};\ni32 local_number, bool local_flag = _private_pair(1);\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main_text = "%%start\nchild = namespace output_sequences \"src/child.w\";\ni32 public_number, bool public_flag = child.public_pair(1);\nnumber, flag = child._private_pair(1);\n%%end";
        let main = module_from_text(main_source.clone(), main_text);
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
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![main_source.clone(), child_source],
        ));
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        let diagnostic = &result.diagnostics[0];
        assert_eq!(diagnostic.code, "M0002");
        assert_eq!(diagnostic.labels[0].span.source, main_source);
        let start = main_text
            .find("child._private_pair")
            .expect("private overload member")
            + "child.".len();
        let start = u32::try_from(start).expect("source span");
        let end = start + u32::try_from("_private_pair".len()).expect("member span");
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, end));
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0004"));
    }

    #[test]
    fn records_distinct_overload_arms_for_shared_input_types() {
        let result = validate_text(
            "%%start\nselect = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n    (bool, i32)(i64) => fn(value) { true, 1 };\n};\ni64 input = 1;\ni32 integer, bool flag = select(input);\nbool boolean, i32 number = select(input);\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(integer) = &result.program.items[2] else {
            panic!("integer overload call");
        };
        let ScalarExpression::Call {
            overload_selection: Some(integer_selection),
            ..
        } = &integer.value
        else {
            panic!("integer overload selection");
        };
        assert_eq!(integer_selection.arm_index, 0);
        let ScalarItem::Binding(boolean) = &result.program.items[3] else {
            panic!("boolean overload call");
        };
        let ScalarExpression::Call {
            overload_selection: Some(boolean_selection),
            ..
        } = &boolean.value
        else {
            panic!("boolean overload selection");
        };
        assert_eq!(boolean_selection.arm_index, 1);
    }

    #[test]
    fn reports_overload_candidate_failures_at_call_spans() {
        let explicit = validate_text(
            "%%start\nselect = overload { i32(i32) => fn(value) { value }; };\ni32 value = select<i32>(1);\n%%end",
        );
        assert!(explicit.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "overload calls do not accept explicit type arguments"
        }));
        let ambiguous = validate_text(
            "%%start\nselect = overload { i32(i32) => fn(value) { value }; i32(i32) => fn(value) { value }; };\ni32 value = select(1);\n%%end",
        );
        assert!(ambiguous
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "overload call is ambiguous"));
    }

    #[test]
    fn infers_integer_literal_overloads_only_from_unique_parameter_or_output_context() {
        let constrained = "%%start\nidentity = overload { generic T; T(T) => fn(value) { value }; };\ni64 value = identity(1);\n%%end";
        let single = validate_text(constrained);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), constrained);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

        let unconstrained = "%%start\nidentity = overload { generic T; T(T) => fn(value) { value }; };\nidentity(1);\n%%end";
        let single = validate_text(unconstrained);
        assert!(single.diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "integer literal requires a unique overload parameter or output context"
        }));

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), unconstrained);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "integer literal requires a unique overload parameter or output context"
        }));

        let ambiguous = "%%start\nselect = overload { i32(i32) => fn(value) { value }; i64(i64) => fn(value) { value }; };\nselect(1);\n%%end";
        let single = validate_text(ambiguous);
        assert!(single
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "overload call is ambiguous"));

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), ambiguous);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "overload call is ambiguous"));
    }

    #[test]
    fn substitutes_generic_zero_one_and_multiple_outputs_in_single_file_and_project() {
        let text = "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\ngeneric T;\nunit(T) discard = fn(value) { value; };\ngeneric T;\n(T, T)(T) pair = fn(value) { value };\ni64 one = identity<i64>(1);\ndiscard<i64>(1);\ni64 first, i64 second = pair<i64>(1);\n%%end";
        let single = validate_text(text);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarItem::Binding(binding) = &single.program.items[5] else {
            panic!("multiple-output binding");
        };
        assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::I64);
        assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::I64);

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
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
    fn accepts_documented_declared_identifier_styles_in_single_file() {
        let result = validate_text(
            "%%start\nsnake_namespace = namespace app \"src/library.w\";\nextern_binding = extern wasm \"env\" { i32(i32) extern_function; };\ni32 snake_case = 1;\ni32 PascalCase = 2;\ni32 SCREAMING_SNAKE_CASE = 3;\ni32 _ = 4;\ni32(i32) function_name = fn(_name) { i32 _local = _name; _local };\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn rejects_camel_case_declarations_at_identifier_spans_in_single_file() {
        let text = "%%start\ncamelNamespace = namespace app \"src/library.w\";\ncamelExternBinding = extern wasm \"env\" { i32(i32) camelExternFunction; };\ni32 camelBinding = 1;\ni32(i32) camelFunction = fn(camelParameter) { i32 camelLocal = camelParameter; camelLocal };\n%%end";
        let result = validate_text(text);
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message
                        == "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE"
            })
            .collect();
        let identifiers = [
            "camelNamespace",
            "camelExternBinding",
            "camelExternFunction",
            "camelBinding",
            "camelFunction",
            "camelParameter",
            "camelLocal",
        ];
        assert_eq!(diagnostics.len(), identifiers.len());
        for identifier in identifiers {
            let start = text.find(identifier).expect("identifier") as u32;
            let span = ByteSpan::new(start, start + identifier.len() as u32);
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.labels[0].span.range == span),
                "missing diagnostic for {identifier}"
            );
        }
    }

    #[test]
    fn validates_declared_identifier_styles_in_project() {
        let source = module_source("src/main.w");
        let accepted = module_from_text(
            source.clone(),
            "%%start\nsnake_namespace = namespace app \"src/library.w\";\nextern_binding = extern wasm \"env\" { i32(i32) extern_function; };\ni32 snake_case = 1;\ni32 PascalCase = 2;\ni32 SCREAMING_SNAKE_CASE = 3;\ni32 _ = 4;\ni32(i32) function_name = fn(_name) { i32 _local = _name; _local };\n%%end",
        );
        let accepted = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                source.clone(),
                accepted.items,
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(
            accepted.diagnostics.is_empty(),
            "{:?}",
            accepted.diagnostics
        );

        let text = "%%start\ncamelNamespace = namespace app \"src/library.w\";\ncamelExternBinding = extern wasm \"env\" { i32(i32) camelExternFunction; };\ni32 camelBinding = 1;\ni32(i32) camelFunction = fn(camelParameter) { i32 camelLocal = camelParameter; camelLocal };\n%%end";
        let rejected = module_from_text(source.clone(), text);
        let rejected = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                source.clone(),
                rejected.items,
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        let diagnostics: Vec<_> = rejected
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message
                        == "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE"
            })
            .collect();
        let identifiers = [
            "camelNamespace",
            "camelExternBinding",
            "camelExternFunction",
            "camelBinding",
            "camelFunction",
            "camelParameter",
            "camelLocal",
        ];
        assert_eq!(diagnostics.len(), identifiers.len());
        for identifier in identifiers {
            let start = text.find(identifier).expect("identifier") as u32;
            let span = ByteSpan::new(start, start + identifier.len() as u32);
            assert!(
                diagnostics.iter().any(|diagnostic| {
                    diagnostic.labels[0].span.source == source
                        && diagnostic.labels[0].span.range == span
                }),
                "missing diagnostic for {identifier}"
            );
        }
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
        assert_eq!(
            binding.output_origin,
            ScalarBindingOutputOrigin::SingleExpression
        );
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
            enum_tag: _,
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
        assert_eq!(result.program.structs[0].fields[0].offset, Some(0));
        assert_eq!(result.program.structs[0].fields[1].offset, Some(4));
        assert_eq!(result.program.structs[0].layout.as_ref().unwrap().size, 8);
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
    fn discards_unreceived_outputs_in_scalar_programs() {
        for text in [
            "%%start\n(i32, u64)() pair = fn { 1 };\npair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = pair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first, u64 second = pair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = 0;\nfirst = pair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = 0;\nu64 second = 0;\nfirst, second = pair();\n%%end",
        ] {
            let valid = validate_text(text);
            assert!(valid.diagnostics.is_empty(), "{text}: {:?}", valid.diagnostics);
        }

        let wrong_binding =
            validate_text("%%start\n(i32, u64)() pair = fn { 1 };\nbool first = pair();\n%%end");
        assert!(wrong_binding.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "expression type does not match expected type"
        }));
        let wrong_assignment = validate_text(
            "%%start\n(i32, u64)() pair = fn { 1 };\nbool first = false;\nfirst = pair();\n%%end",
        );
        assert!(wrong_assignment.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "expression type does not match expected type"
        }));

        for (text, message) in [
            (
                "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first, u64 second, bool third = pair();\n%%end",
                "call has fewer outputs than receivers",
            ),
            (
                "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = 0;\nu64 second = 0;\nbool third = false;\nfirst, second, third = pair();\n%%end",
                "call has fewer outputs than assignment targets",
            ),
        ] {
            let invalid = validate_text(text);
            assert!(invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == message));
            assert!(!invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.message == "call has more outputs than receivers"
                    || diagnostic.message == "call has more outputs than assignment targets"
            }));
        }
    }

    #[test]
    fn discards_unreceived_outputs_in_namespace_resolved_projects() {
        let std_source = module_source("src/std.w");
        let std = module_from_text(
            std_source.clone(),
            "%%start\n(i32, u64)() print = fn { 1 };\n%%end",
        );
        let validate_project_text = |text: &str| {
            let main_source = module_source("src/main.w");
            let main = module_from_text(main_source.clone(), text);
            let namespace_span = match &main.items[0] {
                ScalarItem::Namespace(namespace) => namespace.span,
                _ => panic!("namespace item"),
            };
            validate_scalar_project(ScalarProject::new(
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
                    ScalarModule::new(std_source.clone(), std.items.clone(), Vec::new()),
                ],
                Vec::new(),
            ))
        };

        for text in [
            "%%start\nstd = namespace app \"src/std.w\";\nstd.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first = std.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first, u64 second = std.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first = 0;\nfirst = std.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first = 0;\nu64 second = 0;\nfirst, second = std.print();\n%%end",
        ] {
            let valid = validate_project_text(text);
            assert!(valid.diagnostics.is_empty(), "{text}: {:?}", valid.diagnostics);
        }

        let wrong_type = validate_project_text(
            "%%start\nstd = namespace app \"src/std.w\";\nbool first = std.print();\n%%end",
        );
        assert!(wrong_type.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "expression type does not match expected type"
        }));

        for (text, message) in [
            (
                "%%start\nstd = namespace app \"src/std.w\";\ni32 first, u64 second, bool third = std.print();\n%%end",
                "call has fewer outputs than receivers",
            ),
            (
                "%%start\nstd = namespace app \"src/std.w\";\ni32 first = 0;\nu64 second = 0;\nbool third = false;\nfirst, second, third = std.print();\n%%end",
                "call has fewer outputs than assignment targets",
            ),
        ] {
            let invalid = validate_project_text(text);
            assert!(invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == message));
            assert!(!invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.message == "call has more outputs than receivers"
                    || diagnostic.message == "call has more outputs than assignment targets"
            }));
        }
    }

    #[test]
    fn preserves_independent_output_expression_spans() {
        let result = validate_text("%%start\nu64 first, bool second = 1, true;\n%%end");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("output binding");
        };
        assert_eq!(
            binding.output_origin,
            ScalarBindingOutputOrigin::IndependentExpressions
        );
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
    fn marks_equal_independent_output_expressions_independently() {
        let result = validate_text("%%start\nu64 first, u64 second = 1, 1;\n%%end");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("output binding");
        };
        assert_eq!(
            binding.output_origin,
            ScalarBindingOutputOrigin::IndependentExpressions
        );
        assert!(matches!(
            (
                &binding.output_values[0].value,
                &binding.output_values[1].value
            ),
            (
                ScalarExpression::Integer { value: first, .. },
                ScalarExpression::Integer { value: second, .. }
            ) if first == second
        ));
        assert_ne!(
            expression_span(&binding.output_values[0].value),
            expression_span(&binding.output_values[1].value)
        );
    }

    #[test]
    fn preserves_one_output_call_contexts_for_strict_integers_and_null() {
        let result = validate_text(
            "%%start\nu32(u32) identity = fn(value) { value };\nu32 number = identity(1);\nu32(*?u8) keep = fn(value) { if (value == null) { 1 } else { 1 } };\nu32 result = keep(null);\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Binding(binding) = &result.program.items[1] else {
            panic!("one-output binding");
        };
        assert_eq!(
            binding.output_origin,
            ScalarBindingOutputOrigin::SingleExpression
        );
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
    fn derives_checked_reference_addresses_with_exact_shared_mutable_and_aggregate_types() {
        let result = validate_text(
            "%%start\nstruct Item {\n\tu8 field;\n}\nu8 value = 1;\nItem record = { .field = 2; };\nu8[4] bytes = [3, 4, 5, 6];\n*u8 shared = &value;\n*!u8 mutable = &!value;\n*u8 field = &record.field;\n*(u8[4]) array = &bytes;\n*!(u8[4]) mutable_array = &!bytes;\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        for (item, mutability, array) in [
            (3, ScalarReferenceMutability::Shared, false),
            (4, ScalarReferenceMutability::Mutable, false),
            (5, ScalarReferenceMutability::Shared, false),
            (6, ScalarReferenceMutability::Shared, true),
            (7, ScalarReferenceMutability::Mutable, true),
        ] {
            let item = &result.program.items[item];
            let ScalarItem::Binding(binding) = item else {
                panic!("checked reference binding")
            };
            assert!(matches!(
                &binding.declared_type,
                ScalarType::CheckedReference { mutability: actual, inner }
                    if *actual == mutability
                        && if array {
                            matches!(
                                inner.as_ref(),
                                ScalarType::Array { element, length: 4, .. }
                                    if element.as_ref() == &ScalarType::U8
                            )
                        } else {
                            inner.as_ref() == &ScalarType::U8
                        }
            ));
            assert!(matches!(
                binding.value,
                ScalarExpression::CheckedAddress { .. }
            ));
        }
    }

    #[test]
    fn derives_checked_reference_dereference_reads_and_rejects_deferred_targets() {
        let result = validate_text(
            "%%start\nstruct Record {\n\tu8 first;\n\tu32 second;\n}\nu32 value = 7;\nRecord item = { .first = 1; .second = 2; };\n*u32 shared = &value;\n*!u32 mutable = &!value;\n*Record record_shared = &item;\nu32 shared_value = *shared;\nu32 mutable_value = *mutable;\nu32 field_value = (*record_shared).second;\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for index in [5, 6, 7] {
            let ScalarItem::Binding(binding) = &result.program.items[index] else {
                panic!("checked dereference binding")
            };
            assert!(matches!(
                binding.value,
                ScalarExpression::Dereference { .. }
            ));
        }

        let invalid_text = "%%start\nu32 value = 7;\n*?u32 raw = null;\n*unit unit = null;\nu32 non_reference = *value;\nu32 raw_read = *raw;\nunit unit_read = *unit;\n%%end";
        let invalid = validate_text(invalid_text);
        for message in [
            "cannot dereference value",
            "checked dereference requires a checked reference",
            "cannot read unit through checked reference",
        ] {
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message == message),
                "{:?}",
                invalid.diagnostics
            );
        }
        for (message, spelling) in [
            ("cannot dereference value", "*value"),
            ("checked dereference requires a checked reference", "*raw"),
            ("cannot read unit through checked reference", "*unit"),
        ] {
            let start = invalid_text.rfind(spelling).expect("dereference") as u32;
            let diagnostic = invalid
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message == message)
                .expect("dereference diagnostic");
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + spelling.len() as u32)
            );
        }

        let invalid_field_text = "%%start\nstruct Record {\n\tunit marker;\n\tu32 value;\n}\nRecord item = { .marker = 1; .value = 2; };\n*Record reference = &item;\nunit marker = (*reference).marker;\nu32 missing = (*reference).missing;\n%%end";
        let invalid_field = validate_text(invalid_field_text);
        for (message, field) in [
            ("cannot read unit through checked reference", "marker"),
            ("value has no field", "missing"),
        ] {
            let start = invalid_field_text.rfind(field).expect("field") as u32;
            let diagnostic = invalid_field
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message == message)
                .expect("field diagnostic");
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + field.len() as u32)
            );
        }

        let aggregate = validate_text(
            "%%start\nstruct Record {\n\tu32 value;\n}\nRecord record = { .value = 7; };\n*Record reference = &record;\nRecord read = *reference;\n%%end",
        );
        assert!(aggregate.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "whole aggregate checked dereference read is not supported"
        }));

        let null_reference =
            validate_text("%%start\n*u32 reference = null;\nu32 value = *reference;\n%%end");
        assert!(
            null_reference.diagnostics.is_empty(),
            "{:?}",
            null_reference.diagnostics
        );
    }

    #[test]
    fn reports_project_dereference_field_errors_at_the_field_token() {
        let child_source = module_source("src/child.w");
        let child = module_from_text(
            child_source.clone(),
            "%%start\nstruct Record {\n\tunit marker;\n\tu32 value;\n}\n%%end",
        );
        let root_source = module_source("src/main.w");
        let root_text = "%%start\nchild = namespace app \"src/child.w\";\nu32 value = 1;\n*?u32 raw = null;\n*unit unit = null;\nu32 invalid_value = *value;\nu32 invalid_raw = *raw;\nunit invalid_unit = *unit;\nchild.Record item = { .marker = 1; .value = 1; };\n*child.Record reference = &item;\nunit marker = (*reference).marker;\nu32 result = (*reference).missing;\n%%end";
        let root = module_from_text(root_source.clone(), root_text);
        let namespace = match &root.items[0] {
            ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
            _ => panic!("namespace"),
        };
        let result = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    root,
                    vec![ScalarNamespaceBinding {
                        binding: namespace.0,
                        target: child_source.clone(),
                        span: namespace.1,
                    }],
                ),
                ScalarModule::from_program(child, Vec::new()),
            ],
            vec![root_source, child_source],
        ));
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "value has no field")
            .expect("field diagnostic");
        let start = root_text.rfind("missing").expect("field") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + "missing".len() as u32)
        );
        for (message, spelling) in [
            ("cannot dereference value", "*value"),
            ("checked dereference requires a checked reference", "*raw"),
            ("cannot read unit through checked reference", "*unit"),
        ] {
            let start = root_text.rfind(spelling).expect("dereference") as u32;
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message == message)
                .expect("dereference diagnostic");
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + spelling.len() as u32)
            );
        }
        let marker_start = root_text.rfind("marker").expect("field") as u32;
        let marker = result
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message == "cannot read unit through checked reference"
                    && diagnostic.labels[0].span.range
                        == ByteSpan::new(marker_start, marker_start + "marker".len() as u32)
            })
            .expect("unit field diagnostic");
        assert_eq!(
            marker.labels[0].span.range,
            ByteSpan::new(marker_start, marker_start + "marker".len() as u32)
        );
    }

    #[test]
    fn rejects_checked_reference_mutability_mismatches_and_unsupported_addresses() {
        let mismatch = validate_text(
            "%%start\nu8 value = 1;\n*!u8 shared = &value;\n*u8 mutable = &!value;\n%%end",
        );
        assert_eq!(
            mismatch
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message
                    == "expression type does not match expected type")
                .count(),
            2,
            "{:?}",
            mismatch.diagnostics
        );

        let unsupported = validate_text(
            "%%start\nstruct Record {\n\tu8 field;\n}\n*?Record pointer = null;\n*u8 field = &(*pointer).field;\n%%end",
        );
        assert!(unsupported.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "checked address requires a storage name or direct storage field"
        }));

        let callable =
            validate_text("%%start\nu8() make = fn { 1 };\n*(u8()) reference = &make;\n%%end");
        assert!(callable.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "checked address requires a storage name or direct storage field"
        }));

        let unit = validate_text(
            "%%start\nunit() touch = fn { 1; };\nunit value = touch();\n*unit reference = &value;\n%%end",
        );
        assert!(unit.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "checked address requires a storage name or direct storage field"
        }));
        assert_eq!(
            crate::emit_scalar_llvm(&unit).unwrap_err(),
            "cannot emit LLVM for an invalid scalar program"
        );

        let unit_field = validate_text(
            "%%start\nstruct Record {\n\tunit field;\n}\nunit() touch = fn { 1; };\nRecord record = { .field = touch(); };\n*unit reference = &record.field;\n%%end",
        );
        assert!(unit_field.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "checked address requires a storage name or direct storage field"
        }));
    }

    #[test]
    fn retains_raw_address_safety_and_rejects_checked_references_in_extern_signatures() {
        let raw = validate_text("%%start\nu8 value = 1;\n*?u8 address = &?value;\n%%end");
        assert!(raw.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0012"
                && diagnostic.message == "raw address requires an unsafe block"
        }));

        let externs = validate_text(
            "%%start\nenv = extern wasm \"env\" { unit(*u8) consume; *!u8() produce; };\n%%end",
        );
        assert_eq!(
            externs
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message
                    == "direct FFI checked references are not supported")
                .count(),
            2,
            "{:?}",
            externs.diagnostics
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
    fn types_imported_struct_field_checked_addresses_in_projects() {
        let child_source = module_source("src/child.w");
        let child_program = module_from_text(
            child_source.clone(),
            "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n%%end",
        );
        let imported_id = child_program.structs[0].id.clone();
        let main_source = module_source("src/main.w");
        let main_program = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\n*u32 address = &item.second;\n*child.Pair pair = &item;\nu32 read = (*pair).second;\n%%end",
        );
        let namespace_span = match &main_program.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main_program,
                    vec![ScalarNamespaceBinding {
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::from_program(child_program, Vec::new()),
            ],
            vec![main_source, child_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let ScalarItem::Binding(binding) = &validation.project.modules[0].items[2] else {
            panic!("checked address binding")
        };
        assert_eq!(
            binding.declared_type,
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Shared,
                inner: Box::new(ScalarType::U32),
            }
        );
        let ScalarExpression::CheckedAddress {
            place: ScalarPlace::Field { field, .. },
            ..
        } = &binding.value
        else {
            panic!("checked field address")
        };
        assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
    }

    #[test]
    fn resolves_imported_struct_fixed_array_field_indexes_and_addresses_in_projects() {
        let child_source = module_source("src/child.w");
        let child_program = module_from_text(
            child_source.clone(),
            "%%start\nstruct Record {\n\tu8[3] bytes;\n}\n%%end",
        );
        let imported_id = child_program.structs[0].id.clone();
        let main_source = module_source("src/main.w");
        let main_text = "%%start\nchild = namespace app \"src/child.w\";\nchild.Record value = { .bytes = [3, 5, 7]; };\nu64 index = 1;\nu8 result = value.bytes[index];\n*u8 shared = &value.bytes[index];\n*!u8 mutable = &!value.bytes[index];\nunsafe { *?u8 raw = &?value.bytes[index]; };\n%%end";
        let main_program = module_from_text(main_source.clone(), main_text);
        let namespace_span = match &main_program.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main_program,
                    vec![ScalarNamespaceBinding {
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::from_program(child_program, Vec::new()),
            ],
            vec![main_source, child_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let ScalarItem::Binding(value) = &validation.project.modules[0].items[1] else {
            panic!("imported struct binding")
        };
        assert_eq!(value.declared_type, ScalarType::Struct(imported_id.clone()));
        let field_starts = main_text
            .match_indices("value.bytes[index]")
            .map(|(start, _)| start as u32)
            .collect::<Vec<_>>();
        let ScalarItem::Binding(result) = &validation.project.modules[0].items[3] else {
            panic!("indexed read binding")
        };
        assert_eq!(result.declared_type, ScalarType::U8);
        let ScalarExpression::IndexedRead {
            place:
                ScalarPlace::Index {
                    base,
                    span,
                    index_span,
                    ..
                },
            ..
        } = &result.value
        else {
            panic!("indexed read")
        };
        let expected_field_span = ByteSpan::new(field_starts[0], field_starts[0] + 11);
        let expected_index_span = ByteSpan::new(field_starts[0] + 12, field_starts[0] + 17);
        assert_eq!(
            *span,
            ByteSpan::new(expected_field_span.start, field_starts[0] + 18)
        );
        assert_eq!(*index_span, expected_index_span);
        let ScalarPlace::Field { field, span, .. } = base.as_ref() else {
            panic!("imported fixed-array field")
        };
        assert_eq!(*span, expected_field_span);
        assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));

        for (address_index, (item_index, expected_type)) in [
            (
                4,
                ScalarType::CheckedReference {
                    mutability: ScalarReferenceMutability::Shared,
                    inner: Box::new(ScalarType::U8),
                },
            ),
            (
                5,
                ScalarType::CheckedReference {
                    mutability: ScalarReferenceMutability::Mutable,
                    inner: Box::new(ScalarType::U8),
                },
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let ScalarItem::Binding(binding) = &validation.project.modules[0].items[item_index]
            else {
                panic!("indexed checked address binding")
            };
            assert_eq!(binding.declared_type, expected_type);
            let ScalarExpression::CheckedAddress {
                place:
                    ScalarPlace::Index {
                        base, index_span, ..
                    },
                ..
            } = &binding.value
            else {
                panic!("indexed checked address")
            };
            let field_start = field_starts[address_index + 1];
            let expected_field_span = ByteSpan::new(field_start, field_start + 11);
            assert_eq!(
                *index_span,
                ByteSpan::new(field_start + 12, field_start + 17)
            );
            let ScalarPlace::Field { field, span, .. } = base.as_ref() else {
                panic!("imported fixed-array field")
            };
            assert_eq!(*span, expected_field_span);
            assert!(
                matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id)
            );
        }

        let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            &validation.project.modules[0].items[6]
        else {
            panic!("unsafe block")
        };
        let ScalarBlockItem::LocalBinding(raw) = &block.items[0] else {
            panic!("indexed raw address binding")
        };
        assert_eq!(
            raw.declared_type,
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        );
        let ScalarExpression::RawAddress {
            place: ScalarPlace::Index {
                base, index_span, ..
            },
            ..
        } = &raw.value
        else {
            panic!("indexed raw address")
        };
        let field_start = field_starts[3];
        let expected_field_span = ByteSpan::new(field_start, field_start + 11);
        assert_eq!(
            *index_span,
            ByteSpan::new(field_start + 12, field_start + 17)
        );
        let ScalarPlace::Field { field, span, .. } = base.as_ref() else {
            panic!("imported fixed-array field")
        };
        assert_eq!(*span, expected_field_span);
        assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
    }

    #[test]
    fn validates_imported_struct_fixed_array_field_writes_in_projects() {
        let child_source = module_source("src/child.w");
        let child_program = module_from_text(
            child_source.clone(),
            "%%start\nstruct Record { u8[3] bytes; }\n%%end",
        );
        let main_source = module_source("src/main.w");
        let main_program = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchild.Record holder = { .bytes = [1, 2, 3]; };\nu64 index = 1;\nholder.bytes[0], holder.bytes[2] = 4, 5;\nholder.bytes[index] = 6;\n%%end",
        );
        let namespace_span = match &main_program.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main_program,
                    vec![ScalarNamespaceBinding {
                        binding: "child".to_owned(),
                        target: child_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::from_program(child_program, Vec::new()),
            ],
            vec![main_source, child_source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
    }

    #[test]
    fn resolves_utf8_literals_through_aliased_namespace_targets() {
        let std_source = module_source("src/bootstrap.w");
        let std = module_from_text(
            std_source.clone(),
            "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\ni32 value = 80;\n%%end",
        );
        let imported_id = std.structs[0].id.clone();
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\ntext = namespace std \"src/bootstrap.w\";\nstd = namespace std \"src/bootstrap.w\";\ni32 observed = text.value;\nstd.utf8 standard = \"\";\ntext.utf8 literal = \"hé\";\n*?u8 data = literal.data;\nu64 length = literal.length;\nunsafe { *?text.utf8 pointer = &?literal; *?u8 pointer_data = pointer.data; u64 pointer_length = pointer.length; };\n%%end",
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
                    vec![
                        ScalarNamespaceBinding {
                            binding: "text".to_owned(),
                            target: std_source.clone(),
                            span: namespace_span,
                        },
                        ScalarNamespaceBinding {
                            binding: "std".to_owned(),
                            target: std_source.clone(),
                            span: namespace_span,
                        },
                    ],
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
        let ScalarItem::Binding(observed) = &validation.project.modules[0].items[2] else {
            panic!("namespace member binding")
        };
        assert!(matches!(
            observed.value,
            ScalarExpression::Member { ref receiver, ref name, .. }
                if receiver == "text" && name == "value"
        ));
        let ScalarItem::Binding(standard) = &validation.project.modules[0].items[3] else {
            panic!("conventional utf8 binding")
        };
        assert_eq!(
            standard.declared_type,
            ScalarType::Struct(imported_id.clone())
        );
        let ScalarItem::Binding(text) = &validation.project.modules[0].items[4] else {
            panic!("utf8 binding")
        };
        assert_eq!(text.declared_type, ScalarType::Struct(imported_id.clone()));
        assert!(matches!(
            text.value,
            ScalarExpression::Utf8 { ref value, .. } if value == b"h\xc3\xa9"
        ));
        let ScalarItem::Binding(data) = &validation.project.modules[0].items[5] else {
            panic!("data binding")
        };
        assert_eq!(
            data.declared_type,
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        );
        let ScalarItem::Binding(length) = &validation.project.modules[0].items[6] else {
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

    #[test]
    fn derives_local_pair_returns_in_source_order() {
        let result = validate_text(
            "%%start\n(i32, bool)() pair = fn { i32 local = 1; local, true };\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Function(function) = &result.program.items[0] else {
            panic!("pair function")
        };
        assert_eq!(function.body.final_output_values.len(), 2);
        assert_eq!(function.body.final_output_values[0].position, 0);
        assert_eq!(function.body.final_output_values[0].ty, ScalarType::I32);
        assert_eq!(function.body.final_output_values[1].position, 1);
        assert_eq!(function.body.final_output_values[1].ty, ScalarType::Bool);
        assert!(matches!(
            &function.body.final_output_values[0].value,
            ScalarExpression::Name { ref name, .. } if name == "local"
        ));
        assert!(matches!(
            &function.body.final_output_values[1].value,
            ScalarExpression::Boolean { value: true, .. }
        ));
    }

    #[test]
    fn derives_conditional_output_lists_by_position() {
        let result = validate_text(
            "%%start\n(i32, bool)() pair = fn { if (true) { 1, true } else { 2, false } };\n%%end",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Function(function) = &result.program.items[0] else {
            panic!("conditional pair function")
        };
        assert_eq!(function.body.final_output_values.len(), 2);
        assert_eq!(function.body.final_output_values[0].ty, ScalarType::I32);
        assert_eq!(function.body.final_output_values[1].ty, ScalarType::Bool);
        assert!(matches!(
            &function.body.final_output_values[0].value,
            ScalarExpression::If { .. }
        ));
    }

    #[test]
    fn validates_nested_conditional_output_lists_by_position() {
        let result = validate_text(
            "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end",
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn validates_nested_conditional_output_lists_in_a_project() {
        let source = module_source("src/pair.w");
        let program = module_from_text(
            source.clone(),
            "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end",
        );
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source],
        ));

        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
    }

    #[test]
    fn reports_nested_conditional_output_type_at_the_inner_branch() {
        let text = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, 3 } } else { 3, true } };\n%%end";
        let validation = validate_text(text);
        let diagnostic = validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0006")
            .expect("inner branch type diagnostic");
        let start = text
            .find("if (true) { 1, true } else { 2, 3 }")
            .expect("inner conditional");

        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(
                start as u32,
                (start + "if (true) { 1, true } else { 2, 3 }".len()) as u32
            )
        );
    }

    #[test]
    fn reports_nested_conditional_output_arity_at_the_inner_branch() {
        let text = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1 } else { 2, false } } else { 3, true } };\n%%end";
        let validation = validate_text(text);
        let diagnostic = validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0004")
            .expect("inner branch arity diagnostic");
        let start = text.find("{ 1 }").expect("inner branch") + 2;

        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start as u32, start as u32 + 1)
        );
    }

    #[test]
    fn reports_final_output_list_type_and_arity_diagnostics() {
        let wrong_type = validate_text("%%start\n(i32, bool)() pair = fn { true, 1 };\n%%end");
        assert!(wrong_type
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"));

        let wrong_arity =
            validate_text("%%start\n(i32, bool)() pair = fn { 1, true, false };\n%%end");
        assert!(wrong_arity.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0004"
                && diagnostic.message
                    == "function output arity does not match its final output list"
        }));
    }

    #[test]
    fn validates_control_flow_prefix_before_final_output_list_as_unit() {
        let text = "%%start\n(i32, bool)() pair = fn { if (true) { 1; } else { 2; } while (false) { 3; } 7, true };\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let source = source();
        let program = module_from_text(source.clone(), text);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }

    #[test]
    fn preserves_final_output_expression_spans() {
        let text = "%%start\n(i32, bool)() pair = fn { 12, false };\n%%end";
        let result = validate_text(text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Function(function) = &result.program.items[0] else {
            panic!("pair function")
        };
        let first_start = text.find("12").expect("first output") as u32;
        let second_start = text.find("false").expect("second output") as u32;
        assert_eq!(
            function.body.final_output_values[0].span,
            ByteSpan::new(first_start, first_start + 2)
        );
        assert_eq!(
            function.body.final_output_values[1].span,
            ByteSpan::new(second_start, second_start + 5)
        );
    }

    #[test]
    fn derives_and_contextually_validates_fixed_array_literals() {
        let valid = validate_text("%%start\nu8[2] bytes = [1, 2];\n%%end");
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let ScalarItem::Binding(binding) = &valid.program.items[0] else {
            panic!("array binding")
        };
        assert!(matches!(
            binding.declared_type,
            ScalarType::Array { length: 2, .. }
        ));
        assert!(matches!(
            binding.value,
            ScalarExpression::ArrayLiteral { .. }
        ));

        let count = validate_text("%%start\nu8[2] bytes = [1];\n%%end");
        assert!(count.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "array literal element count does not match fixed array length"
        }));
        let element = validate_text("%%start\nu8[1] bytes = [300];\n%%end");
        assert!(element
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0010"));
    }

    #[test]
    fn rejects_invalid_fixed_array_lengths_at_the_complete_length_span() {
        for length in ["-1", "18446744073709551616"] {
            let text = format!("%%start\nu8[{length}] value = [0];\n%%end");
            let validation = validate_text(&text);
            assert_eq!(
                validation.diagnostics.len(),
                1,
                "{:?}",
                validation.diagnostics
            );
            let diagnostic = &validation.diagnostics[0];
            assert_eq!(diagnostic.code, "B0003");
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(
                    text.find(length).expect("length") as u32,
                    (text.find(length).expect("length") + length.len()) as u32,
                )
            );
            let ScalarItem::Binding(binding) = &validation.program.items[0] else {
                panic!("array binding")
            };
            assert_eq!(binding.declared_type, ScalarType::Error);
        }
    }

    #[test]
    fn fixed_array_layout_overflow_is_a_single_diagnostic_not_a_panic() {
        let text =
            "%%start\nstruct Huge {\n\tu128[18446744073709551615] values;\n\tu32 later;\n}\n%%end";
        let validation = validate_text(text);
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        assert_eq!(validation.diagnostics[0].code, "B0003");
        let length = "18446744073709551615";
        assert_eq!(
            validation.diagnostics[0].labels[0].span.range,
            ByteSpan::new(
                text.find(length).expect("length") as u32,
                (text.find(length).expect("length") + length.len()) as u32,
            )
        );
        let huge = &validation.program.structs[0];
        assert_eq!(huge.layout, None);
        assert_eq!(huge.fields[0].layout, None);
        assert_eq!(huge.fields[0].offset, None);
        assert_eq!(huge.fields[1].layout, None);
        assert_eq!(huge.fields[1].offset, None);
        assert!(crate::emit_scalar_llvm(&validation).is_err());

        let module = ScalarModule::from_program(validation.program, Vec::new());
        let project = validate_scalar_project(ScalarProject::new(vec![module], vec![source()]));
        assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
        assert_eq!(project.diagnostics[0].code, "B0003");
    }

    #[test]
    fn aggregate_layout_addition_overflow_is_a_single_diagnostic_with_absent_layouts() {
        let text =
            "%%start\nstruct Huge {\n\tu8[18446744073709551615] values;\n\tu8 later;\n}\n%%end";
        let validation = validate_text(text);
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        let diagnostic = &validation.diagnostics[0];
        assert_eq!(diagnostic.code, "B0003");
        assert_eq!(diagnostic.message, "struct layout exceeds u64");
        let later = "later";
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(
                text.find(later).expect("later field") as u32,
                (text.find(later).expect("later field") + later.len()) as u32,
            )
        );
        let huge = &validation.program.structs[0];
        assert_eq!(huge.layout, None);
        assert!(huge.fields.iter().all(|field| field.layout.is_none()));
        assert!(huge.fields.iter().all(|field| field.offset.is_none()));
        assert!(crate::emit_scalar_llvm(&validation).is_err());

        let source = source();
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                module_from_text(source.clone(), text),
                Vec::new(),
            )],
            vec![source],
        ));
        assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
        assert_eq!(project.diagnostics[0].code, "B0003");
        assert_eq!(project.diagnostics[0].message, "struct layout exceeds u64");
        let huge = &project.project.modules[0].structs[0];
        assert_eq!(huge.layout, None);
        assert!(huge.fields.iter().all(|field| field.layout.is_none()));
        assert!(huge.fields.iter().all(|field| field.offset.is_none()));
        assert!(crate::emit_scalar_project_llvm(&project).is_err());
    }

    #[test]
    fn rejects_direct_extern_fixed_array_inputs_and_outputs() {
        let text =
            "%%start\nenv = extern wasm \"env\" { unit(u8[2]) consume; u8[2]() produce; };\n%%end";
        let validation = validate_text(text);
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message == "direct FFI arrays are not supported")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "B0003"));
    }

    #[test]
    fn resolves_transitive_local_aggregate_layout_absence() {
        let text = "%%start
struct Outer {
	Middle middle;
}
struct Middle {
	Huge huge;
}
struct Huge {
	u8[18446744073709551615] values;
	u8 later;
}
%%end";
        let validation = validate_text(text);
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        assert_eq!(
            validation.diagnostics[0].message,
            "struct layout exceeds u64"
        );
        assert_eq!(
            validation.diagnostics[0].labels[0].span.range,
            ByteSpan::new(
                text.rfind("later").expect("later field") as u32,
                (text.rfind("later").expect("later field") + "later".len()) as u32,
            )
        );
        assert!(validation.program.structs.iter().all(|structure| {
            structure.layout.is_none()
                && structure
                    .fields
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none())
        }));
        assert!(crate::emit_scalar_llvm(&validation).is_err());
    }

    #[test]
    fn resolves_transitive_project_aggregate_layout_absence() {
        let huge_source = module_source("src/huge.w");
        let huge = module_from_text(
            huge_source.clone(),
            "%%start
struct Huge {
	u8[18446744073709551615] values;
	u8 later;
}
%%end",
        );
        let middle_source = module_source("src/middle.w");
        let middle = module_from_text(
            middle_source.clone(),
            "%%start
huge = namespace app \"src/huge.w\";
struct Middle {
	huge.Huge value;
}
%%end",
        );
        let outer_source = module_source("src/main.w");
        let outer = module_from_text(
            outer_source.clone(),
            "%%start
middle = namespace app \"src/middle.w\";
struct Outer {
	middle.Middle value;
}
%%end",
        );
        let outer_namespace_span = match &outer.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("outer namespace"),
        };
        let middle_namespace_span = match &middle.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("middle namespace"),
        };
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    outer,
                    vec![ScalarNamespaceBinding {
                        binding: "middle".to_owned(),
                        target: middle_source.clone(),
                        span: outer_namespace_span,
                    }],
                ),
                ScalarModule::from_program(
                    middle,
                    vec![ScalarNamespaceBinding {
                        binding: "huge".to_owned(),
                        target: huge_source.clone(),
                        span: middle_namespace_span,
                    }],
                ),
                ScalarModule::from_program(huge, Vec::new()),
            ],
            vec![outer_source, middle_source, huge_source],
        ));
        assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
        assert_eq!(project.diagnostics[0].message, "struct layout exceeds u64");
        assert!(project.project.modules.iter().all(|module| {
            module.structs.iter().all(|structure| {
                structure.layout.is_none()
                    && structure
                        .fields
                        .iter()
                        .all(|field| field.layout.is_none() && field.offset.is_none())
            })
        }));
        assert!(crate::emit_scalar_project_llvm(&project).is_err());
    }

    #[test]
    fn rejects_recursive_by_value_struct_layouts_once_per_component() {
        for text in [
            "%%start
struct Node {
	Node next;
}
%%end",
            "%%start
struct First {
	Second second;
}
struct Second {
	First first;
}
%%end",
        ] {
            let validation = validate_text(text);
            assert_eq!(
                validation.diagnostics.len(),
                1,
                "{:?}",
                validation.diagnostics
            );
            assert_eq!(
                validation.diagnostics[0].message,
                "recursive by-value struct layout is unsupported"
            );
            assert!(validation.program.structs.iter().all(|structure| {
                structure.layout.is_none()
                    && structure
                        .fields
                        .iter()
                        .all(|field| field.layout.is_none() && field.offset.is_none())
            }));
        }
    }

    #[test]
    fn rejects_three_member_recursive_layout_with_duplicate_participating_edges_once() {
        let text = "%%start
struct First {
	Second first_second;
	Second second_second;
}
struct Second {
	Third third;
}
struct Third {
	First first;
}
%%end";
        let validation = validate_text(text);
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "recursive by-value struct layout is unsupported"
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "{:?}", validation.diagnostics);
        let first_field = "first_second";
        let start = text.find(first_field).expect("first participating field") as u32;
        assert_eq!(
            diagnostics[0].labels[0].span.range,
            ByteSpan::new(start, start + first_field.len() as u32)
        );
        assert!(validation.program.structs.iter().all(|structure| {
            structure.layout.is_none()
                && structure
                    .fields
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none())
        }));
    }

    #[test]
    fn rejects_nested_array_recursive_layout_while_raw_pointers_and_callables_are_non_edges() {
        let text = "%%start
struct Node {
	*?Node parent;
	unit(Node) visit;
	Node[1][1] children;
}
%%end";
        let validation = validate_text(text);
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "recursive by-value struct layout is unsupported"
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "{:?}", validation.diagnostics);
        let field = "children";
        let start = text.find(field).expect("recursive field") as u32;
        assert_eq!(
            diagnostics[0].labels[0].span.range,
            ByteSpan::new(start, start + field.len() as u32)
        );
        let node = &validation.program.structs[0];
        assert_eq!(node.layout, None);
        assert!(node
            .fields
            .iter()
            .all(|field| field.layout.is_none() && field.offset.is_none()));
    }

    #[test]
    fn reports_recursive_and_overflowing_connected_aggregate_layouts_independently() {
        let text = "%%start
struct Root {
	Recursive recursive;
	Huge huge;
}
struct Recursive {
	Recursive member;
}
struct Huge {
	u8[18446744073709551615] values;
	u8 later;
}
%%end";
        let validation = validate_text(text);
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
        assert_eq!(
            diagnostics[0].message,
            "recursive by-value struct layout is unsupported"
        );
        assert_eq!(diagnostics[1].message, "struct layout exceeds u64");
        for (diagnostic, field) in diagnostics.iter().zip(["member", "later"]) {
            let start = text.rfind(field).expect("diagnostic field") as u32;
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + field.len() as u32)
            );
        }
        assert!(validation.program.structs.iter().all(|structure| {
            structure.layout.is_none()
                && structure
                    .fields
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none())
        }));
        assert!(crate::emit_scalar_llvm(&validation).is_err());
    }

    #[test]
    fn reports_local_recursive_layout_components_in_source_order() {
        let text = "%%start
struct Root {
	Late late;
}
struct Early {
	Early early;
}
struct Late {
	Late late;
}
%%end";
        let validation = validate_text(text);
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "recursive by-value struct layout is unsupported"
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
        for (diagnostic, field) in diagnostics.iter().zip(["early", "late"]) {
            let start = text.rfind(field).expect("recursive field") as u32;
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + field.len() as u32)
            );
        }
        for structure in &validation.program.structs[1..] {
            assert_eq!(structure.layout, None);
            assert!(structure
                .fields
                .iter()
                .all(|field| { field.layout.is_none() && field.offset.is_none() }));
        }
    }

    #[test]
    fn reports_project_recursive_layout_components_in_source_order() {
        let main_source = module_source("src/main.w");
        let main_text = "%%start
late = namespace app \"src/late.w\";
struct Root {
	late.Late late;
}
struct Early {
	Early early;
}
%%end";
        let main = module_from_text(main_source.clone(), main_text);
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("late namespace"),
        };
        let late_source = module_source("src/late.w");
        let late_text = "%%start
struct Late {
	Late late;
}
%%end";
        let late = module_from_text(late_source.clone(), late_text);
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main,
                    vec![ScalarNamespaceBinding {
                        binding: "late".to_owned(),
                        target: late_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::from_program(late, Vec::new()),
            ],
            vec![main_source.clone(), late_source.clone()],
        ));
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "recursive by-value struct layout is unsupported"
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
        for (diagnostic, (source, text, field)) in diagnostics.iter().zip([
            (&main_source, main_text, "early"),
            (&late_source, late_text, "late"),
        ]) {
            let start = text.find(field).expect("recursive field") as u32;
            assert_eq!(diagnostic.labels[0].span.source, *source);
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + field.len() as u32)
            );
        }
        for structure in [
            &validation.project.modules[0].structs[1],
            &validation.project.modules[1].structs[0],
        ] {
            assert_eq!(structure.layout, None);
            assert!(structure
                .fields
                .iter()
                .all(|field| { field.layout.is_none() && field.offset.is_none() }));
        }
    }

    #[test]
    fn reports_sibling_import_recursive_layout_components_in_import_observation_order() {
        let root_source = module_source("src/main.w");
        let root = module_from_text(
            root_source.clone(),
            "%%start
zeta = namespace app \"src/zeta.w\";
alpha = namespace app \"src/alpha.w\";
%%end",
        );
        let zeta_source = module_source("src/zeta.w");
        let zeta_text = "%%start
struct Zeta {
	Zeta zeta;
}
%%end";
        let zeta = module_from_text(zeta_source.clone(), zeta_text);
        let alpha_source = module_source("src/alpha.w");
        let alpha_text = "%%start
struct Alpha {
	Alpha alpha;
}
%%end";
        let alpha = module_from_text(alpha_source.clone(), alpha_text);
        let zeta_namespace = match &root.items[0] {
            ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
            _ => panic!("zeta namespace"),
        };
        let alpha_namespace = match &root.items[1] {
            ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
            _ => panic!("alpha namespace"),
        };
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    root,
                    vec![
                        ScalarNamespaceBinding {
                            binding: zeta_namespace.0,
                            target: zeta_source.clone(),
                            span: zeta_namespace.1,
                        },
                        ScalarNamespaceBinding {
                            binding: alpha_namespace.0,
                            target: alpha_source.clone(),
                            span: alpha_namespace.1,
                        },
                    ],
                ),
                ScalarModule::from_program(zeta, Vec::new()),
                ScalarModule::from_program(alpha, Vec::new()),
            ],
            vec![root_source, zeta_source.clone(), alpha_source.clone()],
        ));
        let diagnostics = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "recursive by-value struct layout is unsupported"
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
        for (diagnostic, (source, text, field)) in diagnostics.iter().zip([
            (&zeta_source, zeta_text, "zeta"),
            (&alpha_source, alpha_text, "alpha"),
        ]) {
            let start = text.rfind(field).expect("recursive field") as u32;
            assert_eq!(diagnostic.labels[0].span.source, *source);
            assert_eq!(
                diagnostic.labels[0].span.range,
                ByteSpan::new(start, start + field.len() as u32)
            );
        }
        for module in &validation.project.modules[1..] {
            assert!(module.structs.iter().all(|structure| {
                structure.layout.is_none()
                    && structure
                        .fields
                        .iter()
                        .all(|field| field.layout.is_none() && field.offset.is_none())
            }));
        }
    }

    #[test]
    fn absent_local_struct_layout_stops_containing_layout_accumulation() {
        let text = "%%start\nstruct Outer {\n\tHuge huge;\n\tu32 later;\n}\nstruct Huge {\n\tu128[18446744073709551615] values;\n}\n%%end";
        let validation = validate_text(text);
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        assert_eq!(validation.diagnostics[0].code, "B0003");
        let length = "18446744073709551615";
        assert_eq!(
            validation.diagnostics[0].labels[0].span.range,
            ByteSpan::new(
                text.find(length).expect("length") as u32,
                (text.find(length).expect("length") + length.len()) as u32,
            )
        );
        let outer = &validation.program.structs[0];
        assert_eq!(outer.layout, None);
        assert_eq!(outer.fields[0].layout, None);
        assert_eq!(outer.fields[0].offset, None);
        assert_eq!(outer.fields[1].layout, None);
        assert_eq!(outer.fields[1].offset, None);
        let huge = &validation.program.structs[1];
        assert_eq!(huge.layout, None);
        assert_eq!(huge.fields[0].layout, None);
        assert_eq!(huge.fields[0].offset, None);
    }

    #[test]
    fn absent_project_struct_layout_propagates_without_a_duplicate_diagnostic() {
        let library_source = module_source("src/library.w");
        let library_text = "%%start\nstruct Huge {\n\tu128[18446744073709551615] values;\n}\n%%end";
        let library = module_from_text(library_source.clone(), library_text);
        let main_source = module_source("src/main.w");
        let main = module_from_text(
            main_source.clone(),
            "%%start\nlibrary = namespace app \"src/library.w\";\nstruct Outer {\n\tlibrary.Huge huge;\n\tu32 later;\n}\n%%end",
        );
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main,
                    vec![ScalarNamespaceBinding {
                        binding: "library".to_owned(),
                        target: library_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::from_program(library, Vec::new()),
            ],
            vec![main_source, library_source],
        ));
        assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
        assert_eq!(project.diagnostics[0].code, "B0003");
        let length = "18446744073709551615";
        assert_eq!(
            project.diagnostics[0].labels[0].span.range,
            ByteSpan::new(
                library_text.find(length).expect("length") as u32,
                (library_text.find(length).expect("length") + length.len()) as u32,
            )
        );
        let outer = &project.project.modules[0].structs[0];
        assert_eq!(outer.layout, None);
        assert_eq!(outer.fields[0].layout, None);
        assert_eq!(outer.fields[0].offset, None);
        assert_eq!(outer.fields[1].layout, None);
        assert_eq!(outer.fields[1].offset, None);
        let huge = &project.project.modules[1].structs[0];
        assert_eq!(huge.layout, None);
        assert_eq!(huge.fields[0].layout, None);
        assert_eq!(huge.fields[0].offset, None);
        assert!(crate::emit_scalar_project_llvm(&project).is_err());
    }

    #[test]
    fn error_type_is_not_a_valid_equal_type() {
        assert!(!scalar_type_equal(&ScalarType::Error, &ScalarType::Error));
        assert!(!scalar_type_equal(&ScalarType::Error, &ScalarType::U8));
    }

    #[test]
    fn invalid_fixed_array_length_is_rejected_before_llvm_emission() {
        let validation = validate_text("%%start\nu8[-1] value = [0];\n%%end");
        assert!(crate::emit_scalar_llvm(&validation).is_err());
    }

    #[test]
    fn validates_scoped_fixed_array_callable_transport_and_preserves_module_collision() {
        let valid = validate_text(
            "%%start\nu8[2](u8[2]) transport = fn(bytes) { bytes };\nunit(u8) observe = fn(value) { value; };\nunit() run = fn { u8[2] bytes = [7, 9]; transport(bytes); observe(1); };\n%%end",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let ScalarItem::Function(transport) = &valid.program.items[0] else {
            panic!("transport function")
        };
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &transport.signature
        else {
            panic!("transport callable signature")
        };
        assert_eq!(outputs.outputs.len(), 1);
        assert_eq!(parameters.len(), 1);
        assert!(matches!(
            &outputs.outputs[0].ty,
            ScalarType::Array {
                element,
                length: 2,
                ..
            } if **element == ScalarType::U8
        ));
        assert!(matches!(
            &parameters[0],
            ScalarType::Array {
                element,
                length: 2,
                ..
            } if **element == ScalarType::U8
        ));

        let source = source();
        let invalid = module_from_text(
            source.clone(),
            "%%start\nu8[2] bytes = [1, 2];\nu8[2](u8[2]) transport = fn(bytes) { bytes };\n%%end",
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(source.clone(), invalid.items, Vec::new())],
            vec![source],
        ));
        assert!(
            project
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0002"),
            "{:?}",
            project.diagnostics
        );
    }
}
