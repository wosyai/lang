//! Guarded source preparation for the ordinary runtime enum match, not a
//! runtime evaluator. There is no conversion of this plan into emitted code.
use super::{proof_match::{AuthoredMatch, Payload, Selector},
    typecheck::{concrete_integer_literal, integer_literal_fits_type},
    *};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Plan {
    pub(super) source: SourceIdentity,
    pub(super) owner: ByteSpan,
    pub(super) span: ByteSpan,
    pub(super) enumeration: ScalarEnumId,
    pub(super) output: ScalarType,
    pub(super) selector: TypedSelector,
    pub(super) arms: Vec<TypedArm>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TypedSelector {
    Constructor { tag: u32, span: ByteSpan, payload: Vec<(ByteSpan, Operand)> },
    If { predicate: Predicate, yes: Box<Self>, no: Box<Self> },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Operand { Literal(BigInt), Formal(ByteSpan) }

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Predicate { Constant(bool), Formal { use_span: ByteSpan, declaration: ByteSpan } }

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TypedArm {
    pub(super) tag: u32,
    pub(super) scope: ByteSpan,
    pub(super) bindings: Vec<CaseBinding>,
    pub(super) result: ArmResult,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CaseBinding {
    pub(super) slot: usize,
    pub(super) declaration: ByteSpan,
    pub(super) ty: ScalarType,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ArmResult {
    Literal { value: BigInt, span: ByteSpan },
    Payload { slot: usize, use_span: ByteSpan },
}

pub(super) fn diagnostic(source: &SourceIdentity, span: ByteSpan, message: &str) -> Diagnostic {
    Diagnostic { code: "B0003".to_owned(), severity: DiagnosticSeverity::Error,
        message: message.to_owned(), labels: vec![DiagnosticLabel { kind: DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span), message: message.to_owned() }], notes: Vec::new() }
}

type Formals = HashMap<String, (ByteSpan, ScalarType)>;

pub(super) fn prepare(source: &SourceIdentity, enums: &[ScalarEnum], body: &CstNode, work: &mut usize) -> Result<Plan, Diagnostic> {
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
    Ok(Plan { source: source.clone(), owner: wosy_syntax::byte_span(&owner), span,
        enumeration: resolved.enumeration.id.clone(), output: result.ty.clone(), selector, arms })
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
            derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst())
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
        assert!(declarations.diagnostics.iter().any(|diagnostic| diagnostic.message == "typed runtime enum match is not yet eligible for lowering"));
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
