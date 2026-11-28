//! A complete authored-order batch of independent, closed root initializers.
use super::{
    analyze_scalar_build, collect_scalar_diagnostics, derive_block, emit_closed_evaluator,
    prepare_scalar_project, rewrite_body, root_receiver_and_initializer, single_binding_type,
    supported_type, ClosedComptimeRejection as Refusal, ScalarPreparationInputs,
    ScalarTargetLayout, ScalarType,
};
use super::super::{derive_scalar_declarations_with_embeddings, derive_type, EmbeddedComptimeExpression};
use crate::{emit_scalar_build_single_module_artifact_llvm, LlvmPartition, ScalarExpression, UnaryOperator};
use rowan::{ast::SyntaxNodePtr, GreenNode};
use std::collections::HashSet;
use wosy_syntax::{byte_span, CanonicalCstRoot, ComptimeCst, CstNode, FreshParseResult,
    SourceIdentity, SourceSpan, SyntaxKind, WosyLanguage};

pub struct OrderedClosedWasip1 {
    source: SourceIdentity,
    tree: GreenNode,
    evaluators: Vec<OrderedClosedWasip1Site>,
}

pub struct OrderedClosedWasip1Site {
    id: String,
    site: SyntaxNodePtr<WosyLanguage>,
    initializer: SyntaxNodePtr<WosyLanguage>,
    source_span: SourceSpan,
    result_type: ScalarType,
    ir: Vec<u8>,
}

impl OrderedClosedWasip1Site {
    pub fn id(&self) -> &str { &self.id }
    pub fn source_span(&self) -> &SourceSpan { &self.source_span }
    pub fn ir(&self) -> &[u8] { &self.ir }
    pub fn result_type(&self) -> &ScalarType { &self.result_type }
    pub fn read_stdout(&self, stdout: &[u8]) -> Result<ScalarExpression, super::ComptimeStdoutError> {
        super::stdout::parse_scalar(&self.result_type, self.source_span.range, stdout, OrderedClosedWasip1::MAX_STDOUT)
    }
}

impl OrderedClosedWasip1 {
    // A scalar is a Boolean or a signed/unsigned 32-bit decimal plus LF.
    pub const MAX_STDOUT: usize = b"-2147483648\n".len();
    pub fn is_root_initializer(site: &CstNode) -> bool {
        root_receiver_and_initializer(site).is_some()
    }

    pub fn evaluators(&self) -> &[OrderedClosedWasip1Site] { &self.evaluators }

