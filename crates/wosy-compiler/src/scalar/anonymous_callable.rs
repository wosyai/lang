//! Closed inline callable bodies for direct prepared fixed-array owners.
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

pub(crate) struct PreparedAnonymousCallable {
    prepared: PreparedScalarProject,
    results: reference::ProjectReferenceResults,
}

impl PreparedAnonymousCallable {
    pub(crate) fn function(&self) -> &ScalarFunction {
        let ScalarItem::Function(function) = &self.prepared.project().modules[0].items[0]
            else { unreachable!("isolated callable row") };
        function
    }

    pub(crate) fn cfg(&self) -> &ScalarReferenceCfg {
        self.results.reference_cfg(0, 0, None).expect("isolated callable CFG")
    }

    pub(crate) fn typed_nodes(&self) -> Result<control::CallableBodyCallNodes<'_>, String> {
        self.prepared.bind_callable_body_call_nodes(0, 0, None, &self.function().body)
    }
}

fn scalar(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Bool | ScalarType::Char | ScalarType::I8 | ScalarType::I16
        | ScalarType::I32 | ScalarType::I64 | ScalarType::I128 | ScalarType::U8 | ScalarType::U16
        | ScalarType::U32 | ScalarType::U64 | ScalarType::U128 | ScalarType::F32 | ScalarType::F64)
}

pub(super) fn closed_scalar_signature(ty: &ScalarType) -> bool {
    let ScalarType::Callable { parameters, outputs } = ty else { return false; };
    matches!(outputs.outputs.as_slice(), [ScalarOutput { ty, .. }] if scalar(ty)) && parameters.iter().all(scalar)
}

pub(super) fn candidate_fixed_elements(binding: &ScalarBinding) -> Option<&[ScalarExpression]> {
    if binding.receivers.len() != 1 || binding.fallible_assignment.is_some() || binding.is_allocation { return None; }
    let ScalarType::Array { element, length, .. } = &binding.declared_type else { return None; };
    let ScalarExpression::ArrayLiteral { elements, .. } = &binding.value else { return None; };
    if !closed_scalar_signature(element) || *length != elements.len() as u64
        || !elements.iter().any(|item| matches!(item, ScalarExpression::AnonymousCallable { .. }))
        || !elements.iter().all(|item| matches!(item, ScalarExpression::AnonymousCallable { .. } | ScalarExpression::Name { .. }))
    { return None; }
    Some(elements)
}

/// Exact authored parent/child corridor for private lexical generation. A
/// regular named factory is a different case; this must be the actual complete
/// initializer, with a single authored return of the nested callable.
pub(super) struct GeneratedCallableCandidate<'a> {
    pub(super) factory: &'a ScalarExpression,
    pub(super) count: &'a ScalarExpression,
    pub(super) child: &'a ScalarExpression,
    pub(super) element: &'a ScalarType,
    pub(super) invocation_index: Option<&'a (String, ByteSpan)>,
}

impl GeneratedCallableCandidate<'_> {
    pub(super) fn factory_signature(&self) -> ScalarType {
        let span = expression_span(self.child);
        ScalarType::Callable {
            parameters: vec![ScalarType::U64],
            outputs: ScalarOutputSequence {
                outputs: vec![ScalarOutput { ty: self.element.clone(), span }],
                span,
            },
        }
    }
}

