//! Ordinary compiler preparation of a closed scalar comptime fragment.
use super::{
    analyze_scalar_build, collect_scalar_diagnostics, derive_block, derive_type, prepare_scalar_project, span_of,
    typecheck::integer_literal_fits_type, BinaryOperator, ScalarBlock, ScalarBlockItem, ScalarExpression,
    ScalarFunction, ScalarItem, ScalarOutput, ScalarOutputSequence, ScalarPreparationInputs, ScalarReferenceCfg,
    ScalarSourceDeclarations, ScalarTargetLayout, ScalarType, UnaryOperator,
};
use crate::{emit_scalar_build_single_module_artifact_llvm, Diagnostic, LlvmPartition};
use wosy_syntax::{
    byte_span, ByteSpan, CanonicalCstRoot, ComptimeCst, CstNode, FreshParseResult, SourceSpan, SyntaxKind,
};

const MAX_SOURCE_BYTES: usize = 1 << 16;
const FUNCTION: &str = "evaluate";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeParameter {
    Integer(i64),
    Boolean(bool),
    UnsupportedValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeParameterRejection {
    Missing,
    UnsupportedDefault,
    DefaultType,
    UnsupportedProfileValue,
}

#[derive(Debug)]
pub enum ClosedComptimeRejection {
    InvalidParse,
    SourceLimit,
    UnsupportedLayout,
    SiteNotFound,
    RepeatedSite(SourceSpan),
    Unsupported { site: SourceSpan, feature: &'static str },
    Parameter { site: SourceSpan, name: String, reason: ComptimeParameterRejection },
    OrdinaryDiagnostics(Vec<Diagnostic>),
    Emitter(String),
}

/// One real, compiler-validated function with no ambient bindings. Not a
/// selected specialization, runner result, artifact or cache identity.
pub struct ClosedComptimeFragment {
    id: String,
    site: SourceSpan,
    result_type: ScalarType,
    llvm: LlvmPartition,
}

impl ClosedComptimeFragment {
    pub fn id(&self) -> &str { &self.id }
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn result_type(&self) -> &ScalarType { &self.result_type }
    pub fn llvm(&self) -> &LlvmPartition { &self.llvm }
}

/// The first opt-in fragment is the entire initializer of one explicit
/// root binding. Its pure scalar body has no dependencies, function calls,
/// assignments, captures or runner access. Bare parameter calls have an
/// ordinary contextual type at a local binding or the final output.
/// Arithmetic, shifts and ordered comparisons are not admitted.
pub fn derive_closed_comptime_fragment(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
    parameter: impl Fn(&str) -> Option<ComptimeParameter>,
) -> Result<ClosedComptimeFragment, ClosedComptimeRejection> {
    let parsed = fresh.result();
    if !matches!(layout, ScalarTargetLayout::NATIVE64 | ScalarTargetLayout::WASM32) {
        return Err(ClosedComptimeRejection::UnsupportedLayout);
    }
    if !parsed.is_valid() { return Err(ClosedComptimeRejection::InvalidParse); }
    if parsed.text.len() > MAX_SOURCE_BYTES { return Err(ClosedComptimeRejection::SourceLimit); }
    let scope = |range| SourceSpan::new(parsed.source.clone(), range);
    let unsupported = |range, feature| ClosedComptimeRejection::Unsupported { site: scope(range), feature };
    let mut matches = parsed.root.descendants().filter_map(|node| {
        let site = ComptimeCst::from_node(&node)?;
        (site.id.text() == id).then_some((node, site))
    });
    let (node, site) = matches.next().ok_or(ClosedComptimeRejection::SiteNotFound)?;
    if let Some((_, duplicate)) = matches.next() {
        return Err(ClosedComptimeRejection::RepeatedSite(scope(duplicate.span)));
    }
    let declaration = root_receiver_type(&node)
        .ok_or_else(|| unsupported(site.span, "expected one explicit root binding"))?;
    let result_type = derive_type(&declaration);
    if !supported_type(&result_type) { return Err(unsupported(byte_span(&declaration), "result type")); }
    // Guard only constructs that are not in this projection and cannot
    // safely enter ordinary AST extraction as a detached scalar block.
    for child in site.block.descendants() {
        match child.kind() {
            SyntaxKind::ComptimeExpr => return Err(unsupported(byte_span(&child), "nested comptime")),
            SyntaxKind::Assignment => return Err(unsupported(byte_span(&child), "assignment")),
            SyntaxKind::LocalBinding if single_binding_type(&child).is_none() =>
                return Err(unsupported(byte_span(&child), "local binding must have one typed value")),
            _ => {}
        }
    }
    let block_cst = CanonicalCstRoot::new(parsed.source.clone(), site.block.clone());
    let (diagnostics, _) = collect_scalar_diagnostics(&block_cst, |_| {});
    if !diagnostics.is_empty() { return Err(ClosedComptimeRejection::OrdinaryDiagnostics(diagnostics)); }
    let mut body = derive_block(&site.block);
    rewrite_body(&mut body, &result_type, &scope, &parameter)?;
    let output_span = body.final_output_values[0].span;
    let name_span = super::token_span(&site.id);
    let function = ScalarFunction {
        name: FUNCTION.into(), name_span, span: site.span,
        signature: ScalarType::Callable {
            parameters: Vec::new(),
            outputs: ScalarOutputSequence {
                outputs: vec![ScalarOutput { ty: result_type.clone(), span: output_span }],
                span: output_span,
            },
        },
        parameters: Vec::new(), parameter_spans: Vec::new(), generic_parameters: Vec::new(),
        overload_arms: Vec::new(), body, reference_cfg: ScalarReferenceCfg::default(),
    };
    let declarations = ScalarSourceDeclarations {
        source: parsed.source.clone(), items: vec![ScalarItem::Function(function)],
        structs: Vec::new(), enums: Vec::new(), diagnostics: Vec::new(),
    };
    let prepared = prepare_scalar_project(
        vec![(declarations, Vec::new())], vec![parsed.source.clone()],
        ScalarPreparationInputs { target_layout: layout },
    ).map_err(|refusal| ClosedComptimeRejection::OrdinaryDiagnostics(refusal.into_ordered_diagnostics()))?;
    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() {
        return Err(ClosedComptimeRejection::OrdinaryDiagnostics(build.diagnostics().to_vec()));
    }
    let llvm = emit_scalar_build_single_module_artifact_llvm(&build).map_err(ClosedComptimeRejection::Emitter)?;
    Ok(ClosedComptimeFragment { id: id.into(), site: scope(site.span), result_type, llvm })
}

fn root_receiver_type(node: &CstNode) -> Option<CstNode> {
    let primary = node.parent().filter(|p| p.kind() == SyntaxKind::Primary)?;
    let expression = primary.parent().filter(|p| p.kind() == SyntaxKind::Expression)?;
    let outputs = expression.parent().filter(|p| p.kind() == SyntaxKind::OutputList)?;
    let binding = outputs.parent().filter(|p| p.kind() == SyntaxKind::BindingDecl)?;
    let parent = binding.parent()?;
    let parent = if parent.kind() == SyntaxKind::Item { parent.parent()? } else { parent };
    (parent.kind() == SyntaxKind::Root).then(|| single_binding_type(&binding)).flatten()
}

fn single_binding_type(binding: &CstNode) -> Option<CstNode> {
    let receivers = binding.children().find(|n| n.kind() == SyntaxKind::ReceiverList)?;
    let outputs = binding.children().find(|n| n.kind() == SyntaxKind::OutputList)?;
    let mut receivers = receivers.children();
    let receiver = receivers.next()?.children().find(|n| n.kind() == SyntaxKind::TypeSpec)?;
    let mut values = outputs.children().filter(|n| n.kind() == SyntaxKind::Expression);
    values.next()?;
    (receivers.next().is_none() && values.next().is_none()).then_some(receiver)
}

fn supported_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::I32 | ScalarType::U32 | ScalarType::Bool)
}

