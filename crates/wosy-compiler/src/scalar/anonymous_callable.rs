//! Private semantic prerequisite. No ordinary Build path consumes the pool.
#![allow(dead_code)]

use super::*;

#[derive(Debug, Eq, Hash, PartialEq)]
struct AnonymousCallableSite {
    source: SourceIdentity,
    owner: ByteSpan,
    binding: ByteSpan,
    element: ByteSpan,
    body: ByteSpan,
    ordinal: usize,
}

struct AnonymousCallablePool {
    target: ScalarTargetLayout,
    arms: HashMap<AnonymousCallableSite, PreparedAnonymousCallable>,
}

struct PreparedAnonymousCallable {
    prepared: PreparedScalarProject,
    results: reference::ProjectReferenceResults,
}

impl PreparedAnonymousCallable {
    fn function(&self) -> &ScalarFunction {
        let ScalarItem::Function(function) = &self.prepared.project().modules[0].items[0]
            else { unreachable!("isolated callable row") };
        function
    }

    fn cfg(&self) -> &ScalarReferenceCfg {
        self.results.reference_cfg(0, 0, None).expect("isolated callable CFG")
    }

    fn typed_nodes(&self) -> Result<control::CallableBodyCallNodes<'_>, String> {
        self.prepared.bind_callable_body_call_nodes(0, 0, None, &self.function().body)
    }
}

fn scalar(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Bool | ScalarType::Char | ScalarType::I8 | ScalarType::I16
        | ScalarType::I32 | ScalarType::I64 | ScalarType::I128 | ScalarType::U8 | ScalarType::U16
        | ScalarType::U32 | ScalarType::U64 | ScalarType::U128 | ScalarType::F32 | ScalarType::F64)
}

fn diagnostic(source: &SourceIdentity, at: &CstNode, message: &str) -> Diagnostic {
    Diagnostic {
        code: "B0003".to_owned(), severity: DiagnosticSeverity::Error, message: message.to_owned(),
        labels: vec![DiagnosticLabel { kind: DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), wosy_syntax::byte_span(at)), message: message.to_owned() }],
        notes: Vec::new(),
    }
}

fn declaration_context(array: &CstNode) -> Option<(CstNode, CstNode, ScalarType, u64)> {
    let owner = array.ancestors().find(|node| node.kind() == SyntaxKind::FunctionDecl)?;
    let binding = array.ancestors().find(|node| node.kind() == SyntaxKind::LocalBinding)?;
    let outputs = binding.children().find(|node| node.kind() == SyntaxKind::OutputList)?;
    let mut values = outputs.children();
    let value = values.next()?;
    // The fixed list must be the entire initializer, not a nested array or
    // argument. Equal authored spans exclude wrappers carrying other tokens.
    if values.next().is_some() || wosy_syntax::byte_span(array) != wosy_syntax::byte_span(&value)
        || !array.ancestors().any(|node| node == value)
    { return None; }
    let receivers = binding.children().find(|node| node.kind() == SyntaxKind::ReceiverList)?;
    let mut receivers = receivers.children();
    let receiver = receivers.next()?;
    if receivers.next().is_some() { return None; }
    let ty = derive_type(&receiver.children().next()?);
    let ScalarType::Array { element, length, .. } = ty else { return None; };
    let ScalarType::Callable { parameters, outputs } = element.as_ref() else { return None; };
    let [output] = outputs.outputs.as_slice() else { return None; };
    (scalar(&output.ty) && parameters.iter().all(scalar)).then_some((owner, binding, *element, length))
}