pub(super) fn candidate_generated_callable(binding: &ScalarBinding) -> Option<GeneratedCallableCandidate<'_>> {
    if binding.receivers.len() != 1 || binding.fallible_assignment.is_some() || binding.is_allocation {
        return None;
    }
    let ScalarExpression::GeneratedArray { factory, count, .. } = &binding.value else { return None; };
    let element = match (&binding.declared_type, count.as_ref()) {
        (ScalarType::Array { element, length, .. }, ScalarExpression::Integer { value, .. })
            if u64::try_from(value).ok() == Some(*length) => element,
        (ScalarType::RuntimeArray { element, .. }, ScalarExpression::Name { .. } | ScalarExpression::Call { .. }) => element,
        _ => return None,
    };
    if !closed_scalar_signature(element) { return None; }
    let ScalarExpression::AnonymousCallable { parameters, body, .. } = factory.as_ref() else { return None; };
    let [index] = parameters.as_slice() else { return None; };
    if body.unsafe_context || body.items.len() != 1 || body.final_output_values.len() != 1 {
        return None;
    }
    let (child, _) = body.final_expressions().next()?;
    let ScalarExpression::AnonymousCallable { parameters, body, .. } = child else { return None; };
    let ScalarType::Callable { parameters: inputs, .. } = element.as_ref() else { return None; };
    if parameters.len() != inputs.len() || body.unsafe_context { return None; }
    let invocation_index = (index.0 != "_").then_some(index);
    if invocation_index.is_some_and(|index| parameters.iter().any(|child| child.0 == index.0)) {
        // Do not let the internal transport parameter erase authored lexical
        // shadowing. An unneeded non-underscore formal is not in this family.
        return None;
    }
    Some(GeneratedCallableCandidate { factory, count, child, element, invocation_index })
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PreparedInlineOwner {
    pub(crate) module: usize,
    pub(crate) item: usize,
    pub(crate) block: usize,
    pub(crate) binding: usize,
    pub(crate) ordinal: usize,
    pub(crate) rank: usize,
}

struct OwnerTiedAnonymousPool {
    outer: PreparedScalarProject,
    outer_references: reference::ProjectReferenceResults,
    arms: BTreeMap<PreparedInlineOwner, PreparedAnonymousCallable>,
}

impl OwnerTiedAnonymousPool {
    fn arm_for(
        &self, module: usize, item: usize, binding: &ScalarBinding, rank: usize,
    ) -> Option<&PreparedAnonymousCallable> {
        let ScalarItem::Function(function) = self.outer.project().modules.get(module)?.items.get(item)?
            else { return None; };
        let nodes = self.outer.bind_callable_body_call_nodes(module, item, None, &function.body).ok()?;
        let (block, position) = nodes.binding_position(binding)?;
        let value = candidate_fixed_elements(binding)?.get(rank)?;
        let ordinal = nodes.expression_ordinal(value)?;
        self.arms.get(&PreparedInlineOwner { module, item, block, binding: position, ordinal, rank })
    }
}

fn prepare_owner_tied_pool(
    canonical: &CanonicalCstRoot, target: ScalarTargetLayout,
) -> Result<OwnerTiedAnonymousPool, Vec<Diagnostic>> {
    let declarations = derive_scalar_declarations_from_cst(canonical);
    if !declarations.diagnostics.is_empty() { return Err(declarations.diagnostics); }
    let source = declarations.source;
    let mut module = ScalarModule::new(source.clone(), declarations.items, Vec::new());
    module.structs = declarations.structs;
    module.enums = declarations.enums;
    module.target_layout = target;
    let outer = PreparedScalarProject::new(ScalarProject::new(vec![module], vec![source.clone()]))
        .map_err(|error| error.diagnostics.clone())?;
    let checked = analyze_scalar_build(&outer);
    if !checked.diagnostics().is_empty() { return Err(checked.diagnostics().to_vec()); }
    let outer_references = checked.into_reference_results();
    let arms = collect_prepared_inline_arms(&outer)?;
    // The analysis-only entry is not a general collector or a Build mode.
    let authored_count = canonical.root.descendants().filter(|node| node.kind() == SyntaxKind::CallableLiteral).count();
    if arms.len() != authored_count {
        return Err(vec![diagnostic(&source, &canonical.root, "inline callable is outside the admitted direct lexical array owner")]);
    }
    Ok(OwnerTiedAnonymousPool { outer, outer_references, arms })
}