fn rewrite_body(
    body: &mut ScalarBlock, result_type: &ScalarType, scope: &impl Fn(ByteSpan) -> SourceSpan,
    parameter: &impl Fn(&str) -> Option<ComptimeParameter>,
) -> Result<(), ClosedComptimeRejection> {
    let unsupported = |span, feature| ClosedComptimeRejection::Unsupported { site: scope(span), feature };
    let [output] = body.final_output_values.as_mut_slice() else {
        return Err(unsupported(body.span, "expected one final value"));
    };
    let Some((ScalarBlockItem::Expression(final_value), locals)) = body.items.split_last_mut() else {
        return Err(unsupported(body.span, "expected one final expression"));
    };
    for item in locals {
        let ScalarBlockItem::LocalBinding(binding) = item else {
            return Err(unsupported(body.span, "only immutable local bindings may precede the final value"));
        };
        if !supported_type(&binding.declared_type) { return Err(unsupported(binding.span, "binding type")); }
        if rewrite_parameter(&mut binding.value, &binding.declared_type, scope, parameter)? {
            binding.output_values[0].value = binding.value.clone();
        }
    }
    if rewrite_parameter(final_value, result_type, scope, parameter)? {
        output.value = final_value.clone();
        body.expressions[0] = final_value.clone();
    }
    output.ty = result_type.clone();
    Ok(())
}