    /// One compiler-decoded value per site, in authored order. The
    /// caller authenticates result ownership, and this continuation
    /// re-enters the ordinary typer with the original destinations.
    pub fn emit_with_values(&self, values: Vec<ScalarExpression>) -> Result<LlvmPartition, Refusal> {
        if values.len() != self.evaluators.len() {
            return Err(Refusal::Emitter("incomplete selected comptime result set".into()));
        }
        let values = values.into_iter().map(|value| match value {
            ScalarExpression::Integer { ref value, span } if value.sign() == num_bigint::Sign::Minus =>
                ScalarExpression::Unary { operator: UnaryOperator::Negate,
                    operand: Box::new(ScalarExpression::Integer { value: -value, span }), span },
            value => value,
        }).collect::<Vec<_>>();
        let root = CstNode::new_root(self.tree.clone());
        let owners = self.evaluators.iter().map(|item| (
            item.site.to_node(&root), item.initializer.to_node(&root),
        )).collect::<Vec<_>>();
        let embeddings = owners.iter().zip(&values).map(|((site, expression), value)| {
            EmbeddedComptimeExpression { site, expression, value }
        }).collect::<Vec<_>>();
        let declarations = derive_scalar_declarations_with_embeddings(
            &CanonicalCstRoot::new(self.source.clone(), root), &embeddings,
        );
        let prepared = prepare_scalar_project(vec![(declarations, Vec::new())],
            vec![self.source.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).map_err(|refusal| Refusal::OrdinaryDiagnostics(refusal.into_ordered_diagnostics()))?;
        let build = analyze_scalar_build(&prepared);
        if !build.diagnostics().is_empty() { return Err(Refusal::OrdinaryDiagnostics(build.diagnostics().to_vec())); }
        emit_scalar_build_single_module_artifact_llvm(&build).map_err(Refusal::Emitter)
    }
}

/// Enumerate the entire verified authored file once. Closed bodies cannot
/// observe another initializer, so this subset has no value dependencies;
/// their independent tool invocations still run in authored source order.
pub fn prepare_ordered_closed_wasip1(
    fresh: &FreshParseResult, max_ir_bytes: usize,
) -> Result<OrderedClosedWasip1, Refusal> {
    let parsed = fresh.result();
    if !parsed.is_valid() { return Err(Refusal::InvalidParse); }
    let source = &parsed.source;
    let scope = |range| SourceSpan::new(source.clone(), range);
    let unsupported = |range, feature| Refusal::Unsupported { site: scope(range), feature };
    let mut authored = Vec::new();
    let mut names = HashSet::new();
    for node in parsed.root.descendants() {
        let Some(site) = ComptimeCst::from_node(&node) else { continue; };
        let id = site.id.text();
        if !names.insert(id.to_owned()) { return Err(Refusal::RepeatedSite(scope(site.span))); }
        let root = root_receiver_and_initializer(&node)
            .ok_or_else(|| unsupported(site.span, "expected a complete explicitly typed root initializer"))?;
        let result_type = derive_type(&root.declaration);
        if !supported_type(&result_type) { return Err(unsupported(site.span, "unsupported closed scalar result type")); }
        for child in site.block.descendants() {
            match child.kind() {
                SyntaxKind::ComptimeExpr => return Err(unsupported(byte_span(&child), "nested comptime")),
                SyntaxKind::Assignment => return Err(unsupported(byte_span(&child), "assignment")),
                SyntaxKind::LocalBinding if single_binding_type(&child).is_none() =>
                    return Err(unsupported(byte_span(&child), "local binding must have one typed value")),
                _ => {}
            }
        }
        let (diagnostics, _) = collect_scalar_diagnostics(
            &CanonicalCstRoot::new(source.clone(), site.block.clone()), None, |_| {},
        );
        if !diagnostics.is_empty() { return Err(Refusal::OrdinaryDiagnostics(diagnostics)); }
        let mut body = derive_block(&site.block);
        let inputs = rewrite_body(&mut body, &result_type, &scope, &|_| None)?;
        debug_assert!(inputs.is_empty(), "closed site has no parameter authority");
        authored.push((id.to_owned(), node, root.initializer, scope(site.span),
            super::super::token_span(&site.id), result_type, ScalarExpression::Block(body)));
    }
    if authored.is_empty() { return Err(Refusal::SiteNotFound); }
    // Resolve the ordinary module with its real authored closed bodies,
    // not guessed result literals. No invocation may occur if the program
    // or a declared result destination fails ordinary type/CFG checking.
    let embeddings = authored.iter().map(|(_, site, expression, _, _, _, value)|
        EmbeddedComptimeExpression { site, expression, value }).collect::<Vec<_>>();
    let declarations = derive_scalar_declarations_with_embeddings(parsed.canonical_cst(), &embeddings);
    let prepared = prepare_scalar_project(vec![(declarations, Vec::new())], vec![source.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 })
        .map_err(|refusal| Refusal::OrdinaryDiagnostics(refusal.into_ordered_diagnostics()))?;
    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() { return Err(Refusal::OrdinaryDiagnostics(build.diagnostics().to_vec())); }

    let mut evaluators = Vec::with_capacity(authored.len());
    let mut remaining = max_ir_bytes;
    for (id, site, initializer, source_span, name, result_type, value) in authored {
        let ScalarExpression::Block(body) = value else { unreachable!("retained authored comptime block"); };
        let fragment = emit_closed_evaluator(&id, source_span.clone(), name,
            ScalarTargetLayout::WASM32, result_type.clone(), Vec::new(), body, Some(OrderedClosedWasip1::MAX_STDOUT))?;
        let ir = fragment.llvm().to_bounded_text_bytes(remaining)
            .map_err(|error| Refusal::Emitter(error.to_string()))?;
        remaining -= ir.len();
        evaluators.push(OrderedClosedWasip1Site {
            id, site: SyntaxNodePtr::new(&site), initializer: SyntaxNodePtr::new(&initializer), source_span,
            result_type, ir,
        });
    }
    Ok(OrderedClosedWasip1 { source: source.clone(), tree: parsed.root.green().into_owned(), evaluators })
}