pub(crate) fn actual_inline_element<'a>(
    outer: &'a PreparedScalarProject,
    owner: PreparedInlineOwner,
    nodes: &control::CallableBodyCallNodes<'a>,
) -> Result<&'a ScalarExpression, String> {
    let ScalarItem::Function(fun) = &outer.project().modules[owner.module].items[owner.item]
        else { return Err("inline owner changed its function".into()); };
    let ScalarBlockItem::LocalBinding(binding) = fun.body.items.get(owner.binding)
        .ok_or("inline owner lost its binding")? else { return Err("inline owner changed its binding kind".into()); };
    let value = candidate_fixed_elements(binding).and_then(|values| values.get(owner.rank))
        .ok_or("inline owner lost its element rank")?;
    if nodes.binding_position(binding) != Some((owner.block, owner.binding))
        || nodes.expression_ordinal(value) != Some(owner.ordinal)
        || !matches!(value, ScalarExpression::AnonymousCallable { .. })
    {
        return Err("inline callable no longer belongs to its exact prepared owner".into());
    }
    Ok(value)
}

pub(crate) struct PreparedGeneratedCallable {
    pub(crate) child: PreparedAnonymousCallable,
    pub(crate) captures_index: bool,
}

pub(crate) fn actual_generated_callable<'a>(
    outer: &'a PreparedScalarProject,
    owner: PreparedInlineOwner,
    nodes: &control::CallableBodyCallNodes<'a>,
) -> Result<&'a ScalarExpression, String> {
    let ScalarItem::Function(fun) = &outer.project().modules[owner.module].items[owner.item]
        else { return Err("generated owner changed its function".into()); };
    let Some(ScalarBlockItem::LocalBinding(binding)) = fun.body.items.get(owner.binding)
        else { return Err("generated callable lost its binding".into()); };
    let candidate = candidate_generated_callable(binding)
        .ok_or("generated callable changed its authored initializer")?;
    if nodes.binding_position(binding) != Some((owner.block, owner.binding))
        || nodes.expression_ordinal(candidate.factory) != Some(owner.ordinal) || owner.rank != 0
    { return Err("generated callable changed its exact prepared owner".into()); }
    Ok(&binding.value)
}