fn rewrite_parameter(
    expression: &mut ScalarExpression, expected: &ScalarType, scope: &impl Fn(ByteSpan) -> SourceSpan,
    parameter: &impl Fn(&str) -> Option<ComptimeParameter>,
) -> Result<bool, ClosedComptimeRejection> {
    let ScalarExpression::Call { receiver: Some(receiver), name, arguments, type_arguments, span, .. } = expression
        else { admit_pure_expression(expression, scope)?; return Ok(false); };
    if receiver != "core" || name != "comp_param" {
        admit_pure_expression(expression, scope)?;
        return Ok(false);
    }
    let span = *span;
    let unsupported = |feature| ClosedComptimeRejection::Unsupported { site: scope(span), feature };
    if !type_arguments.is_empty() { return Err(unsupported("explicit parameter type argument")); }
    let ([ScalarExpression::Utf8 { value, .. }] | [ScalarExpression::Utf8 { value, .. }, _]) = arguments.as_slice() else {
        return Err(unsupported("expected a static parameter name and optional literal default"));
    };
    let name = std::str::from_utf8(value).expect("authored literal is UTF-8");
    let reject = |reason| ClosedComptimeRejection::Parameter { site: scope(span), name: name.into(), reason };
    // Validate the authored default even when the profile supplies a value.
    // It denotes an expression of the same contextual type, never a cast.
    let default = match arguments.get(1) {
        None => None,
        Some(ScalarExpression::Integer { value, .. }) if integer_literal_fits_type(value, expected) =>
            Some(ScalarExpression::Integer { value: value.clone(), span }),
        Some(ScalarExpression::Boolean { value, .. }) if *expected == ScalarType::Bool =>
            Some(ScalarExpression::Boolean { value: *value, span }),
        Some(ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. }) =>
            return Err(reject(ComptimeParameterRejection::DefaultType)),
        Some(_) => return Err(reject(ComptimeParameterRejection::UnsupportedDefault)),
    };
    *expression = match parameter(name) {
        Some(ComptimeParameter::Integer(value)) => ScalarExpression::Integer { value: value.into(), span },
        Some(ComptimeParameter::Boolean(value)) => ScalarExpression::Boolean { value, span },
        Some(ComptimeParameter::UnsupportedValue) => return Err(reject(ComptimeParameterRejection::UnsupportedProfileValue)),
        None => default.ok_or_else(|| reject(ComptimeParameterRejection::Missing))?,
    };
    Ok(true)
}

