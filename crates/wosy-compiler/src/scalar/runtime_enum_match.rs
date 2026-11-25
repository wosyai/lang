//! Guarded source preparation for the ordinary runtime enum match. This sealed body carries only checked, scope-limited scalar values.
use super::{proof_match::{AuthoredMatch, Payload, Selector},
    typecheck::{concrete_integer_literal, integer_literal_fits_type},
    *};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) source: SourceIdentity,
    pub(crate) owner: ByteSpan,
    pub(crate) span: ByteSpan,
    pub(crate) enumeration: ScalarEnumId,
    name_span: ByteSpan,
    inputs: Vec<(ByteSpan, ScalarType)>,
    pub(crate) output: ScalarType,
    pub(crate) selector: TypedSelector,
    pub(crate) arms: Vec<TypedArm>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TypedSelector {
    Constructor { tag: u32, span: ByteSpan, payload: Vec<(ByteSpan, Operand)> },
    If { predicate: Predicate, yes: Box<Self>, no: Box<Self> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Operand { Literal(BigInt), Formal(ByteSpan) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Predicate { Constant(bool), Formal { use_span: ByteSpan, declaration: ByteSpan } }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypedArm {
    pub(crate) tag: u32,
    pub(crate) scope: ByteSpan,
    pub(crate) bindings: Vec<CaseBinding>,
    pub(crate) result: ArmResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaseBinding {
    pub(crate) slot: usize,
    pub(crate) declaration: ByteSpan,
    pub(crate) ty: ScalarType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArmResult {
    Literal { value: BigInt, span: ByteSpan },
    Payload { slot: usize, use_span: ByteSpan },
}

pub(crate) fn diagnostic(source: &SourceIdentity, span: ByteSpan, message: &str) -> Diagnostic {
    Diagnostic { code: "B0003".to_owned(), severity: DiagnosticSeverity::Error,
        message: message.to_owned(), labels: vec![DiagnosticLabel { kind: DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span), message: message.to_owned() }], notes: Vec::new() }
}

type Formals = HashMap<String, (ByteSpan, ScalarType)>;

pub(crate) fn prepare(source: &SourceIdentity, enums: &[ScalarEnum], body: &CstNode, work: &mut usize) -> Result<Plan, Diagnostic> {
    let span = wosy_syntax::byte_span(body);
    let error = |message| diagnostic(source, span, message);
    let owner = body.parent().filter(|owner| owner.kind() == SyntaxKind::FunctionDecl)
        .ok_or_else(|| error("runtime match must be the sole result of a named function"))?;
    if owner.parent().is_some_and(|parent| parent.kind() == SyntaxKind::GenericFunctionDecl)
        || owner.children_with_tokens().any(|element| element.kind() == SyntaxKind::Unsafe)
    {
        return Err(error("runtime enum match requires a safe nongeneric function"));
    }
    let signature = owner.children().find(|node| node.kind() == SyntaxKind::CallableType)
        .ok_or_else(|| error("missing runtime enum match signature"))?;
    let ScalarType::Callable { outputs, parameters } = derive_type(&signature) else {
        return Err(error("invalid runtime enum match signature"));
    };
    let [result] = outputs.outputs.as_slice() else { return Err(error("runtime enum match requires one integer result")); };
    if !is_integer_type(&result.ty) || !parameters.iter().all(|ty| is_integer_type(ty) || *ty == ScalarType::Bool) {
        return Err(error("runtime enum match requires scalar integer results and primitive formals"));
    }
    let names = owner.children().find(|node| node.kind() == SyntaxKind::Parameters).into_iter()
        .flat_map(|parameters| parameters.children_with_tokens()).filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Identifier).collect::<Vec<_>>();
    if names.len() != parameters.len() { return Err(error("runtime match parameter arity mismatch")); }
    let input_rows = names.iter().map(token_span).zip(parameters.iter().cloned()).collect();
    let mut formals = Formals::with_capacity(names.len());
    for (name, ty) in names.into_iter().zip(parameters) {
        if typecheck::is_null_literal_name(name.text()) || formals.insert(name.text().to_owned(), (token_span(&name), ty)).is_some() {
            return Err(diagnostic(source, token_span(&name), "duplicate or reserved runtime match parameter"));
        }
    }
    let matching = body.children().find(|node| node.kind() == SyntaxKind::ProofMatch)
        .ok_or_else(|| error("missing runtime match selector and arms"))?;
    let matching = AuthoredMatch::prepare(&matching).map_err(|error| diagnostic(source, error.span, error.message))?;
    let resolved = matching.resolve(source, enums, &|name, ty| formals.get(name).is_some_and(|(_, found)| ty == found), work)
        .map_err(|error| diagnostic(source, error.span, error.message))?;
    if resolved.enumeration.copy_modifier_span.is_none() || resolved.enumeration.has_tag_only_layout() {
        return Err(error("runtime enum payload match requires a payload-bearing enum copy declaration"));
    }
    let selector = typed_selector(source, &formals, resolved.selector)?;
    let arms = resolved.arms.into_iter().map(|arm| {
        let scope = arm.body.span;
        let arm_error = |message| diagnostic(source, scope, message);
        let [returned] = arm.body.final_output_values.as_slice() else {
            return Err(arm_error("runtime enum arm requires one scalar result"));
        };
        if arm.body.items.len() != 1 || arm.body.terminated_items.as_slice() != [false] {
            return Err(arm_error("runtime enum arm must directly return its payload or an integer literal"));
        }
        let bindings = arm.binders.iter().zip(arm.binder_spans).enumerate().map(|(slot, ((name, ty), declaration))| {
            if formals.contains_key(*name) || typecheck::is_null_literal_name(name) {
                return Err(diagnostic(source, declaration, "runtime enum payload binder must be fresh"));
            }
            Ok(CaseBinding { slot, declaration, ty: (*ty).clone() })
        }).collect::<Result<Vec<_>, _>>()?;
        let result = if let ScalarExpression::Name { name, span } = &returned.value {
            let Some(slot) = arm.binders.iter().position(|(binder, _)| *binder == name) else {
                return Err(diagnostic(source, *span, "runtime enum arm result must name its own payload binder"));
            };
            if bindings[slot].ty != result.ty {
                return Err(diagnostic(source, *span, "runtime enum payload result type differs from the function output"));
            }
            ArmResult::Payload { slot, use_span: *span }
        } else {
            let literal = concrete_integer_literal(&returned.value)
                .filter(|value| integer_literal_fits_type(value, &result.ty))
                .ok_or_else(|| diagnostic(source, returned.span, "runtime enum arm literal does not fit the function output"))?;
            ArmResult::Literal { value: literal.into_owned(), span: returned.span }
        };
        Ok(TypedArm { tag: arm.tag, scope, bindings, result })
    }).collect::<Result<_, _>>()?;
    let name_span = direct_token(&owner, SyntaxKind::Identifier).as_ref().map(token_span)
        .ok_or_else(|| error("missing runtime enum match callable name"))?;
    Ok(Plan { source: source.clone(), owner: wosy_syntax::byte_span(&owner), span, name_span, inputs: input_rows,
        enumeration: resolved.enumeration.id.clone(), output: result.ty.clone(), selector, arms })
}

impl Plan {
    pub(super) fn for_each_formal_read(&self, mut visit: impl FnMut(ByteSpan, ByteSpan)) {
        let mut pending = vec![&self.selector];
        while let Some(selector) = pending.pop() {
            match selector {
                TypedSelector::Constructor { payload, .. } => {
                    for (use_span, operand) in payload {
                        if let Operand::Formal(declaration) = operand { visit(*use_span, *declaration); }
                    }
                }
                TypedSelector::If { predicate, yes, no } => {
                    if let Predicate::Formal { use_span, declaration } = predicate { visit(*use_span, *declaration); }
                    pending.push(no);
                    pending.push(yes);
                }
            }
        }
    }

    pub(crate) fn authenticates(&self, source: &SourceIdentity, function: &ScalarFunction) -> bool {
        let ScalarType::Callable { parameters, outputs } = &function.signature else { return false; };
        &self.source == source && &self.enumeration.source == source && self.owner == function.span
            && self.name_span == function.name_span && self.span == function.body.span
            && function.body.items.is_empty() && function.body.final_output_values.is_empty()
            && function.overload_arms.is_empty() && function.generic_parameters.is_empty()
            && self.inputs.len() == function.parameter_spans.len()
            && self.inputs.len() == parameters.len()
            && self.inputs.iter().zip(function.parameter_spans.iter().zip(parameters))
                .all(|((declared, ty), (actual, actual_ty))| declared == actual && ty == actual_ty)
            && matches!(outputs.outputs.as_slice(), [output] if output.ty == self.output)
    }
}

fn typed_selector(source: &SourceIdentity, formals: &Formals, selector: Selector<'_>) -> Result<TypedSelector, Diagnostic> {
    match selector {
        Selector::Variant { tag, payload, operands, span } => {
            let payload = operands.into_iter().zip(payload).map(|(use_span, value)| {
                let value = match value {
                    Payload::Literal(value) => Operand::Literal(value),
                    Payload::Entry(name) => Operand::Formal(formals.get(&name)
                        .ok_or_else(|| diagnostic(source, use_span, "missing verified enum payload formal"))?.0),
                };
                Ok((use_span, value))
            }).collect::<Result<_, _>>()?;
            Ok(TypedSelector::Constructor { tag, span, payload })
        }
        Selector::If { condition, then_branch, else_branch } => {
            let predicate = match condition {
                ScalarExpression::Boolean { value, .. } => Predicate::Constant(*value),
                ScalarExpression::Name { name, span } if matches!(formals.get(name), Some((_, ScalarType::Bool))) => {
                    Predicate::Formal { use_span: *span, declaration: formals[name].0 }
                }
                _ => return Err(diagnostic(source, typecheck::expression_span(condition),
                    "runtime enum selector requires a Boolean literal or immutable Boolean formal")),
            };
            Ok(TypedSelector::If { predicate, yes: Box::new(typed_selector(source, formals, *then_branch)?),
                no: Box::new(typed_selector(source, formals, *else_branch)?) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_runtime_match_prepares_typed_positional_bindings_but_is_not_emittable() {
        let text = r#"%%start
            enum copy Region { left(u8); right(u8, u16); }
            u8(bool, u8) choose = fn(flag, input) {
                match (if (flag) { Region.left(input) } else { Region.right(9, 256) }) {
                    right(selected, other) { selected }
                    left(bound) { bound }
                }
            };
        %%end"#;
        let source = SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "test".into());
        let parse = |text: &str| {
            let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
        };
        let declarations = parse(text);
        assert!(declarations.items.is_empty());
        let [plan] = declarations.runtime_enum_matches.as_slice() else { panic!("{:?}", declarations.diagnostics); };
        assert_eq!(plan.enumeration.source, source);
        assert_eq!(plan.enumeration.index, 0);
        assert_eq!(plan.output, ScalarType::U8);
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        let TypedSelector::If { predicate: Predicate::Formal { use_span, declaration }, yes, no } = &plan.selector
            else { panic!("resolved runtime condition"); };
        assert_eq!(snippet(*use_span), "flag");
        assert_eq!(snippet(*declaration), "flag");
        assert_ne!(use_span, declaration);
        let TypedSelector::Constructor { tag: 0, span, payload } = yes.as_ref() else { panic!("left variant"); };
        assert_eq!(snippet(*span), "Region.left(input)");
        let [(usage, Operand::Formal(formal))] = payload.as_slice() else { panic!("one formal payload"); };
        assert_eq!(snippet(*usage), "input");
        assert_eq!(snippet(*formal), "input");
        assert_ne!(usage, formal);
        assert!(matches!(no.as_ref(), TypedSelector::Constructor { tag: 1, payload, .. } if payload.len() == 2));
        let [right, left] = plan.arms.as_slice() else { panic!("authored arm order"); };
        assert_eq!((right.tag, left.tag), (1, 0));
        assert_eq!(right.bindings[0].slot, 0);
        assert_eq!(right.bindings[1].ty, ScalarType::U16);
        assert_eq!(snippet(right.bindings[1].declaration), "other");
        assert!(matches!(right.result, ArmResult::Payload { slot: 0, .. }));
        assert!(declarations.diagnostics.iter().any(|diagnostic| diagnostic.message == "typed runtime enum match is not yet eligible for this preparation route"));
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            assert!(prepare_scalar_project(vec![(parse(text), Vec::new())], vec![source.clone()],
                ScalarPreparationInputs { target_layout }).is_err());
        }
        for (invalid, reason) in [
            (text.replace("enum copy", "enum"), "enum copy"),
            (text.replace("if (flag)", "if (input)"), "Boolean formal"),
            (text.replace("Region.right(9, 256)", "Region.right(256, 9)"), "does not fit"),
            (text.replace("right(selected, other)", "right(selected)"), "payload arity"),
            (text.replace("right(selected, other)", "right(selected, selected)"), "duplicate"),
            (text.replace("{ selected }", "{ other }"), "function output"),
            (text.replace("{ selected }", "{ bound }"), "its own payload binder"),
            (text.replace("left(bound) { bound }", ""), "cover every enum variant"),
        ] {
            let rejected = parse(&invalid);
            assert!(rejected.runtime_enum_matches.is_empty());
            assert!(rejected.diagnostics.iter().any(|diagnostic| diagnostic.message.contains(reason)), "{:?}", rejected.diagnostics);
        }
    }
}

#[cfg(test)]
#[test]
fn prepared_runtime_enum_match_uses_real_tagged_storage_on_native_and_wasi() {
    let text = r#"%%start
        enum copy Region { left(u8); right(u8, u16); }
        u8(bool, u8) choose = fn(flag, input) {
            match (if (flag) { Region.left(input) } else { Region.right(9, 256) }) {
                right(selected, other) { selected }
                left(bound) { bound }
            }
        };
        u8 x = choose(true, 17);
        u8 y = choose(false, 29);
        if (x != 17 || y != 9) { core.system_panic(); };
    %%end"#;
    let source = SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "test".into());
    let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let unused_text = format!("{}%%end", text.split_once("u8 x =").unwrap().0)
        .replace("u8(bool, u8) choose = fn(flag, input)", "u8(bool, u8, u8) choose = fn(flag, input, unused)");
    let unused = crate::parse_source(source.clone(), unused_text.clone(), &[]);
    assert!(unused.diagnostics.is_empty(), "{:?}", unused.diagnostics);
    let rejected = prepare_scalar_project(
        vec![(derive_scalar_build_declarations_from_cst(unused.result.canonical_cst()), Vec::new())],
        vec![source.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::NATIVE64 },
    ).err().expect("runtime enum Build still rejects unused formals");
    let [diagnostic] = rejected.diagnostics.as_slice() else { panic!("{:?}", rejected.diagnostics); };
    let span = diagnostic.labels[0].span.range;
    assert_eq!(diagnostic.code, "B0009");
    assert_eq!(&unused_text[span.start as usize..span.end as usize], "unused");
    let lib = SourceIdentity::new("p".into(), "pkg".into(), "src/lib.w".into(), "test".into());
    let owner_text = format!("{}%%end", text.split_once("u8 x =").unwrap().0);
    let owner = crate::parse_source(lib.clone(), owner_text, &[]);
    let consumer_text = r#"%%start
        library = namespace pkg "src/lib.w";
        u8 x = library.choose(true, 17);
        u8 y = library.choose(false, 29);
        if (x != 17 || y != 9) { core.system_panic(); };
    %%end"#;
    let consumer = crate::parse_source(source.clone(), consumer_text.to_owned(), &[]);
    assert!(owner.diagnostics.is_empty() && consumer.diagnostics.is_empty());
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout },
        ).unwrap();
        let analyzed = analyze_scalar_build(&prepared);
        assert!(analyzed.diagnostics().is_empty(), "{:?}", analyzed.diagnostics());
        for emit in [crate::emit_scalar_build_single_module_llvm_text, crate::emit_scalar_build_project_llvm_text] {
            let llvm = emit(&analyzed).unwrap();
            assert!(llvm.contains("enum.match.storage = alloca"));
            assert!(llvm.contains("switch i32"));
            assert!(llvm.contains("store i16 256"));
            assert!(llvm.contains("load i8"));
        }
        // Public raw serialization cannot install a source-only body seal.
        let encoded = serde_json::to_value(prepared.project()).unwrap();
        let decoded: ScalarProject = serde_json::from_value(encoded).unwrap();
        let raw = validate_scalar_project(decoded);
        assert!(!raw.diagnostics.is_empty());
        let prepared = prepare_scalar_project(
            vec![
                (derive_scalar_build_declarations_from_cst(consumer.result.canonical_cst()), vec![
                    ScalarNamespaceBinding { binding: "library".to_owned(), target: lib.clone(), span: ByteSpan::new(0, 0) },
                ]),
                (derive_scalar_build_declarations_from_cst(owner.result.canonical_cst()), Vec::new()),
            ], vec![source.clone(), lib.clone()], ScalarPreparationInputs { target_layout },
        ).unwrap();
        let analyzed = analyze_scalar_build(&prepared);
        assert!(analyzed.diagnostics().is_empty(), "{:?}", analyzed.diagnostics());
        let llvm = crate::emit_scalar_build_project_llvm_text(&analyzed).unwrap();
        assert!(llvm.contains("enum.match.storage = alloca") && llvm.contains("switch i32"));
    }
}
