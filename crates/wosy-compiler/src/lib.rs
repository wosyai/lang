use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use wosy_syntax::{
    ByteSpan, CstNode, CstToken, ParseResult, SourceIdentity, SourceSpan, SyntaxKind, TypedComment,
};

mod scalar;
pub use llvm::{
    emit_scalar_build_project_llvm, emit_scalar_build_project_llvm_text,
    emit_scalar_build_single_module_llvm, emit_scalar_build_single_module_llvm_text,
    emit_scalar_llvm, emit_scalar_llvm_text, emit_scalar_project_llvm, LlvmFunction, LlvmPartition,
    LlvmValueType,
};
pub use scalar::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, derive_scalar_diagnostics_from_cst,
    derive_scalar_program, derive_scalar_program_from_cst,
    derive_scalar_program_from_cst_with_layout, derive_scalar_program_with_layout,
    prepare_scalar_project, validate_scalar_project, BinaryOperator, PreparedScalarProject,
    ResolvedAssignmentOutputId, ScalarAllocationIdentity, ScalarAssignment, ScalarAssignmentTyping,
    ScalarAutomaticReturnResult, ScalarAutomaticReturnResultState, ScalarBinding, ScalarBlock,
    ScalarBlockItem, ScalarBuildValidation, ScalarEnum, ScalarEnumId, ScalarEnumVariant,
    ScalarExpression, ScalarExtern, ScalarExternFunction, ScalarFunction, ScalarInitializationNode,
    ScalarItem, ScalarLayout, ScalarModule, ScalarNamespaceBinding, ScalarOutput,
    ScalarOutputReceiver, ScalarOutputSequence, ScalarPlace, ScalarPreparationInputs,
    ScalarProgram, ScalarProject, ScalarProjectPreparationRejection, ScalarProjectValidation,
    ScalarSourceDeclarations, ScalarStruct, ScalarStructField, ScalarStructFieldId, ScalarStructId,
    ScalarStructLiteral, ScalarStructLiteralField, ScalarTargetLayout, ScalarType,
    ScalarValidation, UnaryOperator,
};

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub use scalar::{check_scalar_package, ScalarCheckerFailure, ScalarCheckerOperation, ScalarCheckerOutcome};

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
        core_runtime, derive_scalar_program, emit_scalar_llvm, emit_scalar_project_llvm,
        parse_source, validate_scalar_project, ScalarModule, ScalarProject,
    };
    use wosy_syntax::{ByteSpan, CanonicalCstRoot, CanonicalCstSnapshot, SourceIdentity};

    #[test]
    fn publication_uses_first_comment_with_an_exact_span() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let mut parsed = parse_source(
            source,
            "%%start\n# NOTE: first\nu8 value = 1;\n# NOTE: second\n%%end".to_owned(),
            &["NOTE".to_owned()],
        );
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert_eq!(parsed.result.comments.len(), 2);
        let before = super::publication(&parsed);

        parsed.result.comments.reverse();
        let mut duplicate = parsed.result.comments[1].clone();
        duplicate.raw_payload = "duplicate must not replace the first match".to_owned();
        parsed.result.comments.push(duplicate);

        let after = super::publication(&parsed);
        assert_eq!(after.root, before.root);
        assert_eq!(after.comments, parsed.result.comments);
    }

    #[test]
    fn derives_core_runtime_from_llvm_output() {
        assert!(core_runtime("llvm", "native")
            .expect("native runtime")
            .contains("__wosy_core_alloc"));
        let preview1 = core_runtime("llvm", "wasm-wasip1").expect("Preview 1 runtime");
        let preview2 = core_runtime("llvm", "wasm-wasip2").expect("Preview 2 runtime");
        assert_eq!(preview1, preview2);
        assert!(preview1.contains("target triple = \"wasm32-wasi\""));
        assert!(preview1.contains("define ptr @__wosy_core_alloc(i64"));
        assert!(preview1.contains("define void @__wosy_core_free(ptr"));
        assert!(preview1.contains("define void @__wosy_core_system_panic()"));
        assert!(preview1.lines().any(|line| {
            line.contains("icmp ule i64") && line.ends_with(", 4294967296")
        }));
        assert!(preview1.lines().any(|line| {
            line.contains("icmp ult i64") && line.ends_with(", 4294967296")
        }));
    }

    #[test]
    fn reports_unsupported_runtime_target() {
        assert_eq!(
            core_runtime("custom", "artifact"),
            Err("unsupported core runtime target backend custom with output artifact".to_owned())
        );
    }

    #[test]
    fn rejects_retired_while_block_form_with_a_syntax_diagnostic() {
        let text = "%%start\nunit() repeat = fn { while (true) {} };\n%%end";
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
        assert_eq!(output.diagnostics[0].code, "S0001");
        let range = output.diagnostics[0].labels[0].span.range;
        let start = text.find("while").unwrap() as u32;
        assert!(range.start <= start && range.end > start);
        assert_eq!(output.result.reconstruct(), text);
    }

    #[test]
    fn validates_an_ordinary_function_named_while() {
        let text = "%%start\ni32(i32) while = fn(value) { value };\ni32 result = while(2);\n%%end";
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
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let scalar = derive_scalar_program(&output.result);
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
        emit_scalar_llvm(&scalar).expect("ordinary named call LLVM");
    }

    #[test]
    fn validates_ordinary_read_line_source_in_scalar() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/bootstrap.w".into(),
            "r1".into(),
        );
        let text = "%%start\nstruct utf8 { *?u8 data; u64 length; }\nenum ReadLineStatus { line; eof; }\n(*?utf8, ReadLineStatus)(u64) read_line = fn(max_bytes) {\n\tu8[max_bytes] bytes;\n\tbool reading = true;\n\tReadLineStatus status = ReadLineStatus::line;\n\tif (reading) {\n\t\tif (reading) { bytes[0] = 13; reading = false; } else { status = ReadLineStatus::eof; };\n\t};\n\tnull, status\n};\n%%end";
        let output = parse_source(source, text.into(), &[]);

        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let scalar = derive_scalar_program(&output.result);
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
    }
    #[test]
    fn lowers_a_valid_read_line_branch_shape() {
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
	if (reading) {
		if (reading) { bytes[0] = 13; reading = false; } else { status = ReadLineStatus::eof; };
	};
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
        emit_scalar_project_llvm(&validation).expect("read_line branch project LLVM");
    }

    #[test]
    fn archived_decomposed_read_line_snapshot_deserializes_and_reaches_project_validation() {
        use super::{
            analyze_scalar_build, derive_scalar_declarations_from_cst, prepare_scalar_project,
            ScalarPreparationInputs, ScalarTargetLayout,
        };

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
        let mut deserializer = serde_json::Deserializer::from_slice(&serialized);
        deserializer.disable_recursion_limit();
        let snapshot = <CanonicalCstSnapshot as serde::Deserialize>::deserialize(&mut deserializer)
            .expect("deserialize canonical CST snapshot");
        deserializer.end().expect("canonical CST snapshot ends");
        let canonical = CanonicalCstRoot::from_snapshot(snapshot);
        assert_eq!(
            canonical.root.text().to_string(),
            output.result.reconstruct()
        );

        let bootstrap_declarations = derive_scalar_declarations_from_cst(&canonical);
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
        let preview_declarations =
            derive_scalar_declarations_from_cst(preview.result.canonical_cst());
        let prepared = prepare_scalar_project(
            vec![
                (
                    bootstrap_declarations,
                    vec![ScalarNamespaceBinding {
                        binding: "preview1".into(),
                        target: preview_source.clone(),
                        span: ByteSpan::new(0, 0),
                    }],
                ),
                (preview_declarations, Vec::new()),
            ],
            vec![source, preview_source],
            ScalarPreparationInputs {
                target_layout: ScalarTargetLayout::WASM32,
            },
        )
        .unwrap_or_else(|error| panic!("prepare archived project: {:?}", error.diagnostics));
        let validation = analyze_scalar_build(&prepared);
        assert!(
            validation.diagnostics().is_empty(),
            "{:?}",
            validation.diagnostics()
        );
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
    let mut comments_by_span = HashMap::with_capacity(output.result.comments.len());
    for comment in &output.result.comments {
        comments_by_span.entry(comment.span).or_insert(comment);
    }
    CstPublication {
        source: output.result.source.clone(),
        source_revision: output.result.source.revision.clone(),
        root: project_node(&output.result.root, &comments_by_span, "0"),
        comments: output.result.comments.clone(),
        diagnostics: output.diagnostics.clone(),
    }
}

fn project_node(
    node: &CstNode,
    comments: &HashMap<ByteSpan, &TypedComment>,
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
    comments: &HashMap<ByteSpan, &TypedComment>,
    selector: &str,
) -> CstElement {
    let token_range = token.text_range();
    let exact_span = ByteSpan::new(u32::from(token_range.start()), u32::from(token_range.end()));
    let comment = comments.get(&exact_span);
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
