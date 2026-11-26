//! Inspect one direct authored comptime call and emit only its detached evaluator.
use super::{
    analyze_scalar_build, audited_closed_eager_binary, audited_closed_unary, derive_block,
    emit_closed_evaluator, prepare_scalar_project, span_of,
    ClosedComptimeFragment, ClosedComptimeRejection, ScalarBlock,
    ScalarBlockItem, ScalarExpression, ScalarItem, ScalarPreparationInputs, ScalarTargetLayout,
    ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use super::super::{
    control::{ModuleInitializationStepKind, PlainNameIdentity},
    derive_scalar_declarations_with_embedding, ConcreteCallSelection, EmbeddedComptimeExpression,
};
use wosy_syntax::{ByteSpan, ComptimeCst, CstNode, FreshParseResult, SourceSpan, SyntaxKind};

const MAX_DIRECT_STARTUP_CALLS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectComptimeArgument {
    I32(i32),
    U32(u32),
    Bool(bool),
}

/// The proven correspondence between one literal argument and the formal
/// its authored occurrence reads. Pair inputs use ordinary formal order.
#[derive(Debug, PartialEq, Eq)]
pub struct DirectComptimeInput {
    argument: SourceSpan,
    formal: SourceSpan,
    read: SourceSpan,
    operand: FormalOperand,
    value: DirectComptimeArgument,
}

impl DirectComptimeInput {
    pub fn argument(&self) -> &SourceSpan { &self.argument }
    pub fn formal(&self) -> &SourceSpan { &self.formal }
    pub fn read(&self) -> &SourceSpan { &self.read }
    pub fn value(&self) -> DirectComptimeArgument { self.value }
}

/// Authored, typed facts from one inspection, not emitted code, a selected
/// artifact or authority to execute a tool. No prepared AST escapes.
#[derive(Debug, PartialEq, Eq)]
pub struct InspectedDirectComptimeCall<const ARITY: usize = 1> {
    site: SourceSpan,
    function: SourceSpan,
    call: SourceSpan,
    inputs: [DirectComptimeInput; ARITY],
    result: ScalarType,
}

impl<const ARITY: usize> InspectedDirectComptimeCall<ARITY> {
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn function(&self) -> &SourceSpan { &self.function }
    pub fn call(&self) -> &SourceSpan { &self.call }
    pub fn inputs(&self) -> &[DirectComptimeInput; ARITY] { &self.inputs }
    pub fn result(&self) -> &ScalarType { &self.result }

    fn tuple(&self) -> [DirectComptimeArgument; ARITY] {
        self.inputs.each_ref().map(DirectComptimeInput::value)
    }
}

impl InspectedDirectComptimeCall<1> {
    pub fn argument(&self) -> &SourceSpan { self.inputs[0].argument() }
    pub fn formal(&self) -> &SourceSpan { self.inputs[0].formal() }
    pub fn read(&self) -> &SourceSpan { self.inputs[0].read() }
    pub fn value(&self) -> DirectComptimeArgument { self.inputs[0].value() }
}

/// Inspect only one module containing a single ordinary function and one
/// binding that immediately calls it with a scalar literal. Its only
/// comptime tail reads the exact formal, alone or in one eager scalar operation.
/// The caller must establish the
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
pub struct DirectStartupComptimeUse<const ARITY: usize = 1> {
    facts: InspectedDirectComptimeCall<ARITY>,
    evaluator: usize,
}

impl<const ARITY: usize> DirectStartupComptimeUse<ARITY> {
    pub fn facts(&self) -> &InspectedDirectComptimeCall<ARITY> { &self.facts }
    pub fn evaluator_index(&self) -> usize { self.evaluator }
}

/// Detached evaluator IR for a bounded startup list in one authored module.
/// Call sites retain their order and provenance. Nothing runs or persists.
pub struct DirectStartupComptimePlan<const ARITY: usize = 1> {
    calls: Vec<DirectStartupComptimeUse<ARITY>>,
    evaluators: Vec<ClosedComptimeFragment>,
}

impl<const ARITY: usize> DirectStartupComptimePlan<ARITY> {
    pub fn calls(&self) -> &[DirectStartupComptimeUse<ARITY>] { &self.calls }
    pub fn evaluators(&self) -> &[ClosedComptimeFragment] { &self.evaluators }

    pub fn into_parts(self) -> (Vec<DirectStartupComptimeUse<ARITY>>, Vec<ClosedComptimeFragment>) {
        (self.calls, self.evaluators)
    }
}

/// One eager scalar tail and at most sixteen direct literal startup calls to
/// that same declaration. Authenticate the entire ordinary startup list
/// before emitting anything. Equal tuples share one evaluator only within
/// this exact analysis; this does not define a reusable ComptimeKey.
pub fn derive_direct_startup_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan, ClosedComptimeRejection> {
    derive_direct_startup::<1>(fresh, id, layout)
}

/// The same bounded startup proof for two literal arguments and a single
/// eager binary expression reading each exact formal once. The formals and
/// result share an admitted scalar type. Deduplication is on the ordered
/// typed pair, not equivalent IR or either individual input.
pub fn derive_direct_pair_startup_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan<2>, ClosedComptimeRejection> {
    derive_direct_startup::<2>(fresh, id, layout)
}

fn derive_direct_startup<const ARITY: usize>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan<ARITY>, ClosedComptimeRejection> {
    with_verified_direct_calls::<ARITY, _>(fresh, id, layout, MAX_DIRECT_STARTUP_CALLS, |calls, body, name_span| {
        let mut plan = DirectStartupComptimePlan {
            calls: Vec::with_capacity(calls.len()), evaluators: Vec::new(),
        };
        let mut values = Vec::new();
        for facts in calls {
            // Function, exact reads and layout are fixed in this analysis.
            let tuple = facts.tuple();
            let evaluator = match values.iter().position(|value| *value == tuple) {
                Some(index) => index,
                None => {
                    let index = plan.evaluators.len();
                    plan.evaluators.push(emit_bound_evaluator(id, layout, body, name_span, &facts)?);
                    values.push(tuple);
                    index
                }
            };
            plan.calls.push(DirectStartupComptimeUse { facts, evaluator });
        }
        Ok(plan)
    })
}

fn emit_bound_evaluator<const ARITY: usize>(
    id: &str, layout: ScalarTargetLayout, authored_body: &ScalarBlock, name_span: ByteSpan,
    facts: &InspectedDirectComptimeCall<ARITY>,
) -> Result<ClosedComptimeFragment, ClosedComptimeRejection> {
    let mut body = authored_body.clone();
    let ([ScalarBlockItem::Expression(item)], [expression], [output]) = (
        body.items.as_mut_slice(), body.expressions.as_mut_slice(), body.final_output_values.as_mut_slice(),
    ) else {
        return Err(ClosedComptimeRejection::Emitter("verified evaluator body lost its sole output".into()));
    };
    for input in facts.inputs() {
        let span = input.read.range;
        let value = match input.value {
            DirectComptimeArgument::I32(value) => ScalarExpression::Integer { value: value.into(), span },
            DirectComptimeArgument::U32(value) => ScalarExpression::Integer { value: value.into(), span },
            DirectComptimeArgument::Bool(value) => ScalarExpression::Boolean { value, span },
        };
        let Some(read) = input.operand.get_mut(item) else {
            return Err(ClosedComptimeRejection::Emitter("verified formal operand disappeared".into()));
        };
        if !matches!(read, ScalarExpression::Name { span: current, .. } if *current == span) {
            return Err(ClosedComptimeRejection::Emitter("verified formal operand changed identity".into()));
        }
        *read = value;
    }
    *expression = item.clone();
    output.value = item.clone();
    output.ty = facts.result.clone();
    emit_closed_evaluator(id, facts.site.clone(), name_span, layout, facts.result.clone(), Vec::new(), body)
}

fn with_verified_direct_call<T>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
    observe: impl FnOnce(InspectedDirectComptimeCall, &ScalarBlock, ByteSpan) -> Result<T, ClosedComptimeRejection>,
) -> Result<T, ClosedComptimeRejection> {
    with_verified_direct_calls::<1, _>(fresh, id, layout, 1, |mut calls, body, name_span| {
        let call = calls.pop().ok_or_else(|| ClosedComptimeRejection::Emitter("verified single call disappeared".into()))?;
        observe(call, body, name_span)
    })
}

