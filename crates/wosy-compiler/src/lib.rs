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
    ScalarBlockItem, ScalarEnum, ScalarEnumId, ScalarEnumVariant, ScalarExpression, ScalarExtern,
    ScalarExternFunction, ScalarFunction, ScalarInitializationNode, ScalarItem, ScalarLayout,
    ScalarModule, ScalarNamespaceBinding, ScalarOutput, ScalarOutputReceiver, ScalarOutputSequence,
    ScalarPlace, ScalarProgram, ScalarProject, ScalarProjectValidation, ScalarStruct,
    ScalarStructField, ScalarStructFieldId, ScalarStructId, ScalarStructLiteral,
    ScalarStructLiteralField, ScalarTargetLayout, ScalarType, ScalarValidation, ScalarWhile,
    UnaryOperator,
};

mod llvm;

pub const COMPILER_IDENTITY: &str = concat!("wosy-compiler-", env!("CARGO_PKG_VERSION"));

pub fn core_runtime(backend: &str, output: &str) -> Result<&'static str, String> {
    match (backend, output) {
        ("llvm", "native") => Ok(include_str!("../runtime/native64.ll")),
        ("llvm", "wasm-wasip1" | "wasm-wasip2") => Ok(include_str!("../runtime/wasm32.ll")),
        (backend, output) => Err(format!(
            "unsupported core runtime target backend {backend} with output {output}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::ScalarNamespaceBinding;
    use super::{
        core_runtime, derive_scalar_program, derive_scalar_program_from_cst,
        emit_scalar_project_llvm, parse_source, validate_scalar_project, ScalarModule,
        ScalarProject,
    };
    use wosy_syntax::{ByteSpan, CanonicalCstRoot, CanonicalCstSnapshot, SourceIdentity};

    #[test]
    fn derives_core_runtime_from_llvm_output() {
        assert!(core_runtime("llvm", "native")
            .expect("native runtime")
            .contains("__wosy_core_alloc"));
        assert!(core_runtime("llvm", "wasm-wasip1")
            .expect("WASI preview 1 runtime")
            .contains("wasm32-wasi"));
        assert!(core_runtime("llvm", "wasm-wasip2")
            .expect("WASI preview 2 runtime")
            .contains("wasm32-wasi"));
    }

    #[test]
    fn reports_unsupported_runtime_target() {
        assert_eq!(
            core_runtime("custom", "artifact"),
            Err("unsupported core runtime target backend custom with output artifact".to_owned())
        );
    }

    #[test]
    fn reports_only_the_trailing_while_semicolon_syntax_diagnostic() {
        let text = "%%start\ni32(i32) loop = fn(start) {\n\ti32 value = start;\n\twhile (value < 3) {\n\t\tvalue = value + 1;\n\t};\n};\n%%end";
        let output = parse_source(
            SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/main.w".into(),
                "r1".into(),
            ),
            text.into(),
            &[],
        );

        assert_eq!(output.diagnostics.len(), 1);
        let semicolon = text.find("\n\t};\n").expect("trailing semicolon") as u32 + 3;
        assert_eq!(
            output.diagnostics[0].labels[0].span.range,
            ByteSpan::new(semicolon, semicolon + 1)
        );
    }

    #[test]
    fn validates_ordinary_read_line_source_in_scalar() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let text = "%%start\nstruct utf8 { *?u8 data; u64 length; }\nenum ReadLineStatus { line; eof; }\n(*?utf8, ReadLineStatus)(u64) read_line = fn(max_bytes) {\n\tu8[max_bytes] bytes;\n\tbool reading = true;\n\tReadLineStatus status = ReadLineStatus::line;\n\twhile (reading) {\n\t\tif (reading) { bytes[0] = 13; reading = false; } else { status = ReadLineStatus::eof; };\n\t}\n\tnull, status\n};\n%%end";
        let output = parse_source(source, text.into(), &[]);

        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let scalar = derive_scalar_program(&output.result);
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
    }
    #[test]
    fn lowers_a_valid_read_line_loop_shape() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let text = "%%start
struct utf8 { *?u8 data; u64 length; }
enum ReadLineStatus { line; eof; }
(*?utf8, ReadLineStatus)(u64) read_line = fn(max_bytes) {
	u8[max_bytes] bytes;
	bool reading = true;
	ReadLineStatus status = ReadLineStatus::line;
	while (reading) {
		if (reading) { bytes[0] = 13; reading = false; } else { status = ReadLineStatus::eof; };
	}
	null, status
};
%%end";
        let output = parse_source(source.clone(), text.into(), &[]);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let scalar = derive_scalar_program(&output.result);
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(scalar.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        emit_scalar_project_llvm(&validation).expect("read_line loop project LLVM");
    }

    #[test]
    fn archived_decomposed_read_line_snapshot_deserializes_and_reaches_project_validation() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let output = parse_source(
            source.clone(),
            include_str!("../../../stdlib/src/bootstrap.w").into(),
            &[],
        );
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);

        let serialized = serde_json::to_vec(&output.result.canonical_cst().snapshot())
            .expect("serialize canonical CST snapshot");
        let snapshot: CanonicalCstSnapshot =
            serde_json::from_slice(&serialized).expect("deserialize canonical CST snapshot");
        let canonical = CanonicalCstRoot::from_snapshot(snapshot);
        assert_eq!(
            canonical.root.text().to_string(),
            output.result.reconstruct()
        );

        let scalar = derive_scalar_program_from_cst(&canonical);
        let preview_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/wasi/preview1.w".into(),
            "r1".into(),
        );
        let preview = parse_source(
            preview_source.clone(),
            include_str!("../../../stdlib/src/wasi/preview1.w").into(),
            &[],
        );
        assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
        let preview_scalar = derive_scalar_program(&preview.result);
        let _validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    scalar.program,
                    vec![ScalarNamespaceBinding {
                        binding: "preview1".into(),
                        target: preview_source.clone(),
                        span: ByteSpan::new(0, 0),
                    }],
                ),
                ScalarModule::from_program(preview_scalar.program, Vec::new()),
            ],
            vec![source, preview_source],
        ));
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
