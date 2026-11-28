//! Reachability for one paired comptime producer through explicit Wasm roots.
use super::*;
use crate::llvm::validate_wasm_root_export_roster;
use std::collections::{BTreeMap, HashSet};

pub struct ExportedPairWasip1 {
    source: CanonicalExportSource,
    site: SourceSpan,
    id: String,
    exports: BTreeMap<String, String>,
    uses: Vec<SelectedExportUse>,
    evaluators: Vec<BoundedStartupWasip1Evaluator>,
    tuples: Vec<[DirectComptimeArgument; 2]>,
}

struct CanonicalExportSource {
    source: wosy_syntax::SourceIdentity,
    tree: GreenNode,
    site: SyntaxNodePtr<WosyLanguage>,
    tail: SyntaxNodePtr<WosyLanguage>,
}

struct SelectedExportUse {
    declaration: ByteSpan,
    call: ByteSpan,
    evaluator: usize,
}

impl ExportedPairWasip1 {
    pub fn id(&self) -> &str { &self.id }
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn evaluators(&self) -> &[BoundedStartupWasip1Evaluator] { &self.evaluators }
    pub fn tuples(&self) -> &[[DirectComptimeArgument; 2]] { &self.tuples }

    /// This continuation can only emit the authenticated single-module
    /// WASM32 source below, with exactly the selected export names and
    /// patched call sites. It never searches configuration or imports.
    pub fn selected_compiler_inputs(&self) -> Result<Vec<u8>, serde_json::Error> {
        let routes = self.uses.iter().map(|route| (route.declaration, route.call, route.evaluator)).collect::<Vec<_>>();
        serde_json::to_vec(&(&self.source.source, "wasm32-wasip1", &self.exports, routes))
    }


    pub fn emit(&self, values: &[ScalarExpression]) -> Result<crate::LlvmPartition, ClosedComptimeRejection> {
        use ClosedComptimeRejection as Error;
        let unsupported = |feature| Error::Unsupported { site: self.site.clone(), feature };
        if values.len() != self.evaluators.len() {
            return Err(unsupported("incomplete selected export results"));
        }
        let values = values.iter().zip(&self.evaluators).map(|(value, evaluator)| {
            concrete_scalar_literal(value, evaluator.result_type())
                .ok_or_else(|| unsupported("selected export result type changed"))
        }).collect::<Result<Vec<_>, _>>()?;
        let root = CstNode::new_root(self.source.tree.clone());
        let node = self.source.site.to_node(&root);
        let tail = self.source.tail.to_node(&root);
        let scoped = ComptimeCst::from_node(&node).ok_or_else(|| unsupported("selected export site changed"))?;
        let body = ScalarExpression::Block(derive_block(&scoped.block));
        let mut declarations = derive_scalar_declarations_with_embedding(
            &CanonicalCstRoot::new(self.source.source.clone(), root),
            Some(EmbeddedComptimeExpression { site: &node, expression: &tail, value: &body }),
        );
        if !declarations.diagnostics.is_empty() { return Err(Error::OrdinaryDiagnostics(declarations.diagnostics)); }
        let by_declaration = self.uses.iter().map(|route| (route.declaration.start, route))
            .collect::<std::collections::HashMap<_, _>>();
        let mut retained = Vec::with_capacity(self.uses.len());
        for item in declarations.items.drain(..) {
            let ScalarItem::Function(mut function) = item else {
                return Err(unsupported("selected export declarations changed"));
            };
            let Some(route) = by_declaration.get(&function.name_span.start) else { continue; };
            let [ScalarBlockItem::Expression(item)] = function.body.items.as_mut_slice() else {
                return Err(unsupported("selected export no longer has one final expression"));
            };
            let [expression] = function.body.expressions.as_mut_slice() else {
                return Err(unsupported("selected export no longer has one expression"));
            };
            let [output] = function.body.final_output_values.as_mut_slice() else {
                return Err(unsupported("selected export no longer has one output"));
            };
            if function.name_span != route.declaration || span_of(item) != route.call {
                return Err(unsupported("selected export call changed"));
            }
            let value = typed_startup_literal(values[route.evaluator], route.call);
            *item = value.clone();
            *expression = value.clone();
            output.value = value;
            retained.push(ScalarItem::Function(function));
        }
        if retained.len() != self.uses.len() {
            return Err(unsupported("selected export set changed"));
        }
        declarations.items = retained;
        let prepared = prepare_scalar_project(vec![(declarations, Vec::new())], vec![self.source.source.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 })
            .map_err(|error| Error::OrdinaryDiagnostics(error.into_ordered_diagnostics()))?;
        let build = analyze_scalar_build(&prepared);
        if !build.diagnostics().is_empty() { return Err(Error::OrdinaryDiagnostics(build.diagnostics().to_vec())); }
        crate::ScalarArtifactEmission::WasmRootExports(&self.source.source, &self.exports)
            .emit(&build).map_err(Error::Emitter)
    }
}