// The analytical project stays private. The continuation is called only
// after every startup occurrence and every exact formal read were verified.
fn with_verified_direct_calls<const ARITY: usize, T>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout, limit: usize,
    observe: impl FnOnce(Vec<InspectedDirectComptimeCall<ARITY>>, &ScalarBlock, ByteSpan) -> Result<T, ClosedComptimeRejection>,
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
    sole_final_value(&site.block)
        .filter(primitive_operand_source)
        .ok_or_else(|| unsupported("comptime body must only read its formal"))?;
    let authored_body = derive_block(&site.block);
    let [ScalarBlockItem::Expression(authored)] = authored_body.items.as_slice() else {
        return Err(unsupported("comptime body must only read its formal"));
    };
    let mut authored_reads = Vec::with_capacity(ARITY);
    match ARITY {
        1 => {
            let (operand, read) = FormalOperand::select(authored)
                .ok_or_else(|| unsupported(match authored {
                    ScalarExpression::Unary { .. } | ScalarExpression::Binary { .. } =>
                        "comptime accepts one audited eager scalar operator with one formal read",
                    _ => "comptime body must only read its formal",
                }))?;
            authored_reads.push((operand, span_of(read)));
        }
        2 => {
            let ScalarExpression::Binary { operator, left, right, .. } = authored else {
                return Err(unsupported("paired comptime requires an audited binary read of both formals"));
            };
            if !audited_closed_eager_binary(operator) || !is_name(left) || !is_name(right) {
                return Err(unsupported("paired comptime requires an audited binary read of both formals"));
            }
            authored_reads.extend([(FormalOperand::Left, span_of(left)), (FormalOperand::Right, span_of(right))]);
        }
        _ => return Err(unsupported("unsupported comptime literal tuple arity")),
    }

    // The ordinary contextual typer visits the actual authored scalar
    // block. This is not a substitute literal or an executable lowering:
    // the analytical project stays inside this function, and is never emitted.
    let block = ScalarExpression::Block(authored_body);
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
    let [ScalarBlockItem::Expression(typed)] = body.items.as_slice() else {
        return Err(unsupported("ordinary block does not contain one scalar result"));
    };
    let points = build.callable_point_facts(0, 0, None)
        .ok_or_else(|| unsupported("ordinary parameter binding facts are unavailable"))?;
    let nodes = prepared.bind_callable_body_call_nodes(0, 0, None, &function.body)
        .map_err(|_| unsupported("ordinary callable occurrences are unavailable"))?;
    let result_type = nodes.checked_expression_type(typed)
        .ok_or_else(|| unsupported("ordinary comptime scalar result type is unavailable"))?;
    let mut reads = Vec::with_capacity(ARITY);
    for (operand, read_span) in authored_reads {
        let read = operand.get(typed)
            .filter(|value| matches!(value, ScalarExpression::Name { span, .. } if *span == read_span))
            .ok_or_else(|| unsupported("ordinary block does not contain the authored formal read"))?;
        let Some(PlainNameIdentity::Lexical(binding)) = nodes.plain_name(read) else {
            return Err(unsupported("authored read does not select the exact ordinary formal"));
        };
        if nodes.checked_expression_type(read) != Some(result_type) {
            return Err(unsupported("comptime reads and result require one exact scalar type"));
        }
        reads.push((operand, read_span, binding));
    }
    let mut formal_reads = Vec::with_capacity(ARITY);
    for ordinal in 0..ARITY {
        let (formal, formal_type) = nodes.formal(points, ordinal)
            .ok_or_else(|| unsupported("no ordinary formal binding was prepared"))?;
        if formal_type != result_type {
            return Err(unsupported("comptime reads and result require one exact scalar type"));
        }
        let index = reads.iter().position(|(_, _, read)| *read == formal)
            .ok_or_else(|| unsupported("authored read does not select the exact ordinary formal"))?;
        let (operand, read, _) = reads.swap_remove(index);
        formal_reads.push((operand, read, formal.declaration_span));
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
        if arguments.len() != ARITY {
            return Err(unsupported("the exact literal tuple arity is required"));
        }
        if !type_arguments.is_empty() { return Err(unsupported("generic arguments are not admitted")); }
        let destination = facts.binding_output(binding, 0, call)
            .map_err(|_| unsupported("call is not the entire ordinary initializer"))?;
        let target = facts.checked_direct_call(call)
            .map_err(|_| unsupported("no exact direct declaration was selected"))?;
        if target.declaration != &declaration || target.concrete != &ConcreteCallSelection::Declaration {
            return Err(unsupported("initializer did not select the authored function"));
        }
        let [result] = target.outputs else {
            return Err(unsupported("one contextual scalar result is required"));
        };
        if target.parameters.len() != ARITY || destination.ty != &result.ty || &result.ty != result_type {
            return Err(unsupported("the declared result must match the exact read formals' scalar type"));
        }
        let mut inputs = Vec::with_capacity(ARITY);
        for ((argument, parameter), &(operand, read, formal)) in
            arguments.iter().zip(target.parameters).zip(&formal_reads)
        {
            let argument_ty = facts.checked_expression_type(argument)
                .map_err(|_| unsupported("ordinary argument type is unavailable"))?;
            if argument_ty != parameter || parameter != result_type {
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
            inputs.push(DirectComptimeInput {
                argument: scope(span_of(argument)), formal: scope(formal), read: scope(read), operand, value,
            });
        }
        let inputs = inputs.try_into()
            .map_err(|_| unsupported("ordinary literal tuple did not preserve every formal"))?;
        selected.push(InspectedDirectComptimeCall {
            site: scope(site.span), function: declaration.clone(), call: scope(span_of(call)),
            inputs, result: result.ty.clone(),
        });
    }
    observe(selected, body, super::super::token_span(&site.id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormalOperand {
    Result,
    Unary,
    Left,
    Right,
}

impl FormalOperand {
    fn select(expression: &ScalarExpression) -> Option<(Self, &ScalarExpression)> {
        let literal = |value: &ScalarExpression| matches!(value,
            ScalarExpression::Boolean { .. } | ScalarExpression::Integer { .. });
        match expression {
            value if is_name(value) => Some((Self::Result, value)),
            ScalarExpression::Unary { operator, operand, .. }
                if audited_closed_unary(operator) && is_name(operand) => Some((Self::Unary, operand)),
            ScalarExpression::Binary { operator, left, right, .. } if audited_closed_eager_binary(operator) => {
                if is_name(left) && literal(right) { Some((Self::Left, left)) }
                else if literal(left) && is_name(right) { Some((Self::Right, right)) }
                else { None }
            }
            _ => None,
        }
    }

    fn get<'a>(self, expression: &'a ScalarExpression) -> Option<&'a ScalarExpression> {
        match (self, expression) {
            (Self::Result, value) => Some(value),
            (Self::Unary, ScalarExpression::Unary { operand, .. }) => Some(operand),
            (Self::Left, ScalarExpression::Binary { left, .. }) => Some(left),
            (Self::Right, ScalarExpression::Binary { right, .. }) => Some(right),
            _ => None,
        }
    }

    fn get_mut<'a>(self, expression: &'a mut ScalarExpression) -> Option<&'a mut ScalarExpression> {
        match (self, expression) {
            (Self::Result, value) => Some(value),
            (Self::Unary, ScalarExpression::Unary { operand, .. }) => Some(operand),
            (Self::Left, ScalarExpression::Binary { left, .. }) => Some(left),
            (Self::Right, ScalarExpression::Binary { right, .. }) => Some(right),
            _ => None,
        }
    }
}

