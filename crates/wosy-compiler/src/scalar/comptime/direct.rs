//! Inspect one direct authored comptime call and emit only its detached evaluator.
use super::{
    analyze_scalar_build, derive_block, emit_closed_evaluator, prepare_scalar_project, span_of,
    ClosedComptimeFragment, ClosedComptimeRejection, ScalarBlock,
    ScalarBlockItem, ScalarExpression, ScalarItem, ScalarPreparationInputs, ScalarTargetLayout,
    ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use super::super::{
    control::{ModuleInitializationStepKind, PlainNameIdentity},
    derive_scalar_declarations_with_embedding, ConcreteCallSelection, EmbeddedComptimeExpression,
};
use rowan::NodeOrToken;
use wosy_syntax::{ByteSpan, ComptimeCst, CstNode, FreshParseResult, SourceSpan, SyntaxKind};

const MAX_DIRECT_STARTUP_CALLS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectComptimeArgument {
    I32(i32),
    U32(u32),
    Bool(bool),
}

/// Authored, typed facts from one inspection, not emitted code, a selected
/// artifact or authority to execute a tool. No prepared AST escapes.
#[derive(Debug, PartialEq, Eq)]
pub struct InspectedDirectComptimeCall {
    site: SourceSpan,
    function: SourceSpan,
    call: SourceSpan,
    argument: SourceSpan,
    formal: SourceSpan,
    read: SourceSpan,
    value: DirectComptimeArgument,
    result: ScalarType,
}

impl InspectedDirectComptimeCall {
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn function(&self) -> &SourceSpan { &self.function }
    pub fn call(&self) -> &SourceSpan { &self.call }
    pub fn argument(&self) -> &SourceSpan { &self.argument }
    pub fn formal(&self) -> &SourceSpan { &self.formal }
    pub fn read(&self) -> &SourceSpan { &self.read }
    pub fn value(&self) -> DirectComptimeArgument { self.value }
    pub fn result(&self) -> &ScalarType { &self.result }
}

/// Inspect only one module containing a single ordinary function and one
/// binding that immediately calls it with a scalar literal. Its only
/// comptime tail reads the exact formal. The caller must establish the
/// module's real target-entry authority separately. No general proof,
/// profile lookup, execution, result substitution or cache identity.
pub fn inspect_direct_function_comptime(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<InspectedDirectComptimeCall, ClosedComptimeRejection> {
    with_verified_direct_call(fresh, id, layout, |facts, _, _| Ok(facts))
}

/// Exact typed facts and the detached evaluator for this one concrete input.
/// Not an executed result, selected profile, manifest, or cache key.
pub struct DirectFunctionComptimeFragment {
    facts: InspectedDirectComptimeCall,
    evaluator: ClosedComptimeFragment,
}

impl DirectFunctionComptimeFragment {
    pub fn facts(&self) -> &InspectedDirectComptimeCall { &self.facts }
    pub fn evaluator(&self) -> &ClosedComptimeFragment { &self.evaluator }

    pub fn into_parts(self) -> (InspectedDirectComptimeCall, ClosedComptimeFragment) {
        (self.facts, self.evaluator)
    }
}

/// Derive evaluator LLVM from the compiler-observed concrete argument. No
/// API accepts a value or allows an inspected fact from another source to
/// be applied here. Neither the analytical project nor the caller is emitted.
pub fn derive_direct_function_comptime_fragment(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectFunctionComptimeFragment, ClosedComptimeRejection> {
    with_verified_direct_call(fresh, id, layout, |facts, body, name_span| {
        emit_bound_evaluator(id, layout, body, name_span, &facts)
            .map(|evaluator| DirectFunctionComptimeFragment { facts, evaluator })
    })
}

/// An exact canonical startup occurrence and its index into this plan's
/// evaluator list. The index has no meaning outside its owning plan.
#[derive(Debug)]
pub struct DirectStartupComptimeUse {
    facts: InspectedDirectComptimeCall,
    evaluator: usize,
}

impl DirectStartupComptimeUse {
    pub fn facts(&self) -> &InspectedDirectComptimeCall { &self.facts }
    pub fn evaluator_index(&self) -> usize { self.evaluator }
}

/// Detached evaluator IR for a bounded startup list in one authored module.
/// Call sites retain their order and provenance. Nothing runs or persists.
pub struct DirectStartupComptimePlan {
    calls: Vec<DirectStartupComptimeUse>,
    evaluators: Vec<ClosedComptimeFragment>,
}

impl DirectStartupComptimePlan {
    pub fn calls(&self) -> &[DirectStartupComptimeUse] { &self.calls }
    pub fn evaluators(&self) -> &[ClosedComptimeFragment] { &self.evaluators }

    pub fn into_parts(self) -> (Vec<DirectStartupComptimeUse>, Vec<ClosedComptimeFragment>) {
        (self.calls, self.evaluators)
    }
}

/// One formal-only tail and at most sixteen direct literal startup calls to
/// that same declaration. Authenticate the entire ordinary startup list
/// before emitting anything. Equal tuples share one evaluator only within
/// this exact analysis; this does not define a reusable ComptimeKey.
pub fn derive_direct_startup_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan, ClosedComptimeRejection> {
    with_verified_direct_calls(fresh, id, layout, MAX_DIRECT_STARTUP_CALLS, |calls, body, name_span| {
        let mut plan = DirectStartupComptimePlan {
            calls: Vec::with_capacity(calls.len()), evaluators: Vec::new(),
        };
        let mut values = Vec::new();
        for facts in calls {
            // The rooted function, its only formal read, and layout are
            // identical in this analysis. The tagged literal is all that varies.
            let evaluator = match values.iter().position(|value| *value == facts.value) {
                Some(index) => index,
                None => {
                    let index = plan.evaluators.len();
                    plan.evaluators.push(emit_bound_evaluator(id, layout, body, name_span, &facts)?);
                    values.push(facts.value);
                    index
                }
            };
            plan.calls.push(DirectStartupComptimeUse { facts, evaluator });
        }
        Ok(plan)
    })
}

fn emit_bound_evaluator(
    id: &str, layout: ScalarTargetLayout, authored_body: &ScalarBlock, name_span: ByteSpan,
    facts: &InspectedDirectComptimeCall,
) -> Result<ClosedComptimeFragment, ClosedComptimeRejection> {
    let mut body = authored_body.clone();
    let ([ScalarBlockItem::Expression(item)], [expression], [output]) = (
        body.items.as_mut_slice(), body.expressions.as_mut_slice(), body.final_output_values.as_mut_slice(),
    ) else {
        return Err(ClosedComptimeRejection::Emitter("verified evaluator body lost its sole output".into()));
    };
    let span = facts.read.range;
    let value = match facts.value {
        DirectComptimeArgument::I32(value) => ScalarExpression::Integer { value: value.into(), span },
        DirectComptimeArgument::U32(value) => ScalarExpression::Integer { value: value.into(), span },
        DirectComptimeArgument::Bool(value) => ScalarExpression::Boolean { value, span },
    };
    *item = value.clone();
    *expression = value.clone();
    output.value = value;
    output.ty = facts.result.clone();
    emit_closed_evaluator(id, facts.site.clone(), name_span, layout, facts.result.clone(), Vec::new(), body)
}

fn with_verified_direct_call<T>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
    observe: impl FnOnce(InspectedDirectComptimeCall, &ScalarBlock, ByteSpan) -> Result<T, ClosedComptimeRejection>,
) -> Result<T, ClosedComptimeRejection> {
    with_verified_direct_calls(fresh, id, layout, 1, |mut calls, body, name_span| {
        let call = calls.pop().ok_or_else(|| ClosedComptimeRejection::Emitter("verified single call disappeared".into()))?;
        observe(call, body, name_span)
    })
}

// The analytical project stays private. The continuation is called only
// after every startup occurrence and the sole formal read were verified.
fn with_verified_direct_calls<T>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout, limit: usize,
    observe: impl FnOnce(Vec<InspectedDirectComptimeCall>, &ScalarBlock, ByteSpan) -> Result<T, ClosedComptimeRejection>,
) -> Result<T, ClosedComptimeRejection> {
    use ClosedComptimeRejection as Error;
    let parsed = fresh.result();
    if !matches!(layout, ScalarTargetLayout::NATIVE64 | ScalarTargetLayout::WASM32) {
        return Err(Error::UnsupportedLayout);
    }
    if !parsed.is_valid() { return Err(Error::InvalidParse); }
    if parsed.text.len() > CLOSED_COMPTIME_MAX_SOURCE_BYTES { return Err(Error::SourceLimit); }
    let scope = |span| SourceSpan::new(parsed.source.clone(), span);
    let mut sites = parsed.root.descendants().filter_map(|node| {
        let site = ComptimeCst::from_node(&node)?;
        (site.id.text() == id).then_some((node, site))
    });
    let (node, site) = sites.next().ok_or(Error::SiteNotFound)?;
    if let Some((_, repeated)) = sites.next() { return Err(Error::RepeatedSite(scope(repeated.span))); }
    let unsupported = |feature| Error::Unsupported { site: scope(site.span), feature };
    let (tail, declaration_item, source) = function_tail(&node)
        .ok_or_else(|| unsupported("expected the only tail of one ordinary function"))?;
    let mut authored_items = source.children()
        .filter(|child| !matches!(child.kind(), SyntaxKind::BoundaryStart | SyntaxKind::BoundaryEnd));
    if authored_items.next().as_ref() != Some(&declaration_item) {
        return Err(unsupported("expected the only tail of one ordinary function"));
    }
    let mut count = 0;
    for item in authored_items {
        if only(item.children()).is_none_or(|child| child.kind() != SyntaxKind::BindingDecl) {
            return Err(unsupported("each authored startup item must be one ordinary binding"));
        }
        count += 1;
        if count > limit {
            return Err(unsupported("direct comptime startup call limit exceeded"));
        }
    }
    if count == 0 { return Err(unsupported("no direct startup call was authored")); }
    let name = sole_final_value(&site.block)
        .and_then(|expression| only(expression.children()))
        .filter(|value| value.kind() == SyntaxKind::Primary && value.children().next().is_none())
        .and_then(|value| only(value.children_with_tokens().filter_map(NodeOrToken::into_token)
            .filter(|token| !matches!(token.kind(), SyntaxKind::Whitespace | SyntaxKind::LineEnding
                | SyntaxKind::Indentation | SyntaxKind::TypedComment))))
        .filter(|token| token.kind() == SyntaxKind::Identifier)
        .ok_or_else(|| unsupported("comptime body must only read its formal"))?;

    // The ordinary contextual typer visits the actual authored scalar
    // block. This is not a substitute literal or an executable lowering:
    // the analytical project stays inside this function, and is never emitted.
    let block = ScalarExpression::Block(derive_block(&site.block));
    let declarations = derive_scalar_declarations_with_embedding(parsed.canonical_cst(), Some(
        EmbeddedComptimeExpression { site: &node, expression: &tail, value: &block },
    ));
    let prepared = prepare_scalar_project(
        vec![(declarations, Vec::new())], vec![parsed.source.clone()],
        ScalarPreparationInputs { target_layout: layout },
    ).map_err(|error| Error::OrdinaryDiagnostics(error.into_ordered_diagnostics()))?;
    let [module] = prepared.project().modules.as_slice() else { unreachable!("one supplied module"); };
    let [ScalarItem::Function(function), bindings @ ..] = module.items.as_slice() else {
        return Err(unsupported("expected one ordinary function before startup"));
    };
    let Some(region) = prepared.initialization_region(0) else {
        return Err(unsupported("ordinary entry initialization was not prepared"));
    };
    if bindings.len() != count || region.steps.len() != count
        || region.steps.iter().enumerate().any(|(offset, step)| step.item != offset + 1
            || step.kind != ModuleInitializationStepKind::BindingInitialization)
    {
        return Err(unsupported("ordinary entry initialization order does not match the authored list"));
    }

    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() { return Err(Error::OrdinaryDiagnostics(build.diagnostics().to_vec())); }
    let [ScalarBlockItem::Expression(ScalarExpression::Block(body))] = function.body.items.as_slice() else {
        return Err(unsupported("ordinary callable tail does not match the authored block"));
    };
    let [ScalarBlockItem::Expression(read @ ScalarExpression::Name { .. })] = body.items.as_slice() else {
        return Err(unsupported("ordinary block does not contain one formal read"));
    };
    let points = build.callable_point_facts(0, 0, None)
        .ok_or_else(|| unsupported("ordinary parameter binding facts are unavailable"))?;
    let nodes = prepared.bind_callable_body_call_nodes(0, 0, None, &function.body)
        .map_err(|_| unsupported("ordinary callable occurrences are unavailable"))?;
    let (formal, formal_type) = nodes.formal(points, 0)
        .ok_or_else(|| unsupported("no ordinary formal binding was prepared"))?;
    if nodes.checked_expression_type(read) != Some(formal_type)
        || !matches!(nodes.plain_name(read), Some(PlainNameIdentity::Lexical(binding)) if binding == formal)
        || span_of(read) != super::super::token_span(&name)
    {
        return Err(unsupported("authored read does not select the exact ordinary formal"));
    }

    let facts = prepared.initializer_facts(0)
        .map_err(|_| unsupported("ordinary initializer occurrence facts are unavailable"))?;
    let declaration = scope(function.name_span);
    let mut selected = Vec::with_capacity(count);
    for item in bindings {
        let ScalarItem::Binding(binding) = item else {
            return Err(unsupported("expected one argument binding at each startup step"));
        };
        if binding.receivers.len() != 1 {
            return Err(unsupported("expected one direct initializer destination"));
        }
        let call = &binding.value;
        let ScalarExpression::Call { receiver: None, type_arguments, arguments, .. } = call else {
            return Err(unsupported("expected a direct ordinary initializer call"));
        };
        let [argument] = arguments.as_slice() else { return Err(unsupported("one literal argument is required")); };
        if !type_arguments.is_empty() { return Err(unsupported("generic arguments are not admitted")); }
        let destination = facts.binding_output(binding, 0, call)
            .map_err(|_| unsupported("call is not the entire ordinary initializer"))?;
        let target = facts.checked_direct_call(call)
            .map_err(|_| unsupported("no exact direct declaration was selected"))?;
        if target.declaration != &declaration || target.concrete != &ConcreteCallSelection::Declaration {
            return Err(unsupported("initializer did not select the authored function"));
        }
        let ([parameter], [result]) = (target.parameters, target.outputs) else {
            return Err(unsupported("one contextual scalar parameter and result are required"));
        };
        if destination.ty != &result.ty || parameter != &result.ty || parameter != formal_type {
            return Err(unsupported("the declared result must be the exact read formal's scalar type"));
        }
        let argument_ty = facts.checked_expression_type(argument)
            .map_err(|_| unsupported("ordinary argument type is unavailable"))?;
        if argument_ty != parameter {
            return Err(unsupported("literal argument does not have the selected formal type"));
        }
        let value = match (argument, parameter) {
            (ScalarExpression::Integer { value, .. }, ScalarType::I32) =>
                i32::try_from(value).ok().map(DirectComptimeArgument::I32),
            (ScalarExpression::Integer { value, .. }, ScalarType::U32) =>
                u32::try_from(value).ok().map(DirectComptimeArgument::U32),
            (ScalarExpression::Boolean { value, .. }, ScalarType::Bool) => Some(DirectComptimeArgument::Bool(*value)),
            _ => None,
        }.ok_or_else(|| unsupported("argument must be an exact scalar literal"))?;
        selected.push(InspectedDirectComptimeCall {
            site: scope(site.span), function: declaration.clone(), call: scope(span_of(call)),
            argument: scope(span_of(argument)), formal: scope(formal.declaration_span),
            read: scope(span_of(read)), value, result: result.ty.clone(),
        });
    }
    observe(selected, body, super::super::token_span(&site.id))
}

fn only<T>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

fn sole_final_value(block: &CstNode) -> Option<CstNode> {
    let list = only(block.children()).filter(|child| child.kind() == SyntaxKind::FinalOutputList)?;
    let output = only(list.children()).filter(|child| child.kind() == SyntaxKind::OutputList)?;
    only(output.children()).filter(|child| child.kind() == SyntaxKind::Expression)
}

fn function_tail(node: &CstNode) -> Option<(CstNode, CstNode, CstNode)> {
    let primary = node.parent().filter(|p| p.kind() == SyntaxKind::Primary)?;
    let expression = primary.parent().filter(|p| p.kind() == SyntaxKind::Expression)?;
    let output = expression.parent().filter(|p| p.kind() == SyntaxKind::OutputList)?;
    let tail = output.parent().filter(|p| p.kind() == SyntaxKind::FinalOutputList)?;
    let block = tail.parent().filter(|p| p.kind() == SyntaxKind::Block)?;
    if sole_final_value(&block).as_ref() != Some(&expression) { return None; }
    let function = block.parent().filter(|p| p.kind() == SyntaxKind::FunctionDecl)?;
    if function.children_with_tokens().any(|child| child.kind() == SyntaxKind::Unsafe) { return None; }
    let item = function.parent().filter(|p| p.kind() == SyntaxKind::Item)?;
    let source = item.parent().filter(|p| p.kind() == SyntaxKind::Root)?;
    Some((expression, item, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::derive_scalar_declarations_from_cst;
    use wosy_syntax::SourceIdentity;

    fn fresh(source: &str) -> FreshParseResult {
        let source = format!("%%start\n# WHY: café na órbita ☕\n{source}\n%%end");
        wosy_syntax::parse_with_snapshot(
            SourceIdentity::new("owned".into(), "entry".into(), "main.w".into(), "authored".into()),
            source, &["WHY".into()],
        )
    }

    fn spelling<'a>(fresh: &'a FreshParseResult, span: &SourceSpan) -> &'a str {
        assert_eq!(span.source, fresh.result().source);
        &fresh.result().text[span.range.start as usize..span.range.end as usize]
    }

    #[test]
    fn direct_comptime_inspection_binds_typed_literal_call_and_authored_formal() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            for (ty, literal, value, expected) in [
                ("i32", "7", DirectComptimeArgument::I32(7), ScalarType::I32),
                ("u32", "4294967295", DirectComptimeArgument::U32(u32::MAX), ScalarType::U32),
                ("bool", "true", DirectComptimeArgument::Bool(true), ScalarType::Bool),
            ] {
                let authored = fresh(&format!(
                    "{ty}({ty}) solve = fn(input) {{ comptime value {{ input }} }};\n{ty} answer = solve({literal});"
                ));
                assert!(authored.result().is_valid());
                let fact = inspect_direct_function_comptime(&authored, "value", layout)
                    .unwrap_or_else(|why| panic!("exact typed inspection refused {ty} on {layout:?}: {why:?}"));
                assert_eq!(fact.value(), value);
                assert_eq!(fact.result(), &expected);
                assert_eq!(spelling(&authored, fact.site()), "comptime value { input }");
                assert_eq!(spelling(&authored, fact.function()), "solve");
                assert_eq!(spelling(&authored, fact.call()), format!("solve({literal})"));
                assert_eq!(spelling(&authored, fact.argument()), literal);
                assert_eq!(spelling(&authored, fact.formal()), "input");
                assert_eq!(spelling(&authored, fact.read()), "input");
                assert_ne!(fact.formal(), fact.read());

                let ordinary = derive_scalar_declarations_from_cst(authored.result().canonical_cst());
                let [diagnostic] = ordinary.diagnostics() else { panic!("untouched ordinary refusal"); };
                assert_eq!(diagnostic.code, "B0003");
                assert_eq!(&diagnostic.labels[0].span, fact.site());
                assert!(ordinary.items.is_empty());
            }
        }
    }

    #[test]
    fn direct_comptime_evaluator_emits_only_the_compiler_bound_argument() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let mut i32_evaluators = Vec::new();
            for (ty, literal, value) in [
                ("i32", "7", DirectComptimeArgument::I32(7)),
                ("i32", "9", DirectComptimeArgument::I32(9)),
                ("u32", "4294967295", DirectComptimeArgument::U32(u32::MAX)),
                ("bool", "false", DirectComptimeArgument::Bool(false)),
                ("bool", "true", DirectComptimeArgument::Bool(true)),
            ] {
                let direct = fresh(&format!(
                    "{ty}({ty}) solve = fn(input) {{ comptime value {{ input }} }};\n{ty} answer = solve({literal});"
                ));
                let exact = derive_direct_function_comptime_fragment(&direct, "value", layout)
                    .unwrap_or_else(|error| panic!("typed evaluator {ty} on {layout:?}: {error:?}"));
                let ordinary = fresh(&format!("{ty} answer = comptime value {{ {literal} }};"));
                let expected = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None)
                    .unwrap_or_else(|error| panic!("literal oracle {ty}: {error:?}"));
                assert_eq!(exact.facts().value(), value);
                assert_eq!(spelling(&direct, exact.facts().argument()), literal);
                assert_eq!(exact.facts().site(), exact.evaluator().site());
                assert_eq!(exact.facts().result(), exact.evaluator().result_type());
                assert_eq!(exact.evaluator().layout(), layout);
                assert!(exact.evaluator().parameter_uses().is_empty());
                let actual = exact.evaluator().llvm().to_text();
                assert_eq!(actual, expected.llvm().to_text());
                assert!(!actual.contains("@solve("), "analytical program leaked into evaluator");
                if ty == "i32" { i32_evaluators.push(actual); }
            }
            assert_eq!(i32_evaluators.len(), 2);
            assert_ne!(i32_evaluators[0], i32_evaluators[1]);

            let unbound = fresh("i32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(1 | 2);");
            assert!(matches!(derive_direct_function_comptime_fragment(&unbound, "value", layout),
                Err(ClosedComptimeRejection::Unsupported { feature: "argument must be an exact scalar literal", .. })));
        }
    }

    const STARTUP_FUNCTION: &str = "i32(i32) solve = fn(input) { comptime value { input } };";

    #[test]
    fn direct_startup_plan_preserves_order_and_shares_only_identical_typed_tuples() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let authored = fresh(&format!(
                "{STARTUP_FUNCTION}\ni32 first = solve(7);\ni32 second = solve(9);\ni32 third = solve(7);"
            ));
            let plan = derive_direct_startup_comptime_plan(&authored, "value", layout)
                .unwrap_or_else(|error| panic!("finite startup plan: {error:?}"));
            let [a, b, c] = plan.calls() else { panic!("three canonical startup occurrences"); };
            assert_eq!([a.evaluator_index(), b.evaluator_index(), c.evaluator_index()], [0, 1, 0]);
            for (call, literal, value) in [(a, "7", 7), (b, "9", 9), (c, "7", 7)] {
                let facts = call.facts();
                assert_eq!(facts.value(), DirectComptimeArgument::I32(value));
                assert_eq!(facts.result(), &ScalarType::I32);
                assert_eq!(facts.function(), a.facts().function());
                assert_eq!(facts.formal(), a.facts().formal());
                assert_eq!(facts.read(), a.facts().read());
                assert_eq!(facts.site(), a.facts().site());
                assert_eq!(spelling(&authored, facts.argument()), literal);
                assert_eq!(spelling(&authored, facts.call()), format!("solve({literal})"));
            }
            assert!(a.facts().call().range.end < b.facts().call().range.start);
            assert!(b.facts().call().range.end < c.facts().call().range.start);
            let [seven, nine] = plan.evaluators() else { panic!("two exact evaluator tuples"); };
            for (evaluator, literal) in [(seven, "7"), (nine, "9")] {
                let ordinary = fresh(&format!("i32 answer = comptime value {{ {literal} }};"));
                let expected = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None).unwrap();
                assert_eq!(evaluator.llvm().to_text(), expected.llvm().to_text());
                assert_eq!(evaluator.site(), a.facts().site());
                assert_eq!(evaluator.layout(), layout);
            }
            assert_ne!(seven.llvm().to_text(), nine.llvm().to_text());
            assert!(matches!(inspect_direct_function_comptime(&authored, "value", layout),
                Err(ClosedComptimeRejection::Unsupported { feature: "direct comptime startup call limit exceeded", .. })));
        }
    }

    #[test]
    fn direct_startup_plan_refuses_a_late_unproven_call_and_caps_the_entire_list() {
        let layout = ScalarTargetLayout::WASM32;
        let computed = fresh(&format!(
            "{STARTUP_FUNCTION}\ni32 first = solve(7);\ni32 second = solve(first);"
        ));
        assert!(computed.result().is_valid());
        assert!(matches!(derive_direct_startup_comptime_plan(&computed, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "argument must be an exact scalar literal", .. })));
        let unrelated = fresh(&format!(
            "{STARTUP_FUNCTION}\ni32 first = solve(7);\ni32 unrelated = 9;"
        ));
        assert!(unrelated.result().is_valid());
        assert!(matches!(derive_direct_startup_comptime_plan(&unrelated, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "expected a direct ordinary initializer call", .. })));

        let mut full = STARTUP_FUNCTION.to_owned();
        for index in 0..MAX_DIRECT_STARTUP_CALLS {
            full.push_str(&format!("\ni32 result_{index} = solve(7);"));
        }
        let at_limit = derive_direct_startup_comptime_plan(&fresh(&full), "value", layout).unwrap();
        assert_eq!(at_limit.calls().len(), MAX_DIRECT_STARTUP_CALLS);
        assert_eq!(at_limit.evaluators().len(), 1);
        assert!(at_limit.calls().iter().all(|call| call.evaluator_index() == 0));
        full.push_str("\ni32 last = solve(7);");
        let over = fresh(&full);
        assert!(over.result().is_valid());
        assert!(matches!(derive_direct_startup_comptime_plan(&over, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "direct comptime startup call limit exceeded", .. })));
    }

    #[test]
    fn direct_comptime_inspection_does_not_guess_values_targets_or_lexical_reads() {
        let layout = ScalarTargetLayout::WASM32;
        for (source, reason) in [
            (
                "i32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(1 | 2);",
                "argument must be an exact scalar literal",
            ),
            (
                "i32 preceding = 1;\ni32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(7);",
                "expected the only tail of one ordinary function",
            ),
            (
                "i32(i32) solve = fn(input) { comptime value { 7 } };\ni32 answer = solve(7);",
                "comptime body must only read its formal",
            ),
        ] {
            let authored = fresh(source);
            assert!(authored.result().is_valid());
            match inspect_direct_function_comptime(&authored, "value", layout) {
                Err(ClosedComptimeRejection::Unsupported { feature, .. }) => assert_eq!(feature, reason),
                wrong => panic!("expected {reason}; got {wrong:?}"),
            }
        }
        let wrong_type = fresh("i32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(true);");
        assert!(wrong_type.result().is_valid());
        assert!(matches!(inspect_direct_function_comptime(&wrong_type, "value", layout),
            Err(ClosedComptimeRejection::OrdinaryDiagnostics(_))));
    }
}
