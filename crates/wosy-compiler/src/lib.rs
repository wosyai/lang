use serde::{Deserialize, Serialize};
use wosy_syntax::{
    ByteSpan, CstNode, CstToken, ParseResult, SourceIdentity, SourceSpan, SyntaxKind,
};

mod scalar;
pub use llvm::{
    emit_scalar_llvm, emit_scalar_llvm_text, emit_scalar_project_llvm, LlvmFunction, LlvmPartition,
    LlvmValueType,
};
pub use scalar::{
    derive_scalar_diagnostics_from_cst, derive_scalar_program, derive_scalar_program_from_cst,
    derive_scalar_program_from_cst_with_layout, derive_scalar_program_with_layout,
    validate_scalar_project, BinaryOperator, ScalarAssignment, ScalarBinding, ScalarBlock,
    ScalarBlockItem, ScalarExpression, ScalarExtern, ScalarExternFunction, ScalarFunction,
    ScalarInitializationNode, ScalarItem, ScalarLayout, ScalarModule, ScalarNamespaceBinding,
    ScalarOutput, ScalarOutputReceiver, ScalarOutputSequence, ScalarPlace, ScalarProgram,
    ScalarProject, ScalarProjectValidation, ScalarStruct, ScalarStructField, ScalarStructFieldId,
    ScalarStructId, ScalarStructLiteral, ScalarStructLiteralField, ScalarTargetLayout, ScalarType,
    ScalarValidation, ScalarWhile, UnaryOperator,
};

mod llvm;

pub const COMPILER_IDENTITY: &str = concat!("wosy-compiler-", env!("CARGO_PKG_VERSION"));

pub fn core_runtime(target: &str) -> Result<&'static str, String> {
    match target {
        "native64" => Ok(include_str!("../runtime/native64.ll")),
        "wasm32" => Ok(include_str!("../runtime/wasm32.ll")),
        target => Err(format!("core runtime {target} is unsupported")),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiagnosticLabelKind {
    Primary,
    Secondary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticLabel {
    pub kind: DiagnosticLabelKind,
    pub span: SourceSpan,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CstMetadata {
    None,
    TypedComment {
        category: String,
        raw_payload: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CstElement {
    pub selector: String,
    pub kind: String,
    pub span: ByteSpan,
    pub text: String,
    pub metadata: CstMetadata,
    pub recovery: bool,
    pub children: Vec<CstElement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CstPublication {
    pub source: SourceIdentity,
    pub source_revision: String,
    pub root: CstElement,
    pub comments: Vec<wosy_syntax::TypedComment>,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct ParseOutput {
    pub result: ParseResult,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_source(
    source: SourceIdentity,
    text: String,
    configured_comments: &[String],
) -> ParseOutput {
    let result = wosy_syntax::parse(source, text, configured_comments);
    let diagnostics = result
        .errors
        .iter()
        .map(|error| Diagnostic {
            code: "S0001".to_owned(),
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            labels: vec![DiagnosticLabel {
                kind: DiagnosticLabelKind::Primary,
                span: SourceSpan::new(result.source.clone(), error.span),
                message: "syntax error originates here".to_owned(),
            }],
            notes: vec!["parse the selected source with the configured project rules".to_owned()],
        })
        .collect();
    ParseOutput {
        result,
        diagnostics,
    }
}

pub fn publication(output: &ParseOutput) -> CstPublication {
    CstPublication {
        source: output.result.source.clone(),
        source_revision: output.result.source.revision.clone(),
        root: project_node(&output.result.root, &output.result.comments, "0"),
        comments: output.result.comments.clone(),
        diagnostics: output.diagnostics.clone(),
    }
}

fn project_node(
    node: &CstNode,
    comments: &[wosy_syntax::TypedComment],
    selector: &str,
) -> CstElement {
    let span = wosy_syntax::byte_span(node);
    let children = node
        .children_with_tokens()
        .enumerate()
        .map(|(index, element)| {
            let child_selector = format!("{selector}.{index}");
            match element {
                rowan::NodeOrToken::Node(child) => project_node(&child, comments, &child_selector),
                rowan::NodeOrToken::Token(token) => {
                    project_token(&token, comments, &child_selector)
                }
            }
        })
        .collect();
    CstElement {
        selector: selector.to_owned(),
        kind: format!("{:?}", node.kind()),
        span,
        text: node.text().to_string(),
        metadata: CstMetadata::None,
        recovery: node.kind() == SyntaxKind::Error,
        children,
    }
}

fn project_token(
    token: &CstToken,
    comments: &[wosy_syntax::TypedComment],
    selector: &str,
) -> CstElement {
    let token_range = token.text_range();
    let exact_span = ByteSpan::new(u32::from(token_range.start()), u32::from(token_range.end()));
    let comment = comments.iter().find(|comment| comment.span == exact_span);
    CstElement {
        selector: selector.to_owned(),
        kind: format!("{:?}", token.kind()),
        span: exact_span,
        text: token.text().to_string(),
        metadata: comment.map_or(CstMetadata::None, |value| CstMetadata::TypedComment {
            category: value.category.clone(),
            raw_payload: value.raw_payload.clone(),
        }),
        recovery: token.kind() == SyntaxKind::Error,
        children: Vec::new(),
    }
}

pub fn serialize_publication(publication: &CstPublication) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(publication)
}