pub(super) fn collect_prepared_generated_callables(
    outer: &PreparedScalarProject,
) -> Result<BTreeMap<PreparedInlineOwner, PreparedGeneratedCallable>, Vec<Diagnostic>> {
    let mut arms = BTreeMap::new();
    for (module, actual) in outer.project().modules.iter().enumerate() {
        for (item, value) in actual.items.iter().enumerate() {
            let ScalarItem::Function(fun) = value else { continue; };
            if !fun.generic_parameters.is_empty() || !fun.overload_arms.is_empty() { continue; }
            let runtime_owners: HashSet<_> = fun.body.items.iter().filter_map(|item| {
                let ScalarBlockItem::LocalBinding(binding) = item else { return None; };
                matches!(binding.declared_type, ScalarType::RuntimeArray { .. })
                    .then(|| candidate_generated_callable(binding).map(|_| binding.name_span)).flatten()
            }).collect();
            let mut nodes = None;
            for value in &fun.body.items {
                let ScalarBlockItem::LocalBinding(binding) = value else { continue; };
                let Some(candidate) = candidate_generated_callable(binding) else { continue; };
                if nodes.is_none() {
                    let bound = outer.bind_callable_body_call_nodes(module, item, None, &fun.body)
                        .map_err(|_| vec![arm_diagnostic(&actual.source, fun.span, "missing prepared generated callable owner")])?;
                    if !runtime_owners.is_empty() && !bound.generated_runtime_callable_uses_are_closed(&fun.body, &runtime_owners) {
                        return Err(vec![arm_diagnostic(&actual.source, fun.span,
                            "a generated runtime callable owner allows only direct checked selection or an immediate array_len observation")]);
                    }
                    nodes = Some(bound);
                }
                let nodes = nodes.as_ref().expect("bound generated callable owner");
                let invalid = || vec![arm_diagnostic(&actual.source, binding.span, "missing exact generated callable binding or count")];
                let (block, position) = nodes.binding_position(binding).ok_or_else(invalid)?;
                let ordinal = nodes.expression_ordinal(candidate.factory).ok_or_else(invalid)?;
                if nodes.checked_expression_type(&binding.value) != Some(&binding.declared_type)
                    || nodes.checked_expression_type(candidate.factory) != Some(&candidate.factory_signature())
                    || nodes.checked_expression_type(candidate.count) != Some(&ScalarType::U64)
                { return Err(invalid()); }
                let key = PreparedInlineOwner { module, item, block, binding: position, ordinal, rank: 0 };
                let child = check_isolated_arm_with_capture(&actual.source, candidate.child.clone(),
                    candidate.element.clone(), actual.target_layout, candidate.invocation_index)?;
                if arms.insert(key, PreparedGeneratedCallable { child, captures_index: candidate.invocation_index.is_some() }).is_some() {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(arms)
}

pub(super) fn collect_prepared_inline_arms(
    outer: &PreparedScalarProject,
) -> Result<BTreeMap<PreparedInlineOwner, PreparedAnonymousCallable>, Vec<Diagnostic>> {
    let mut arms = BTreeMap::new();
    for (module, actual) in outer.project().modules.iter().enumerate() {
        for (item, value) in actual.items.iter().enumerate() {
            let ScalarItem::Function(fun) = value else { continue; };
            if !fun.generic_parameters.is_empty() || !fun.overload_arms.is_empty() { continue; }
            let source = &actual.source;
            let target = actual.target_layout;
            let candidates = fun.body.items.iter().filter_map(|item| match item {
                ScalarBlockItem::LocalBinding(binding) => candidate_fixed_elements(binding).map(|elements| (binding, elements)),
                _ => None,
            }).collect::<Vec<_>>();
            if candidates.is_empty() { continue; }
            let nodes = outer.bind_callable_body_call_nodes(module, item, None, &fun.body)
                .map_err(|_| vec![arm_diagnostic(source, fun.span, "missing prepared inline callable owner")])?;
            for (binding, values) in candidates {
                let (block, position) = nodes.binding_position(binding)
                    .ok_or_else(|| vec![arm_diagnostic(source, binding.span, "missing prepared inline callable binding")])?;
                let ScalarType::Array { element, .. } = &binding.declared_type else { unreachable!("typed fixed array"); };
                for (rank, value) in values.iter().enumerate() {
                    if !matches!(value, ScalarExpression::AnonymousCallable { .. }) { continue; }
                    let ordinal = nodes.expression_ordinal(value)
                        .filter(|_| nodes.checked_expression_type(value) == Some(element.as_ref()))
                        .ok_or_else(|| vec![arm_diagnostic(source, expression_span(value), "missing actual typed inline callable element")])?;
                    let key = PreparedInlineOwner { module, item, block, binding: position, ordinal, rank };
                    let arm = check_isolated_arm(source, value.clone(), element.as_ref().clone(), target)?;
                    if arms.insert(key, arm).is_some() {
                        return Err(vec![arm_diagnostic(source, expression_span(value), "duplicate prepared inline callable element")]);
                    }
                }
            }
        }
    }
    Ok(arms)
}

fn diagnostic(source: &SourceIdentity, at: &CstNode, message: &str) -> Diagnostic {
    arm_diagnostic(source, wosy_syntax::byte_span(at), message)
}

fn arm_diagnostic(source: &SourceIdentity, at: ByteSpan, message: &str) -> Diagnostic {
    Diagnostic {
        code: "B0003".to_owned(), severity: DiagnosticSeverity::Error, message: message.to_owned(),
        labels: vec![DiagnosticLabel { kind: DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), at), message: message.to_owned() }],
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
            let authored = derive_expression(&literal);
            let checked = check_isolated_arm(source, authored, signature.clone(), target)?;
            if pool.arms.insert(key, checked).is_some() {
                return Err(vec![diagnostic(source, &literal, "duplicate anonymous callable occurrence")]);
            }
        }
    }
    Ok(pool)
}

fn check_isolated_arm(
    source: &SourceIdentity, expression: ScalarExpression, signature: ScalarType,
    target: ScalarTargetLayout,
) -> Result<PreparedAnonymousCallable, Vec<Diagnostic>> {
    check_isolated_arm_with_capture(source, expression, signature, target, None)
}

fn check_isolated_arm_with_capture(
    source: &SourceIdentity, expression: ScalarExpression, mut signature: ScalarType,
    target: ScalarTargetLayout, capture: Option<&(String, ByteSpan)>,
) -> Result<PreparedAnonymousCallable, Vec<Diagnostic>> {
    let ScalarType::Callable { parameters: inputs, outputs } = &mut signature else { unreachable!("sealed scalar signature") };
    let ScalarExpression::AnonymousCallable { parameters: mut authored, body, span } = expression
        else { unreachable!("authored anonymous callable") };
    if authored.len() != inputs.len() {
        return Err(vec![arm_diagnostic(source, span, "inline callable parameter count differs from its array element signature")]);
    }
    if let Some(capture) = capture {
        // Only the authenticated authored factory formal is in this scope.
        // Its value is a private ABI input, not an added source argument.
        inputs.insert(0, ScalarType::U64);
        authored.insert(0, capture.clone());
    }
    let (parameters, parameter_spans) = authored.into_iter().unzip();
    let mut body = *body;
    for (value, output) in body.final_output_values.iter_mut().zip(&outputs.outputs) {
        value.ty = output.ty.clone();
    }
    // This adapter is never added to the authored project or emitted. Empty
    // spelling is not lexable, so it cannot shadow a name or be called.
    let arm = ScalarFunction { name: String::new(), name_span: span, signature, parameters,
        parameter_spans, body, span, generic_parameters: Vec::new(), overload_arms: Vec::new(),
        runtime_enum_match: None,
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
        return Err(vec![arm_diagnostic(source, span, "inline callable pool admits only closed scalar computation")]);
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
        assert!(derive_scalar_declarations_from_cst(canonical).diagnostics.is_empty());
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

#[cfg(test)]
#[test]
fn anonymous_elements_are_tied_to_actual_prepared_fixed_array_owners() {
    let text = "%%start
u8() named = fn { 7 };
unit() owner = fn {
    u8()[2] first = [fn() { 1 }, fn() { 2 }];
    u8()[2] second = [named, fn() { 3 }];
    first;
    second;
};
%%end";
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/owner-inline.w".into(), "r1".into());
        let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let canonical = parsed.result.canonical_cst();
        let pool = prepare_owner_tied_pool(canonical, target).unwrap_or_else(|diagnostics| panic!("{diagnostics:?}"));
        assert_eq!(pool.arms.len(), 3);
        let [ScalarItem::Function(_), ScalarItem::Function(owner)] = pool.outer.project().modules[0].items.as_slice()
            else { panic!("exact authored owner") };
        let [ScalarBlockItem::LocalBinding(first), ScalarBlockItem::LocalBinding(second), ..] = owner.body.items.as_slice()
            else { panic!("both actual array bindings") };
        assert!(pool.outer_references.reference_cfg(0, 1, None).is_some());
        let first_zero = pool.arm_for(0, 1, first, 0).expect("first[0]");
        let first_one = pool.arm_for(0, 1, first, 1).expect("first[1]");
        let second_one = pool.arm_for(0, 1, second, 1).expect("second[1]");
        assert_ne!(first_zero.function().span, first_one.function().span);
        assert_ne!(first_one.function().span, second_one.function().span);
        assert!(pool.arm_for(0, 1, second, 0).is_none()); // named slot uses the old path
        assert!(pool.arm_for(0, 0, first, 0).is_none()); // other function
        assert!(pool.arm_for(0, 1, &first.clone(), 0).is_none()); // same source metadata, wrong actual node
        assert!(pool.arm_for(0, 1, first, 2).is_none()); // wrong rank
        assert!(derive_scalar_declarations_from_cst(canonical).diagnostics.is_empty());
        assert_eq!(pool.outer.inline_arms().count(), 3);
    }
}