fn prepare_anonymous_pool(
    canonical: &CanonicalCstRoot, target: ScalarTargetLayout,
) -> Result<AnonymousCallablePool, Vec<Diagnostic>> {
    let source = &canonical.source;
    let mut pool = AnonymousCallablePool { target, arms: HashMap::new() };
    for array in canonical.root.descendants().filter(|node| node.kind() == SyntaxKind::ArrayLiteral) {
        if !array.children().any(|node| node.kind() == SyntaxKind::CallableLiteral) { continue; }
        let Some((owner, binding, signature, length)) = declaration_context(&array) else {
            return Err(vec![diagnostic(source, &array, "inline callables require a complete lexical fixed array with a scalar signature")]);
        };
        let elements = array.children().collect::<Vec<_>>();
        if length != elements.len() as u64 || elements.iter().any(|node| node.kind() != SyntaxKind::CallableLiteral) {
            return Err(vec![diagnostic(source, &array, "anonymous callable pool requires a complete inline list")]);
        }
        for (ordinal, literal) in elements.into_iter().enumerate() {
            let body = literal.children().find(|node| node.kind() == SyntaxKind::Block).expect("authored callable body");
            if body.descendants().any(|node| matches!(node.kind(),
                SyntaxKind::CallableLiteral | SyntaxKind::InvalidTemporaryPlace))
            {
                return Err(vec![diagnostic(source, &literal, "nested inline callables are not in this closed scalar pool")]);
            }
            let key = AnonymousCallableSite { source: source.clone(), owner: wosy_syntax::byte_span(&owner),
                binding: wosy_syntax::byte_span(&binding), element: wosy_syntax::byte_span(&literal),
                body: wosy_syntax::byte_span(&body), ordinal };
            let checked = check_isolated_arm(source, &literal, &body, signature.clone(), target)?;
            if pool.arms.insert(key, checked).is_some() {
                return Err(vec![diagnostic(source, &literal, "duplicate anonymous callable occurrence")]);
            }
        }
    }
    Ok(pool)
}

fn check_isolated_arm(
    source: &SourceIdentity, literal: &CstNode, body: &CstNode, signature: ScalarType,
    target: ScalarTargetLayout,
) -> Result<PreparedAnonymousCallable, Vec<Diagnostic>> {
    let ScalarType::Callable { parameters: inputs, outputs } = &signature else { unreachable!("sealed scalar signature") };
    let mut parameters = Vec::new();
    let mut parameter_spans = Vec::new();
    if let Some(list) = literal.children().find(|node| node.kind() == SyntaxKind::Parameters) {
        for token in list.children_with_tokens().filter_map(NodeOrToken::into_token)
            .filter(|token| token.kind() == SyntaxKind::Identifier)
        {
            parameters.push(token.text().to_owned());
            parameter_spans.push(token_span(&token));
        }
    }
    if parameters.len() != inputs.len() {
        return Err(vec![diagnostic(source, literal, "inline callable parameter count differs from its array element signature")]);
    }
    let mut body = derive_block(body);
    for (value, output) in body.final_output_values.iter_mut().zip(&outputs.outputs) {
        value.ty = output.ty.clone();
    }
    let span = wosy_syntax::byte_span(literal);
    // This adapter is never added to the authored project or emitted. Empty
    // spelling is not lexable, so it cannot shadow a name or be called.
    let arm = ScalarFunction { name: String::new(), name_span: span, signature, parameters,
        parameter_spans, body, span, generic_parameters: Vec::new(), overload_arms: Vec::new(),
        reference_cfg: ScalarReferenceCfg {
            facts: Vec::new(), points: Vec::new(), initial_points: Vec::new(), allocations: BTreeSet::new(),
            specialized_policies: Vec::new(), observed_runtime_allocations: None, guarded_scalar_allocation: None,
            primary_call_points: HashMap::new(), runtime_array_read_sites: HashMap::new(),
            runtime_array_child_sites: Vec::new(), runtime_array_exclusive_sites: HashMap::new(),
            runtime_array_field_sites: HashMap::new(),
        },
    };
    // An anonymous slot has no module member or initialization entry.
    let mut module = ScalarModule::new(source.clone(), Vec::new(), Vec::new());
    module.items.push(ScalarItem::Function(arm));
    module.target_layout = target;
    let prepared = PreparedScalarProject::new_anonymous_arm(module)
        .map_err(|error| error.diagnostics.clone())?;
    let checked = analyze_scalar_build(&prepared);
    if !checked.diagnostics().is_empty() { return Err(checked.diagnostics().to_vec()); }
    let cfg = checked.reference_cfg(0, 0, None).expect("ordinary callable CFG");
    let ScalarItem::Function(arm) = &prepared.project().modules[0].items[0] else { unreachable!() };
    if !crate::llvm::closed_scalar_target(arm, cfg) {
        return Err(vec![diagnostic(source, literal, "inline callable pool admits only closed scalar computation")]);
    }
    let results = checked.into_reference_results();
    Ok(PreparedAnonymousCallable { prepared, results })
}

