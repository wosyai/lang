//! A non-emitting inspection of one directly called authored comptime tail.
use super::{
    analyze_scalar_build, derive_block, prepare_scalar_project, span_of, ClosedComptimeRejection,
    ScalarBlockItem, ScalarExpression, ScalarItem, ScalarPreparationInputs, ScalarTargetLayout,
    ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use super::super::{
    control::{ModuleInitializationStepKind, PlainNameIdentity},
    derive_scalar_declarations_with_embedding, ConcreteCallSelection, EmbeddedComptimeExpression,
};
use rowan::NodeOrToken;
use wosy_syntax::{byte_span, ComptimeCst, CstNode, FreshParseResult, SourceSpan, SyntaxKind};

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
    let tail = function_tail(&node)
        .ok_or_else(|| unsupported("expected the only tail of one ordinary function"))?;
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
    let [ScalarItem::Function(function), ScalarItem::Binding(binding)] = module.items.as_slice() else {
        return Err(unsupported("expected one ordinary function and one entry binding"));
    };
    let Some(region) = prepared.initialization_region(0) else {
        return Err(unsupported("ordinary entry initialization was not prepared"));
    };
    if !matches!(region.steps.as_slice(), [step] if step.item == 1
        && step.kind == ModuleInitializationStepKind::BindingInitialization) {
        return Err(unsupported("direct call is not the only ordinary entry initialization"));
    }
    if binding.receivers.len() != 1 {
        return Err(unsupported("expected one direct initializer destination"));
    }
    let call = &binding.value;
    let ScalarExpression::Call { receiver: None, type_arguments, arguments, .. } = call else {
        return Err(unsupported("expected a direct ordinary initializer call"));
    };
    let [argument] = arguments.as_slice() else { return Err(unsupported("one literal argument is required")); };
    if !type_arguments.is_empty() { return Err(unsupported("generic arguments are not admitted")); }

    let facts = prepared.initializer_facts(0)
        .map_err(|_| unsupported("ordinary initializer occurrence facts are unavailable"))?;
    let destination = facts.binding_output(binding, 0, call)
        .map_err(|_| unsupported("call is not the entire ordinary initializer"))?;
    let target = facts.checked_direct_call(call)
        .map_err(|_| unsupported("no exact direct declaration was selected"))?;
    let declaration = scope(function.name_span);
    if target.declaration != &declaration || target.concrete != &ConcreteCallSelection::Declaration {
        return Err(unsupported("initializer did not select the authored function"));
    }
    let ([parameter], [result]) = (target.parameters, target.outputs) else {
        return Err(unsupported("one contextual scalar parameter and result are required"));
    };
    if destination.ty != &result.ty || parameter != &result.ty {
        return Err(unsupported("the declared result must be the formal's scalar type"));
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
    if formal_type != parameter || nodes.checked_expression_type(read) != Some(parameter)
        || !matches!(nodes.plain_name(read), Some(PlainNameIdentity::Lexical(binding)) if binding == formal)
        || span_of(read) != super::super::token_span(&name)
    {
        return Err(unsupported("authored read does not select the exact ordinary formal"));
    }
    Ok(InspectedDirectComptimeCall {
        site: scope(site.span), function: declaration, call: scope(span_of(call)), argument: scope(span_of(argument)),
        formal: scope(formal.declaration_span), read: scope(span_of(read)), value, result: result.ty.clone(),
    })
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

fn function_tail(node: &CstNode) -> Option<CstNode> {
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
    let mut items = source.children()
        .filter(|child| !matches!(child.kind(), SyntaxKind::BoundaryStart | SyntaxKind::BoundaryEnd));
    if items.next().as_ref() != Some(&item) {
        return None;
    }
    only(items).and_then(|next| only(next.children()))
        .filter(|next| next.kind() == SyntaxKind::BindingDecl)?;
    Some(expression)
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
