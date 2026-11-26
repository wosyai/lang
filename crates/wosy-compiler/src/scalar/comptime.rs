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

pub const CLOSED_COMPTIME_MAX_SOURCE_BYTES: usize = 1 << 16;
const FUNCTION: &str = "evaluate";

mod direct;
pub use direct::{
    derive_direct_function_comptime_fragment, derive_direct_pair_startup_comptime_plan,
    derive_direct_startup_comptime_plan, derive_forwarded_pair_startup_comptime_plan,
    derive_pair_startup_call_graph_comptime_plan, inspect_direct_function_comptime,
    DirectComptimeArgument, DirectComptimeInput, DirectFunctionComptimeFragment, DirectStartupComptimePlan,
    DirectStartupComptimeUse, ForwardedPairStartupComptimePlan, InspectedDirectComptimeCall,
    PairStartupBranch, PairStartupBranchSelection, PairStartupBypass, PairStartupCallGraphComptimePlan,
    PairStartupCallGraphEvidence, PairStartupCallHop, PairStartupCallPath, PairStartupForwarding,
    PairStartupHopInput, PairStartupOccurrence, PairStartupSelector, StartupArgumentOrigin,
};

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

/// Only occurrences actually substituted in this compiler-derived fragment.
/// A default records absence in the profile, not an explicit override.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComptimeParameterUse {
    pub name: String,
    pub call: ByteSpan,
    pub context: ScalarType,
    pub resolved: ComptimeParameterValue,
    pub origin: ComptimeParameterOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeParameterValue {
    Integer(i64),
    Boolean(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeParameterOrigin {
    Profile,
    Default { literal: ByteSpan },
}

#[derive(Debug)]
pub enum ClosedComptimeRejection {
    InvalidParse,
    SourceLimit,
    UnsupportedLayout,
    SiteNotFound,
    RepeatedSite(SourceSpan),
    PrecededByAuthoredItem { site: SourceSpan, first: SourceSpan },
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
    layout: ScalarTargetLayout,
    parameter_uses: Vec<ComptimeParameterUse>,
    llvm: LlvmPartition,
}

impl ClosedComptimeFragment {
    pub fn id(&self) -> &str { &self.id }
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn result_type(&self) -> &ScalarType { &self.result_type }
    pub fn layout(&self) -> ScalarTargetLayout { self.layout }
    pub fn parameter_uses(&self) -> &[ComptimeParameterUse] { &self.parameter_uses }
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
    with_closed_comptime_occurrence(fresh, id, layout, SiteRequirement::RootBinding,
        parameter, |fragment, _, _, _| fragment)
}

/// Admit only the first authored item's complete initializer, for a closed
/// body with no parameter inputs. The caller must independently authenticate
/// this file as the actual target entry to interpret the startup placement.
/// No claim about other initializers, function reachability, or tool launch.
pub fn derive_first_source_initializer_comptime(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<ClosedComptimeFragment, ClosedComptimeRejection> {
    with_closed_comptime_occurrence(fresh, id, layout, SiteRequirement::FirstSourceItem,
        |_| Some(ComptimeParameter::UnsupportedValue), |fragment, _, _, _| fragment)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SiteRequirement {
    RootBinding,
    FirstSourceItem,
}

// Keep the exact Rowan occurrence with its authored root in this compiler
// invocation. The public preview returns only a fragment, not a way to
// substitute a value or to reattach it to another source/profile.
fn with_closed_comptime_occurrence<R>(
    fresh: &FreshParseResult,
    id: &str,
    layout: ScalarTargetLayout,
    requirement: SiteRequirement,
    parameter: impl Fn(&str) -> Option<ComptimeParameter>,
    inspect: impl FnOnce(ClosedComptimeFragment, &CanonicalCstRoot, &CstNode, &CstNode) -> R,
) -> Result<R, ClosedComptimeRejection> {
    let parsed = fresh.result();
    if !matches!(layout, ScalarTargetLayout::NATIVE64 | ScalarTargetLayout::WASM32) {
        return Err(ClosedComptimeRejection::UnsupportedLayout);
    }
    if !parsed.is_valid() { return Err(ClosedComptimeRejection::InvalidParse); }
    if parsed.text.len() > CLOSED_COMPTIME_MAX_SOURCE_BYTES { return Err(ClosedComptimeRejection::SourceLimit); }
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
    let RootInitializer { declaration, initializer, item, source } = root_receiver_and_initializer(&node)
        .ok_or_else(|| unsupported(site.span, "expected one explicit root binding"))?;
    if requirement == SiteRequirement::FirstSourceItem {
        // BoundaryStart and trivia have no authored initialization. Require
        // the first item itself rather than deciding if any prefix is inert.
        let first = source.children().find(|child| child.kind() != SyntaxKind::BoundaryStart)
            .ok_or_else(|| unsupported(site.span, "expected the first authored source item"))?;
        if first != item {
            return Err(ClosedComptimeRejection::PrecededByAuthoredItem {
                site: scope(site.span), first: scope(byte_span(&first)),
            });
        }
    }
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
    let (diagnostics, _) = collect_scalar_diagnostics(&block_cst, None, |_| {});
    if !diagnostics.is_empty() { return Err(ClosedComptimeRejection::OrdinaryDiagnostics(diagnostics)); }
    let mut body = derive_block(&site.block);
    let parameter_uses = rewrite_body(&mut body, &result_type, &scope, &parameter)?;
    let fragment = emit_closed_evaluator(
        id, scope(site.span), super::token_span(&site.id), layout, result_type, parameter_uses, body,
    )?;
    Ok(inspect(fragment, parsed.canonical_cst(), &node, &initializer))
}

// Every caller supplies a detached scalar body already bound to its exact
// authored inputs. Only this body enters the ordinary evaluator emitter.
fn emit_closed_evaluator(
    id: &str, site: SourceSpan, name_span: ByteSpan, layout: ScalarTargetLayout,
    result_type: ScalarType, parameter_uses: Vec<ComptimeParameterUse>, body: ScalarBlock,
) -> Result<ClosedComptimeFragment, ClosedComptimeRejection> {
    let [output] = body.final_output_values.as_slice() else {
        return Err(ClosedComptimeRejection::Emitter("closed evaluator must have one output".into()));
    };
    let output_span = output.span;
    let function = ScalarFunction {
        name: FUNCTION.into(), name_span, span: site.range,
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
        source: site.source.clone(), items: vec![ScalarItem::Function(function)],
        structs: Vec::new(), enums: Vec::new(), diagnostics: Vec::new(),
    };
    let prepared = prepare_scalar_project(
        vec![(declarations, Vec::new())], vec![site.source.clone()],
        ScalarPreparationInputs { target_layout: layout },
    ).map_err(|refusal| ClosedComptimeRejection::OrdinaryDiagnostics(refusal.into_ordered_diagnostics()))?;
    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() {
        return Err(ClosedComptimeRejection::OrdinaryDiagnostics(build.diagnostics().to_vec()));
    }
    let llvm = emit_scalar_build_single_module_artifact_llvm(&build).map_err(ClosedComptimeRejection::Emitter)?;
    Ok(ClosedComptimeFragment { id: id.into(), site, result_type, layout, parameter_uses, llvm })
}

struct RootInitializer {
    declaration: CstNode,
    initializer: CstNode,
    item: CstNode,
    source: CstNode,
}

fn root_receiver_and_initializer(node: &CstNode) -> Option<RootInitializer> {
    let primary = node.parent().filter(|p| p.kind() == SyntaxKind::Primary)?;
    let expression = primary.parent().filter(|p| p.kind() == SyntaxKind::Expression)?;
    let outputs = expression.parent().filter(|p| p.kind() == SyntaxKind::OutputList)?;
    let binding = outputs.parent().filter(|p| p.kind() == SyntaxKind::BindingDecl)?;
    let parent = binding.parent()?;
    let (item, source) = if parent.kind() == SyntaxKind::Item {
        (parent.clone(), parent.parent()?)
    } else {
        (binding.clone(), parent)
    };
    if source.kind() != SyntaxKind::Root { return None; }
    Some(RootInitializer { declaration: single_binding_type(&binding)?, initializer: expression, item, source })
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
) -> Result<Vec<ComptimeParameterUse>, ClosedComptimeRejection> {
    let unsupported = |span, feature| ClosedComptimeRejection::Unsupported { site: scope(span), feature };
    let [output] = body.final_output_values.as_mut_slice() else {
        return Err(unsupported(body.span, "expected one final value"));
    };
    let Some((ScalarBlockItem::Expression(final_value), locals)) = body.items.split_last_mut() else {
        return Err(unsupported(body.span, "expected one final expression"));
    };
    let mut uses = Vec::new();
    for item in locals {
        let ScalarBlockItem::LocalBinding(binding) = item else {
            return Err(unsupported(body.span, "only immutable local bindings may precede the final value"));
        };
        if !supported_type(&binding.declared_type) { return Err(unsupported(binding.span, "binding type")); }
        if let Some(observed) = rewrite_parameter(&mut binding.value, &binding.declared_type, scope, parameter)? {
            binding.output_values[0].value = binding.value.clone();
            uses.push(observed);
        }
    }
    if let Some(observed) = rewrite_parameter(final_value, result_type, scope, parameter)? {
        output.value = final_value.clone();
        body.expressions[0] = final_value.clone();
        uses.push(observed);
    }
    output.ty = result_type.clone();
    Ok(uses)
}

fn rewrite_parameter(
    expression: &mut ScalarExpression, expected: &ScalarType, scope: &impl Fn(ByteSpan) -> SourceSpan,
    parameter: &impl Fn(&str) -> Option<ComptimeParameter>,
) -> Result<Option<ComptimeParameterUse>, ClosedComptimeRejection> {
    let ScalarExpression::Call { receiver: Some(receiver), name, arguments, type_arguments, span, .. } = expression
        else { admit_pure_expression(expression, scope)?; return Ok(None); };
    if receiver != "core" || name != "comp_param" {
        admit_pure_expression(expression, scope)?;
        return Ok(None);
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
        Some(ScalarExpression::Integer { value, span }) if integer_literal_fits_type(value, expected) =>
            Some((ComptimeParameterValue::Integer(i64::try_from(value).expect("a supported scalar default fits i64")), *span)),
        Some(ScalarExpression::Boolean { value, span }) if *expected == ScalarType::Bool =>
            Some((ComptimeParameterValue::Boolean(*value), *span)),
        Some(ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. }) =>
            return Err(reject(ComptimeParameterRejection::DefaultType)),
        Some(_) => return Err(reject(ComptimeParameterRejection::UnsupportedDefault)),
    };
    let (resolved, origin) = match parameter(name) {
        Some(ComptimeParameter::Integer(value)) => (ComptimeParameterValue::Integer(value), ComptimeParameterOrigin::Profile),
        Some(ComptimeParameter::Boolean(value)) => (ComptimeParameterValue::Boolean(value), ComptimeParameterOrigin::Profile),
        Some(ComptimeParameter::UnsupportedValue) => return Err(reject(ComptimeParameterRejection::UnsupportedProfileValue)),
        None => {
            let (resolved, literal) = default.ok_or_else(|| reject(ComptimeParameterRejection::Missing))?;
            (resolved, ComptimeParameterOrigin::Default { literal })
        }
    };
    let observed = ComptimeParameterUse { name: name.into(), call: span, context: expected.clone(), resolved, origin };
    *expression = match resolved {
        ComptimeParameterValue::Integer(value) => ScalarExpression::Integer { value: value.into(), span },
        ComptimeParameterValue::Boolean(value) => ScalarExpression::Boolean { value, span },
    };
    Ok(Some(observed))
}

fn audited_closed_unary(operator: &UnaryOperator) -> bool {
    matches!(operator, UnaryOperator::LogicalNot | UnaryOperator::BitwiseNot)
}

fn audited_closed_eager_binary(operator: &BinaryOperator) -> bool {
    matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual
        | BinaryOperator::BitAnd | BinaryOperator::BitOr | BinaryOperator::BitXor)
}

fn admit_pure_expression(
    expression: &ScalarExpression, scope: &impl Fn(ByteSpan) -> SourceSpan,
) -> Result<(), ClosedComptimeRejection> {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match expression {
            ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. } | ScalarExpression::Name { .. } => (),
            ScalarExpression::Unary { operator, operand, .. } if audited_closed_unary(operator) =>
                pending.push(operand),
            ScalarExpression::Binary { operator, left, right, .. }
                if audited_closed_eager_binary(operator) || matches!(operator, BinaryOperator::And | BinaryOperator::Or) => {
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
    use crate::scalar::{
        derive_scalar_declarations_from_cst, derive_scalar_declarations_with_embedding,
        EmbeddedComptimeExpression,
    };
    use wosy_syntax::SourceIdentity;

    fn fresh(source: &str) -> FreshParseResult {
        wosy_syntax::parse_with_snapshot(
            SourceIdentity::new("fixture".into(), "fixture".into(), "main.w".into(), "authored".into()),
            format!("%%start\n# WHY: café ☕\n{source}\n%%end"), &["WHY".into()],
        )
    }

    #[derive(Clone, Copy)]
    enum FixtureReturn {
        I32(i32),
        U32(u32),
        Bool(bool),
    }

    // No production result ingress. This test-only adapter attaches a known
    // scalar to the occurrence and effective inputs of the same invocation.
    fn project_fixture_return(
        source: &FreshParseResult,
        layout: ScalarTargetLayout,
        parameter: Option<ComptimeParameter>,
        sample: impl FnOnce(&ClosedComptimeFragment) -> FixtureReturn,
    ) -> Result<ScalarSourceDeclarations, &'static str> {
        with_closed_comptime_occurrence(
            source, "solve", layout, SiteRequirement::RootBinding, |_| parameter,
            |fragment, canonical, site, initializer| {
                let span = fragment.site().range;
                let value = match (fragment.result_type(), sample(&fragment)) {
                    (ScalarType::I32, FixtureReturn::I32(value)) =>
                        ScalarExpression::Integer { value: value.into(), span },
                    (ScalarType::U32, FixtureReturn::U32(value)) =>
                        ScalarExpression::Integer { value: value.into(), span },
                    (ScalarType::Bool, FixtureReturn::Bool(value)) =>
                        ScalarExpression::Boolean { value, span },
                    _ => return Err("fixture return does not match compiler result type"),
                };
                Ok(derive_scalar_declarations_with_embedding(canonical, Some(
                    EmbeddedComptimeExpression { site, expression: initializer, value: &value },
                )))
            },
        ).expect("ordinary closed fragment preparation")
    }

    fn consumer_llvm(declarations: ScalarSourceDeclarations, layout: ScalarTargetLayout) -> String {
        let owner = declarations.source().clone();
        let prepared = prepare_scalar_project(
            vec![(declarations, Vec::new())],
            vec![owner],
            ScalarPreparationInputs { target_layout: layout },
        ).unwrap_or_else(|refusal| panic!("{:?}", refusal.into_ordered_diagnostics()));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        emit_scalar_build_single_module_artifact_llvm(&build).unwrap().to_text()
    }

    #[test]
    fn first_source_initializer_matches_ordinary_startup_item_and_refuses_nested_path() {
        use super::super::control::{module_initialization_region, ModuleInitializationStepKind};

        let ordinary = fresh("i32 answer = 42;\ni32 following = 7;");
        let declarations = derive_scalar_declarations_from_cst(ordinary.result().canonical_cst());
        assert!(declarations.diagnostics().is_empty());
        let region = module_initialization_region(declarations.source(), &declarations.items, &[]);
        assert_eq!(region.steps.len(), 2);
        assert!(matches!(region.steps[0].kind, ModuleInitializationStepKind::BindingInitialization));
        let ScalarItem::Binding(first) = &declarations.items[region.steps[0].item] else {
            panic!("ordinary initialization begins with the first binding");
        };
        assert_eq!(first.name, "answer");

        let direct = fresh("i32 answer = comptime solve { 42 };\ni32 following = 7;");
        let nested = fresh("i32 answer = if (true) { comptime solve { 42 } } else { 0 };");
        let parameter = fresh(r#"i32 answer = comptime solve { core.comp_param("limit", 41) };"#);
        assert!(direct.result().is_valid());
        assert!(nested.result().is_valid());
        assert!(parameter.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let fragment = derive_first_source_initializer_comptime(&direct, "solve", layout).unwrap();
            assert_eq!(fragment.result_type(), &ScalarType::I32);
            assert!(fragment.parameter_uses().is_empty());
            assert!(matches!(derive_first_source_initializer_comptime(&nested, "solve", layout),
                Err(ClosedComptimeRejection::Unsupported { feature: "expected one explicit root binding", .. })));
            assert!(matches!(derive_first_source_initializer_comptime(&parameter, "solve", layout),
                Err(ClosedComptimeRejection::Parameter { reason: ComptimeParameterRejection::UnsupportedProfileValue, .. })));
        }
    }

    #[test]
    fn closed_return_embeds_in_the_same_authored_root_as_an_ordinary_literal() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let cases = [
                ("i32", "41", None, FixtureReturn::I32(41), "41"),
                ("i32", "41", Some(ComptimeParameter::Integer(42)), FixtureReturn::I32(42), "42"),
                ("u32", "4294967295", None, FixtureReturn::U32(u32::MAX), "4294967295"),
                ("bool", "true", Some(ComptimeParameter::Boolean(false)), FixtureReturn::Bool(false), "false"),
            ];
            for (ty, default, parameter, sample, expected) in cases {
                let expression = format!("comptime solve {{ core.comp_param(\"value\", {default}) }}");
                let source = fresh(&format!("{ty} result = {expression};"));
                let canonical = source.result().canonical_cst();
                assert_eq!(derive_scalar_declarations_from_cst(canonical).diagnostics.len(), 1);
                let embedded = project_fixture_return(&source, layout, parameter, |fragment| {
                    let [read] = fragment.parameter_uses() else { panic!("expected one observed read") };
                    assert_eq!(read.name, "value");
                    assert_eq!(&read.context, fragment.result_type());
                    assert_eq!(matches!(read.origin, ComptimeParameterOrigin::Profile), parameter.is_some());
                    assert_eq!(read.resolved, match sample {
                        FixtureReturn::I32(value) => ComptimeParameterValue::Integer(value.into()),
                        FixtureReturn::U32(value) => ComptimeParameterValue::Integer(value.into()),
                        FixtureReturn::Bool(value) => ComptimeParameterValue::Boolean(value),
                    });
                    sample
                }).unwrap();
                assert_eq!(embedded.source(), &source.result().source);
                let [ScalarItem::Binding(binding)] = embedded.items.as_slice() else {
                    panic!("expected the original root binding");
                };
                let span = span_of(&binding.value);
                assert_eq!(&source.result().text[span.start as usize..span.end as usize], expression);
                assert_eq!(binding.output_values[0].value, binding.value);
                assert_eq!(binding.output_values[0].span, span);
                let ordinary = fresh(&format!("{ty} result = {expected};"));
                let ordinary = derive_scalar_declarations_from_cst(ordinary.result().canonical_cst());
                let ir = consumer_llvm(embedded, layout);
                assert_eq!(ir, consumer_llvm(ordinary, layout));
                assert!(!ir.contains("@evaluate"));
            }
        }
    }

    #[test]
    fn fixture_type_mismatch_and_another_authored_occurrence_do_not_embed() {
        let source = fresh("i32 result = comptime solve { 42 };");
        assert_eq!(project_fixture_return(&source, ScalarTargetLayout::NATIVE64, None, |_|
            FixtureReturn::U32(42)).err(), Some("fixture return does not match compiler result type"));

        let source = fresh("i32 result = comptime solve { 42 };\ni32 other = comptime pending { 43 };");
        let canonical = source.result().canonical_cst();
        let ordinary = derive_scalar_declarations_from_cst(canonical);
        let [solved, pending] = ordinary.diagnostics.as_slice() else { panic!("expected two ordinary refusals"); };
        let span = |diagnostic: &Diagnostic| {
            let range = diagnostic.labels[0].span.range;
            &source.result().text[range.start as usize..range.end as usize]
        };
        assert_eq!(span(solved), "comptime solve { 42 }");
        assert_eq!(span(pending), "comptime pending { 43 }");
        let projected = project_fixture_return(&source, ScalarTargetLayout::WASM32, None, |_|
            FixtureReturn::I32(42)).unwrap();
        assert_eq!(projected.diagnostics.as_slice(), std::slice::from_ref(pending));
        assert!(projected.items.is_empty());
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