#[cfg(test)]
#[test]
fn anonymous_fixed_list_is_prepared_without_inheriting_authored_scope() {
    fn inspect(text: &str, layout: ScalarTargetLayout) -> Result<AnonymousCallablePool, Vec<Diagnostic>> {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/inline.w".into(), "r1".into());
        let parsed = crate::parse_source(source, text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let canonical = parsed.result.canonical_cst();
        assert!(derive_scalar_declarations_from_cst(canonical).diagnostics.iter().any(|diagnostic|
            diagnostic.message == "inline callable array elements are not available for Build yet"));
        prepare_anonymous_pool(canonical, layout)
    }
    let ordinary = "%%start
unit() store = fn {
    u8()[2] operations = [fn() { 1 }, fn() { 2 }];
    operations;
};
%%end";
    let parameters = ordinary.replace("u8()[2]", "u8(u8)[2]")
        .replace("fn() { 1 }, fn() { 2 }", "fn(x) { x + 1 }, fn(y) { y + 2 }");
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for text in [ordinary, parameters.as_str()] {
            let actual = inspect(text, target).unwrap_or_else(|errors| panic!("{errors:?}"));
            assert_eq!(actual.target, target);
            assert_eq!(actual.arms.len(), 2);
            for (site, arm) in actual.arms {
                let function = arm.function();
                assert_eq!(site.element, function.span);
                assert!(function.name.is_empty());
                assert!(matches!(arm.cfg().points.first().map(|point| &point.kind), Some(CfgPointKind::Entry)));
                assert!(arm.cfg().points.iter().any(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. })));
                let Some(ScalarBlockItem::Expression(output)) = function.body.items.last()
                    else { panic!("authored scalar output") };
                let typed = arm.typed_nodes().unwrap();
                assert!(typed.expression_ordinal(output).is_some());
                assert_eq!(typed.checked_expression_type(output), Some(&ScalarType::U8));
                assert!(site.binding.start > site.owner.start && site.body.start > site.element.start);
            }
        }
        let capture = ordinary.replace("    u8()[2]", "    u8 outer = 1;\n    u8()[2]")
            .replace("fn() { 1 }", "fn() { outer }");
        let capture = capture.replace("\\n", "\n");
        let errors = inspect(&capture, target).err().expect("outer scope must not be captured");
        assert!(errors.iter().any(|d| d.code == "B0001"
            && d.labels.iter().any(|label| capture.get(label.span.range.start as usize..label.span.range.end as usize) == Some("outer"))),
            "{errors:?}");
        let wrong_arity = ordinary.replace("fn() { 1 }", "fn(x) { x }");
        let errors = inspect(&wrong_arity, target).err().expect("parameter arity must match");
        assert!(errors.iter().any(|d| d.message.contains("parameter count differs")), "{errors:?}");
        let incomplete = ordinary.replace("u8()[2]", "u8()[3]");
        assert!(inspect(&incomplete, target).is_err());
        let wrong_return = ordinary.replace("fn() { 1 }", "fn() { true }");
        assert!(inspect(&wrong_return, target).is_err());
    }
}