fn is_name(value: &ScalarExpression) -> bool {
    matches!(value, ScalarExpression::Name { .. })
}

// Bound the syntactic projection before detached AST extraction. The AST,
// not syntax spelling, selects the operator and its authenticated read.
fn primitive_operand_source(expression: &CstNode) -> bool {
    let mut operator_nodes = 0;
    expression.descendants().enumerate().all(|(index, node)| {
        index < 16 && match node.kind() {
            SyntaxKind::Expression | SyntaxKind::Primary | SyntaxKind::Parenthesized | SyntaxKind::Operator => true,
            SyntaxKind::Unary | SyntaxKind::Binary => {
                operator_nodes += 1;
                operator_nodes == 1
            }
            _ => false,
        }
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
    fn direct_comptime_eager_scalar_operation_is_not_an_identity_projection() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            for (ty, body, literal, bound_expression, argument) in [
                ("u32", "input ^ 255", "7", "7 ^ 255", DirectComptimeArgument::U32(7)),
                ("u32", "255 ^ input", "9", "255 ^ 9", DirectComptimeArgument::U32(9)),
                ("i32", "~input", "7", "~7", DirectComptimeArgument::I32(7)),
                ("bool", "!input", "true", "!true", DirectComptimeArgument::Bool(true)),
                ("bool", "input != true", "true", "true != true", DirectComptimeArgument::Bool(true)),
            ] {
                let direct = fresh(&format!(
                    "{ty}({ty}) solve = fn(input) {{ comptime value {{ {body} }} }};\n{ty} answer = solve({literal});"
                ));
                assert!(direct.result().is_valid());
                let actual = derive_direct_function_comptime_fragment(&direct, "value", layout)
                    .unwrap_or_else(|error| panic!("direct body {body} on {layout:?}: {error:?}"));
                assert_eq!(spelling(&direct, actual.facts().read()), "input");
                assert_eq!(spelling(&direct, actual.facts().argument()), literal);
                assert_eq!(actual.facts().value(), argument);
                let emit_closed = |expression: &str| {
                    let authored = fresh(&format!("{ty} answer = comptime value {{ {expression} }};"));
                    super::super::derive_closed_comptime_fragment(&authored, "value", layout, |_| None)
                        .unwrap_or_else(|error| panic!("ordinary oracle {expression}: {error:?}"))
                        .llvm().to_text()
                };
                let text = actual.evaluator().llvm().to_text();
                assert_eq!(text, emit_closed(bound_expression));
                assert_ne!(text, emit_closed(literal), "{body} was reduced to an identity projection");
            }

            let batch = fresh("u32(u32) solve = fn(input) { comptime value { input ^ 255 } };\nu32 first = solve(7);\nu32 second = solve(9);\nu32 third = solve(7);");
            let plan = derive_direct_startup_comptime_plan(&batch, "value", layout).unwrap();
            assert_eq!(plan.calls().iter().map(DirectStartupComptimeUse::evaluator_index).collect::<Vec<_>>(), [0, 1, 0]);
            let [first, next] = plan.evaluators() else { panic!("two evaluator tuples"); };
            assert_ne!(first.llvm().to_text(), next.llvm().to_text());
        }
    }

    #[test]
    fn direct_comptime_eager_scope_rejects_unaudited_operations_and_multiple_reads() {
        for (out, input, body, argument) in [
            ("i32", "i32", "input + 1", "7"),
            ("i32", "i32", "-input", "7"),
            ("i32", "i32", "input << 1", "7"),
            ("bool", "i32", "input < 8", "7"),
            ("bool", "bool", "false && input", "true"),
            ("u32", "u32", "input ^ input", "7"),
        ] {
            let authored = fresh(&format!(
                "{out}({input}) solve = fn(input) {{ comptime value {{ {body} }} }};\n{out} answer = solve({argument});"
            ));
            assert!(authored.result().is_valid(), "fixture {body}");
            assert!(matches!(derive_direct_function_comptime_fragment(&authored, "value", ScalarTargetLayout::WASM32),
                Err(ClosedComptimeRejection::Unsupported {
                    feature: "comptime accepts one audited eager scalar operator with one formal read", ..
                })), "unexpectedly admitted {body}");
        }
    }

    const PAIRED_FUNCTION: &str = "u32(u32, u32) solve = fn(left, right) { comptime value { right ^ left } };";

    #[test]
    fn paired_startup_keeps_ordinary_formal_order_even_when_reversed_ir_is_equal() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let authored = fresh(&format!(
                "{PAIRED_FUNCTION}\nu32 first = solve(7, 9);\nu32 reversed = solve(9, 7);\nu32 repeated = solve(7, 9);\nu32 changed = solve(7, 11);"
            ));
            assert!(authored.result().is_valid());
            let plan = derive_direct_pair_startup_comptime_plan(&authored, "value", layout)
                .unwrap_or_else(|error| panic!("typed pair on {layout:?}: {error:?}"));
            let [first, reversed, repeated, changed] = plan.calls() else { panic!("four ordinary calls"); };
            assert_eq!([first.evaluator_index(), reversed.evaluator_index(), repeated.evaluator_index(),
                changed.evaluator_index()], [0, 1, 0, 2]);
            for (call, expected) in [(first, [7, 9]), (reversed, [9, 7]), (repeated, [7, 9]), (changed, [7, 11])] {
                let facts = call.facts();
                assert_eq!(facts.tuple(), expected.map(DirectComptimeArgument::U32));
                assert_eq!(facts.site(), first.facts().site());
                assert_eq!(facts.function(), first.facts().function());
                assert_eq!(facts.result(), &ScalarType::U32);
                for (input, (value, name)) in facts.inputs().iter().zip(expected.into_iter().zip(["left", "right"])) {
                    assert_eq!(spelling(&authored, input.formal()), name);
                    assert_eq!(spelling(&authored, input.read()), name);
                    assert_eq!(spelling(&authored, input.argument()), value.to_string());
                }
                assert!(facts.inputs()[0].argument().range.end < facts.inputs()[1].argument().range.start);
                assert!(facts.inputs()[1].read().range.end < facts.inputs()[0].read().range.start);
            }
            let [a, b, different] = plan.evaluators() else { panic!("three fully typed distinct tuples"); };
            let emit_closed = |expression: &str| {
                let root = fresh(&format!("u32 answer = comptime value {{ {expression} }};"));
                super::super::derive_closed_comptime_fragment(&root, "value", layout, |_| None)
                    .unwrap().llvm().to_text()
            };
            assert_eq!(a.llvm().to_text(), emit_closed("9 ^ 7"));
            assert_eq!(b.llvm().to_text(), emit_closed("7 ^ 9"));
            assert_eq!(different.llvm().to_text(), emit_closed("11 ^ 7"));
            assert_eq!(a.llvm().to_text(), b.llvm().to_text());
            assert_ne!(a.llvm().to_text(), different.llvm().to_text());
            assert!(derive_direct_startup_comptime_plan(&authored, "value", layout).is_err());
        }
    }

    #[test]
    fn paired_startup_refuses_a_missing_formal_and_a_late_nonliteral_operand() {
        let layout = ScalarTargetLayout::WASM32;
        let repeated_read = fresh("u32(u32, u32) solve = fn(left, right) { comptime value { left ^ left } };\nu32 first = solve(7, 9);");
        assert!(repeated_read.result().is_valid());
        let Err(ClosedComptimeRejection::OrdinaryDiagnostics(diagnostics)) =
            derive_direct_pair_startup_comptime_plan(&repeated_read, "value", layout)
        else { panic!("the real ordinary typer must refuse the unread formal"); };
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "B0009"
            && diagnostic.labels.iter().any(|label| spelling(&repeated_read, &label.span) == "right")));

        let late = fresh(&format!("{PAIRED_FUNCTION}\nu32 first = solve(7, 9);\nu32 later = solve(9, first);"));
        assert!(late.result().is_valid());
        assert!(matches!(derive_direct_pair_startup_comptime_plan(&late, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "argument must be an exact scalar literal", .. })));

        let short_circuit = fresh("bool(bool, bool) solve = fn(left, right) { comptime value { left && right } };\nbool first = solve(false, true);");
        assert!(short_circuit.result().is_valid());
        assert!(matches!(derive_direct_pair_startup_comptime_plan(&short_circuit, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "paired comptime requires an audited binary read of both formals", .. })));
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