fn admit_pure_expression(
    expression: &ScalarExpression, scope: &impl Fn(ByteSpan) -> SourceSpan,
) -> Result<(), ClosedComptimeRejection> {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match expression {
            ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. } | ScalarExpression::Name { .. } => (),
            ScalarExpression::Unary { operator: UnaryOperator::LogicalNot | UnaryOperator::BitwiseNot, operand, .. } =>
                pending.push(operand),
            ScalarExpression::Binary {
                operator: BinaryOperator::Equal | BinaryOperator::NotEqual | BinaryOperator::And | BinaryOperator::Or
                    | BinaryOperator::BitAnd | BinaryOperator::BitOr | BinaryOperator::BitXor,
                left, right, ..
            } => {
                pending.extend([&**left, &**right]);
            }
            _ => return Err(ClosedComptimeRejection::Unsupported {
                site: scope(span_of(expression)), feature: "not a closed scalar expression in this fragment",
            }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wosy_syntax::SourceIdentity;

    fn fresh(source: &str) -> FreshParseResult {
        wosy_syntax::parse_with_snapshot(
            SourceIdentity::new("fixture".into(), "fixture".into(), "main.w".into(), "authored".into()),
            format!("%%start\n# WHY: café ☕\n{source}\n%%end"), &["WHY".into()],
        )
    }

    #[test]
    fn declared_parameter_diagnostics_preserve_the_authored_context() {
        let source = fresh(r#"i32 result = comptime solve { core.comp_param("limit") };"#);
        let read = |parameter| derive_closed_comptime_fragment(&source, "solve", ScalarTargetLayout::NATIVE64, |_| parameter);
        let missing = read(None).err().expect("missing declared parameter");
        let ClosedComptimeRejection::Parameter { site, name, reason } = missing else { panic!("{missing:?}"); };
        assert_eq!((name.as_str(), reason), ("limit", ComptimeParameterRejection::Missing));
        assert_eq!(&source.result().text[site.range.start as usize..site.range.end as usize], r#"core.comp_param("limit")"#);
        let error = read(Some(ComptimeParameter::Boolean(true))).err().expect("expected ordinary contextual type check");
        let ClosedComptimeRejection::OrdinaryDiagnostics(diagnostics) = error else { panic!("{error:?}"); };
        // The ordinary single-output type check labels the enclosing body.
        // Use its exact authored block instead of relabeling the compiler's diagnostic.
        let block = source.result().root.descendants().find_map(|node| ComptimeCst::from_node(&node)).unwrap().block;
        let expected = SourceSpan::new(site.source, byte_span(&block));
        assert!(expected.range.start <= site.range.start && site.range.end <= expected.range.end);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0003" && diagnostic.message == "expression type does not match expected type"
                && diagnostic.labels.iter().any(|label| label.span == expected)
        }));
        assert!(read(Some(ComptimeParameter::Integer(21))).is_ok());
    }

    #[test]
    fn pure_fragment_refuses_ambient_reads_and_nested_sites() {
        let source = fresh("i32 outside = 41;\ni32 result = comptime solve { outside };");
        assert!(matches!(
            derive_closed_comptime_fragment(&source, "solve", ScalarTargetLayout::WASM32, |_| None),
            Err(ClosedComptimeRejection::OrdinaryDiagnostics(_))
        ));
        let source = fresh("i32 result = comptime solve { comptime inner { 42 } };");
        assert!(matches!(
            derive_closed_comptime_fragment(&source, "solve", ScalarTargetLayout::WASM32, |_| None),
            Err(ClosedComptimeRejection::Unsupported { feature: "nested comptime", .. })
        ));
        let source = fresh("i32 result = 1 + comptime solve { 42 };");
        assert!(matches!(
            derive_closed_comptime_fragment(&source, "solve", ScalarTargetLayout::NATIVE64, |_| None),
            Err(ClosedComptimeRejection::Unsupported { feature: "expected one explicit root binding", .. })
        ));
    }

    #[test]
    fn no_operator_that_requires_an_unavailable_std_contract_reaches_llvm() {
        for expression in ["left + 1", "left - 1", "left * 1", "left / 1", "left % 1", "left << 1", "left >> 1", "-left"] {
            let source = fresh(&format!("i32 result = comptime solve {{ i32 left = 2; {expression} }};"));
            let refusal = derive_closed_comptime_fragment(&source, "solve", ScalarTargetLayout::WASM32, |_| None)
                .err().expect("operator without its std contract must not emit a fragment");
            let ClosedComptimeRejection::Unsupported { site, .. } = refusal else { panic!("{refusal:?}"); };
            assert_eq!(&source.result().text[site.range.start as usize..site.range.end as usize], expression);
        }
    }

    #[test]
    fn ordered_comparison_waits_for_exact_operand_signedness() {
        for operator in ["<", "<=", ">", ">="] {
            let expression = format!("left {operator} 1");
            let source = fresh(&format!("bool result = comptime solve {{ u32 left = 2; {expression} }};"));
            let refusal = derive_closed_comptime_fragment(&source, "solve", ScalarTargetLayout::NATIVE64, |_| None)
                .err().expect("ordered comparison without exact lowering must not emit a fragment");
            let ClosedComptimeRejection::Unsupported { site, .. } = refusal else { panic!("{refusal:?}"); };
            assert_eq!(&source.result().text[site.range.start as usize..site.range.end as usize], expression);
        }
    }
}