/// All reachable paths in this explicit partition are whole typed scalar
/// tails. No ordinary initializer or executable root is dropped; such a
/// module needs the general mixed-root reachability graph.
pub fn prepare_exported_pair_wasip1(
    fresh: &FreshParseResult, exports: &BTreeMap<String, String>, max_ir_bytes: usize,
) -> Result<ExportedPairWasip1, ClosedComptimeRejection> {
    use ClosedComptimeRejection as Error;
    let parsed = fresh.result();
    if !parsed.is_valid() { return Err(Error::InvalidParse); }
    if exports.is_empty() { return Err(Error::SiteNotFound); }
    let scope = |range| SourceSpan::new(parsed.source.clone(), range);
    let mut sites = parsed.root.descendants().filter_map(|node| {
        let site = ComptimeCst::from_node(&node)?;
        Some((node, site))
    });
    let (node, site) = sites.next().ok_or(Error::SiteNotFound)?;
    let site_span = scope(site.span);
    let unsupported = |feature| Error::Unsupported { site: site_span.clone(), feature };
    if sites.next().is_some() { return Err(unsupported("explicit exports require one paired comptime producer")); }
    let (tail, declaration, _) = function_tail(&node)
        .ok_or_else(|| unsupported("expected one complete comptime function tail"))?;
    let function_node = declaration.children().find(|child| child.kind() == SyntaxKind::FunctionDecl)
        .unwrap_or(declaration);
    let producer_name = super::super::super::direct_token(&function_node, SyntaxKind::Identifier)
        .ok_or_else(|| unsupported("authored comptime function identity is missing"))?;
    let producer_name = super::super::super::token_span(&producer_name);
    sole_final_value(&site.block).filter(primitive_operand_source)
        .ok_or_else(|| unsupported("expected one authored paired operation"))?;
    let body = derive_block(&site.block);
    let [ScalarBlockItem::Expression(ScalarExpression::Binary { operator, left, right, .. })] =
        body.items.as_slice() else { return Err(unsupported("expected one audited paired scalar operation")); };
    if !audited_closed_eager_binary(operator) || !is_name(left) || !is_name(right) {
        return Err(unsupported("expected an audited operation reading two exact formals"));
    }
    let authored_reads = [(FormalOperand::Left, span_of(left)), (FormalOperand::Right, span_of(right))];
    let authored = ScalarExpression::Block(body.clone());
    let declarations = derive_scalar_declarations_with_embedding(parsed.canonical_cst(),
        Some(EmbeddedComptimeExpression { site: &node, expression: &tail, value: &authored }));
    if declarations.items.iter().any(|item| !matches!(item, ScalarItem::Function(_))) {
        return Err(unsupported("explicit scalar roots cannot omit ordinary startup items"));
    }
    let prepared = prepare_scalar_project(vec![(declarations, Vec::new())], vec![parsed.source.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 })
        .map_err(|error| Error::OrdinaryDiagnostics(error.into_ordered_diagnostics()))?;
    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() { return Err(Error::OrdinaryDiagnostics(build.diagnostics().to_vec())); }
    validate_wasm_root_export_roster(&build, &parsed.source, exports).map_err(Error::Emitter)?;
    let module = &prepared.project().modules[0];
    if prepared.initialization_region(0).is_none_or(|region| !region.steps.is_empty()) {
        return Err(unsupported("explicit scalar roots require an empty ordinary startup region"));
    }
    let (index, producer) = module.items.iter().enumerate().find_map(|(index, item)| match item {
        ScalarItem::Function(function) if function.name_span == producer_name => Some((index, function)),
        _ => None,
    }).ok_or_else(|| unsupported("checked producer declaration disappeared"))?;
    let [ScalarBlockItem::Expression(ScalarExpression::Block(typed_body))] = producer.body.items.as_slice()
        else { return Err(unsupported("checked producer no longer owns the exact comptime tail")); };
    let [ScalarBlockItem::Expression(typed)] = typed_body.items.as_slice()
        else { return Err(unsupported("checked producer no longer has the exact output")); };
    let points = build.callable_point_facts(0, index, None)
        .ok_or_else(|| unsupported("ordinary parameter binding facts are unavailable"))?;
    let nodes = prepared.bind_callable_body_call_nodes(0, index, None, &producer.body)
        .map_err(|_| unsupported("ordinary callable occurrences are unavailable"))?;
    let result_type = nodes.checked_expression_type(typed)
        .ok_or_else(|| unsupported("ordinary comptime result type is unavailable"))?;
    if producer.parameters.len() != 2 || !matches!(result_type, ScalarType::I32 | ScalarType::U32) {
        return Err(unsupported("expected an i32/u32 producer with two exact formals"));
    }
    let mut reads = Vec::with_capacity(2);
    for (operand, span) in authored_reads {
        let read = operand.get(typed).filter(|read| span_of(read) == span)
            .ok_or_else(|| unsupported("ordinary block lost an authored formal read"))?;
        let Some(PlainNameIdentity::Lexical(binding)) = nodes.plain_name(read) else {
            return Err(unsupported("ordinary read is not an exact lexical formal"));
        };
        if nodes.checked_expression_type(read) != Some(result_type) { return Err(unsupported("ordinary formal read type changed")); }
        reads.push((operand, span, binding));
    }
    let mut formal_reads = Vec::with_capacity(2);
    for ordinal in 0..2 {
        let (formal, ty) = nodes.formal(points, ordinal)
            .ok_or_else(|| unsupported("ordinary formal identity is unavailable"))?;
        if ty != result_type { return Err(unsupported("ordinary formal type differs from the result")); }
        let read = reads.iter().position(|(_, _, binding)| *binding == formal)
            .ok_or_else(|| unsupported("the paired body does not read both exact formals"))?;
        let (operand, span, _) = reads.swap_remove(read);
        formal_reads.push((operand, span, formal.declaration_span));
    }

    let selected: HashSet<_> = exports.values().map(String::as_str).collect();
    let names: HashSet<_> = module.items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) => Some(function.name.as_str()), _ => None,
    }).collect();
    if exports.iter().any(|(label, name)| label != name && names.contains(label.as_str())) {
        return Err(unsupported("an export label collides with an authored function"));
    }
    let mut calls = Vec::with_capacity(selected.len());
    let mut uses = Vec::with_capacity(selected.len());
    for (index, item) in module.items.iter().enumerate() {
        let ScalarItem::Function(wrapper) = item else { unreachable!("ordinary function-only projection"); };
        if !selected.contains(wrapper.name.as_str()) { continue; }
        if !wrapper.parameters.is_empty() || wrapper.body.unsafe_context {
            return Err(unsupported("selected export requires a safe zero-argument entry"));
        }
        let [ScalarBlockItem::Expression(call)] = wrapper.body.items.as_slice()
            else { return Err(unsupported("selected export requires one complete ordinary call")); };
        let ScalarExpression::Call { receiver: None, type_arguments, arguments, .. } = call else {
            return Err(unsupported("selected export requires an exact direct call"));
        };
        let call_nodes = prepared.bind_callable_body_call_nodes(0, index, None, &wrapper.body)
            .map_err(|_| unsupported("selected export call facts are unavailable"))?;
        let call_points = build.callable_point_facts(0, index, None)
            .ok_or_else(|| unsupported("selected export evaluation facts are unavailable"))?;
        if !type_arguments.is_empty() || arguments.len() != 2
            || call_nodes.final_output(call) != Some(0)
            || call_nodes.evaluation_point(call_points, call).is_none()
            || call_nodes.checked_expression_type(call) != Some(result_type)
            || !matches!(call_nodes.target(call), Ok(ResolvedCallTarget::ModuleCallable {
                source, declaration_span, concrete: ConcreteCallSelection::Declaration,
            }) if source == &parsed.source && declaration_span == &producer_name)
        {
            return Err(unsupported("selected export must directly select the verified comptime producer"));
        }
        let mut inputs = Vec::with_capacity(2);
        for (argument, &(operand, read, formal)) in arguments.iter().zip(&formal_reads) {
            if call_nodes.checked_expression_type(argument) != Some(result_type) {
                return Err(unsupported("selected export argument does not have the exact ordinary type"));
            }
            let value = concrete_scalar_literal(argument, result_type)
                .ok_or_else(|| unsupported("selected export argument is not an exact concrete scalar"))?;
            let argument = scope(span_of(argument));
            inputs.push(DirectComptimeInput {
                origin: StartupArgumentOrigin { literal: argument.clone(), prior_bypass_slot: None },
                argument, formal: scope(formal), read: scope(read), operand, value,
            });
        }
        uses.push(SelectedExportUse { declaration: wrapper.name_span, call: span_of(call), evaluator: 0 });
        calls.push(InspectedDirectComptimeCall {
            site: site_span.clone(), function: scope(producer_name), call: scope(span_of(call)), destination: scope(wrapper.name_span),
            inputs: inputs.try_into().map_err(|_| unsupported("selected pair arity changed"))?, result: result_type.clone(),
        });
    }
    if uses.len() != selected.len() { return Err(unsupported("selected export set is incomplete")); }
    let (calls, fragments) = emit_startup_plan(site.id.text(), ScalarTargetLayout::WASM32, &body, producer_name,
        calls, Some(super::super::OrderedClosedWasip1::MAX_STDOUT))?.into_parts();
    let mut tuples = Vec::with_capacity(fragments.len());
    for (used, call) in uses.iter_mut().zip(&calls) {
        used.evaluator = call.evaluator_index();
        if used.evaluator == tuples.len() { tuples.push(call.facts().tuple()); }
    }
    let mut remaining = max_ir_bytes;
    let mut evaluators = Vec::with_capacity(fragments.len());
    for ClosedComptimeFragment { llvm, result_type, site, .. } in fragments {
        let bytes = llvm.to_bounded_text_bytes(remaining).map_err(|error| Error::Emitter(error.to_string()))?;
        remaining -= bytes.len();
        evaluators.push(BoundedStartupWasip1Evaluator { ir: std::sync::Arc::new(bytes), result_type, output_span: site.range });
    }
    Ok(ExportedPairWasip1 {
        source: CanonicalExportSource { source: parsed.source.clone(), tree: parsed.root.green().into_owned(),
            site: SyntaxNodePtr::new(&node), tail: SyntaxNodePtr::new(&tail) },
        site: site_span, id: site.id.text().to_owned(), exports: exports.clone(), uses, evaluators, tuples,
    })
}
