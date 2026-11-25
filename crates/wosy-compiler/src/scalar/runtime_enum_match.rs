//! Guarded source preparation for the ordinary runtime enum match. This sealed body carries only checked, scope-limited scalar values.
use super::{proof_match::{AuthoredMatch, RuntimeSelector},
    typecheck::{concrete_integer_literal, integer_literal_fits_type},
    *};

mod primitive;
pub(crate) use primitive::{IntegerExpression, IntegerKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) source: SourceIdentity,
    pub(crate) owner: ByteSpan,
    pub(crate) span: ByteSpan,
    pub(crate) enumeration: ScalarEnumId,
    name_span: ByteSpan,
    inputs: Vec<(ByteSpan, ScalarType)>,
    initializer: bool,
    input_names: Vec<String>,
    pub(crate) outputs: Vec<ScalarType>,
    pub(crate) selector: TypedSelector,
    pub(crate) local: Option<LexicalEnumBinding>,
    pub(crate) arms: Vec<TypedArm>,
}

/// Opaque source-prepared value occurrence; raw/serialized IR has no constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence(pub(crate) std::sync::Arc<Plan>);

impl Occurrence {
    pub(crate) fn for_each_input_name(&self, mut visit: impl FnMut(&str)) {
        self.0.for_each_formal_read(|_, declaration| {
            if let Ok(index) = self.0.inputs.binary_search_by_key(&declaration.start, |(span, _)| span.start) {
                visit(&self.0.input_names[index]);
            }
        });
    }

    pub(crate) fn ty(&self) -> &ScalarType { &self.0.outputs[0] }
    pub(crate) fn span(&self) -> ByteSpan { self.0.span }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LexicalEnumBinding {
    enumeration: ScalarEnumId,
    declaration: ByteSpan,
    initializer: ByteSpan,
    read: ByteSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TypedSelector {
    Constructor { tag: u32, span: ByteSpan, payload: Vec<(ByteSpan, Operand)> },
    If { predicate: Predicate, yes: Box<Self>, no: Box<Self> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Operand { Literal(BigInt), Formal(ByteSpan), Expression(IntegerExpression) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Predicate {
    Constant(bool),
    Formal { use_span: ByteSpan, declaration: ByteSpan },
    Compare { operator: BinaryOperator, operands: Box<[IntegerExpression; 2]> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypedArm {
    pub(crate) tag: u32,
    pub(crate) scope: ByteSpan,
    pub(crate) bindings: Vec<CaseBinding>,
    pub(crate) results: Vec<ArmResult>,
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
    Expression(IntegerExpression),
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
    let initializer = body.kind() == SyntaxKind::RuntimeMatchExpr;
    let (owner, initializer_type) = if initializer {
        let parent = |node: &CstNode, kind| node.parent().filter(|node| node.kind() == kind)
            .ok_or_else(|| error("runtime match expression requires one scalar local initializer"));
        let expression = parent(body, SyntaxKind::Expression)?;
        let outputs = parent(&expression, SyntaxKind::OutputList)?;
        let binding = parent(&outputs, SyntaxKind::LocalBinding)?;
        let item = parent(&binding, SyntaxKind::BlockItem)?;
        let block = parent(&item, SyntaxKind::Block)?;
        let owner = parent(&block, SyntaxKind::FunctionDecl)?;
        if outputs.children().count() != 1 || block.children().next().as_ref() != Some(&item) {
            return Err(error("runtime match initializer requires the first ordinary function local and one output"));
        }
        let receivers = binding.children().find(|node| node.kind() == SyntaxKind::ReceiverList)
            .ok_or_else(|| error("missing runtime match initializer receiver"))?;
        let mut receivers = receivers.children().filter(|node| node.kind() == SyntaxKind::Receiver);
        let receiver = receivers.next().ok_or_else(|| error("missing runtime match initializer receiver"))?;
        if receivers.next().is_some() || direct_token(&receiver, SyntaxKind::Identifier).is_none() {
            return Err(error("runtime match initializer requires exactly one lexical receiver"));
        }
        let ty = receiver.children().find(|node| node.kind() == SyntaxKind::TypeSpec)
            .ok_or_else(|| error("missing runtime match initializer receiver type"))?;
        (owner, Some(derive_type(&ty)))
    } else {
        let owner = body.parent().filter(|owner| owner.kind() == SyntaxKind::FunctionDecl)
            .ok_or_else(|| error("runtime match must be the sole result of a named function"))?;
        (owner, None)
    };
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
    let output_types = if let Some(ty) = initializer_type {
        vec![ty]
    } else {
        outputs.outputs.into_iter().map(|output| output.ty).collect()
    };
    if output_types.is_empty() || !output_types.iter().all(is_integer_type)
        || !parameters.iter().all(|ty| is_integer_type(ty) || *ty == ScalarType::Bool)
    {
        return Err(error("runtime enum match requires scalar integer results and primitive formals"));
    }
    analysis_budget::charge_call_work(work, output_types.len()).ok_or_else(||
        error("runtime enum output sequence exceeds analysis limits"))?;
    let names = owner.children().find(|node| node.kind() == SyntaxKind::Parameters).into_iter()
        .flat_map(|parameters| parameters.children_with_tokens()).filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Identifier).collect::<Vec<_>>();
    if names.len() != parameters.len() { return Err(error("runtime match parameter arity mismatch")); }
    let input_rows = names.iter().map(token_span).zip(parameters.iter().cloned()).collect();
    let input_names = if initializer {
        analysis_budget::charge_call_work(work, names.len()).ok_or_else(|| error("runtime enum formals exceed analysis limits"))?;
        names.iter().map(|name| name.text().to_owned()).collect()
    } else { Vec::new() };
    let mut formals = Formals::with_capacity(names.len());
    for (name, ty) in names.into_iter().zip(parameters) {
        if typecheck::is_null_literal_name(name.text()) || formals.insert(name.text().to_owned(), (token_span(&name), ty)).is_some() {
            return Err(diagnostic(source, token_span(&name), "duplicate or reserved runtime match parameter"));
        }
    }
    let (matching, local) = if initializer {
        (body.children().find(|node| node.kind() == SyntaxKind::ProofMatch)
            .ok_or_else(|| error("missing runtime match selector and arms"))?, None)
    } else {
        let mut parts = body.children();
        let first = parts.next().ok_or_else(|| error("missing runtime match selector and arms"))?;
        if first.kind() == SyntaxKind::ProofMatch && parts.next().is_none() {
            (first, None)
        } else if first.kind() == SyntaxKind::BlockItem {
            let matching = parts.next().filter(|node| node.kind() == SyntaxKind::ProofMatch)
                .ok_or_else(|| error("runtime enum match allows only one preceding immutable enum local"))?;
            if parts.next().is_some() {
                return Err(error("runtime enum match must be the final result"));
            }
            let declaration = first.children().find(|part| part.kind() == SyntaxKind::LocalBinding)
                .ok_or_else(|| error("runtime enum match allows only a preceding immutable enum local"))?;
            let receivers = declaration.children().find(|part| part.kind() == SyntaxKind::ReceiverList)
                .ok_or_else(|| error("runtime enum match requires an initialized enum local"))?;
            let mut receivers = receivers.children().filter(|part| part.kind() == SyntaxKind::Receiver);
            let receiver = receivers.next().ok_or_else(|| error("missing runtime enum local receiver"))?;
            if receivers.next().is_some() {
                return Err(error("runtime enum match requires exactly one enum receiver"));
            }
            let ty = receiver.children().find(|part| part.kind() == SyntaxKind::TypeSpec)
                .ok_or_else(|| error("missing runtime enum local type"))?;
            let mut semantic = semantic_children(&ty);
            let Some(ty_name) = semantic.next().and_then(|element| element.into_token())
                .filter(|token| token.kind() == SyntaxKind::TypeName) else {
                return Err(diagnostic(source, wosy_syntax::byte_span(&ty), "runtime enum local needs a simple same-source enum type"));
            };
            if semantic.next().is_some() {
                return Err(diagnostic(source, wosy_syntax::byte_span(&ty), "runtime enum local needs a simple same-source enum type"));
            }
            let name = direct_token(&receiver, SyntaxKind::Identifier)
                .ok_or_else(|| error("missing runtime enum local name"))?;
            if formals.contains_key(name.text()) || typecheck::is_null_literal_name(name.text()) {
                return Err(diagnostic(source, token_span(&name), "runtime enum local must be fresh"));
            }
            let output = declaration.children().find(|part| part.kind() == SyntaxKind::OutputList)
                .ok_or_else(|| error("missing runtime enum local initializer"))?;
            let mut outputs = output.children().filter(|part| part.kind() == SyntaxKind::Expression);
            let initializer = outputs.next().ok_or_else(|| error("missing runtime enum local initializer"))?;
            if outputs.next().is_some() {
                return Err(error("runtime enum match requires exactly one enum initializer"));
            }
            (matching, Some((name, ty_name, initializer)))
        } else {
            return Err(error("runtime enum match allows only one preceding immutable enum local"));
        }
    };
    let matching = AuthoredMatch::prepare_with_initializer(&matching, local.as_ref().map(|(_, _, node)| node.clone()))
        .map_err(|error| diagnostic(source, error.span, error.message))?;
    let resolved = matching.resolve_runtime(source, enums, work)
        .map_err(|error| diagnostic(source, error.span, error.message))?;
    if resolved.enumeration.copy_modifier_span.is_none() || resolved.enumeration.has_tag_only_layout() {
        return Err(error("runtime enum payload match requires a payload-bearing enum copy declaration"));
    }
    let local_binding = local.as_ref().map(|(name, ty, initializer)| {
        let type_span = token_span(ty);
        if ty.text() != resolved.enumeration.name.as_str() {
            return Err(diagnostic(source, type_span, "runtime enum local type does not match its initializer enum"));
        }
        let use_span = match matching.lexical_selector() {
            Some(ScalarExpression::Name { name: used, span }) if used == name.text() => *span,
            Some(selector) => return Err(diagnostic(source, typecheck::expression_span(selector),
                "runtime match must read the preceding enum local")),
            None => return Err(error("missing preceding runtime enum local read")),
        };
        Ok(LexicalEnumBinding { enumeration: resolved.enumeration.id.clone(), declaration: token_span(name),
            initializer: wosy_syntax::byte_span(initializer), read: use_span })
    }).transpose()?;
    let selector = typed_selector(source, &formals, resolved.selector, work)?;
    let arms = resolved.arms.into_iter().map(|arm| {
        let scope = arm.body.span;
        let arm_error = |message| diagnostic(source, scope, message);
        let returned = &arm.body.final_output_values;
        if returned.len() != output_types.len() {
            return Err(arm_error("runtime enum arms must match the function output arity"));
        }
        if arm.body.items.len() != returned.len()
            || arm.body.terminated_items.len() != returned.len()
            || arm.body.terminated_items.iter().any(|terminated| *terminated)
        {
            return Err(arm_error("runtime enum arms must return a direct primitive output sequence"));
        }
        analysis_budget::charge_call_work(work, returned.len()).ok_or_else(||
            arm_error("runtime enum output sequence exceeds analysis limits"))?;
        let bindings = arm.binders.iter().zip(arm.binder_spans).enumerate().map(|(slot, ((name, ty), declaration))| {
            if formals.contains_key(*name) || typecheck::is_null_literal_name(name)
                || local.as_ref().is_some_and(|(binding, ..)| binding.text() == *name) {
                return Err(diagnostic(source, declaration, "runtime enum payload binder must be fresh"));
            }
            Ok(CaseBinding { slot, declaration, ty: (*ty).clone() })
        }).collect::<Result<Vec<_>, _>>()?;
        let slots = arm.binders.iter().enumerate().map(|(slot, (name, _))| (*name, slot))
            .collect::<HashMap<_, _>>();
        let names = |name: &str| slots.get(name).map(|&slot| (bindings[slot].ty.clone(), IntegerKind::Payload(slot)));
        let results = returned.iter().zip(&output_types).map(|(returned, result)| {
            if let ScalarExpression::Name { name, span } = &returned.value {
                let Some(&slot) = slots.get(name.as_str()) else {
                    return Err(diagnostic(source, *span, "runtime enum arm result must name its own payload binder"));
                };
                if bindings[slot].ty != *result {
                    return Err(diagnostic(source, *span, "runtime enum payload result type differs from the function output"));
                }
                Ok(ArmResult::Payload { slot, use_span: *span })
            } else if let Some(literal) = concrete_integer_literal(&returned.value) {
                if !integer_literal_fits_type(&literal, result) {
                    return Err(diagnostic(source, returned.span, "runtime enum arm literal does not fit the function output"));
                }
                Ok(ArmResult::Literal { value: literal.into_owned(), span: returned.span })
            } else {
                Ok(ArmResult::Expression(primitive::compile(source, &returned.value, result, &names, work, 0)?))
            }
        }).collect::<Result<_, _>>()?;
        Ok(TypedArm { tag: arm.tag, scope, bindings, results })
    }).collect::<Result<_, _>>()?;
    let name_span = direct_token(&owner, SyntaxKind::Identifier).as_ref().map(token_span)
        .ok_or_else(|| error("missing runtime enum match callable name"))?;
    Ok(Plan { source: source.clone(), owner: wosy_syntax::byte_span(&owner), span, name_span, inputs: input_rows, initializer, input_names,
        enumeration: resolved.enumeration.id.clone(), outputs: output_types, selector, local: local_binding, arms })
}

impl Plan {
    pub(super) fn for_each_formal_read(&self, mut visit: impl FnMut(ByteSpan, ByteSpan)) {
        let mut pending = vec![&self.selector];
        while let Some(selector) = pending.pop() {
            match selector {
                TypedSelector::Constructor { payload, .. } => {
                    for (use_span, operand) in payload {
                        match operand {
                            Operand::Formal(declaration) => visit(*use_span, *declaration),
                            Operand::Expression(expression) => expression.for_each_formal(&mut visit),
                            Operand::Literal(_) => {}
                        }
                    }
                }
                TypedSelector::If { predicate, yes, no } => {
                    match predicate {
                        Predicate::Formal { use_span, declaration } => visit(*use_span, *declaration),
                        Predicate::Compare { operands, .. } => {
                            operands[0].for_each_formal(&mut visit);
                            operands[1].for_each_formal(&mut visit);
                        }
                        Predicate::Constant(_) => {}
                    }
                    pending.push(no);
                    pending.push(yes);
                }
            }
        }
    }

    pub(super) fn is_initializer(&self) -> bool { self.initializer }

    pub(crate) fn input_spans(&self) -> impl Iterator<Item = ByteSpan> + '_ {
        self.inputs.iter().map(|(span, _)| *span)
    }

    pub(super) fn authenticates_initializer(&self, source: &SourceIdentity, function: &ScalarFunction,
        binding: &ScalarBinding,
    ) -> bool {
        let ScalarType::Callable { parameters, .. } = &function.signature else { return false; };
        self.initializer && &self.source == source && &self.enumeration.source == source
            && self.owner == function.span && self.name_span == function.name_span
            && function.overload_arms.is_empty() && function.generic_parameters.is_empty()
            && self.local.is_none() && !binding.is_allocation && binding.contextual_storage().is_none()
            && binding.receivers.len() == 1 && binding.output_values.len() == 1
            && binding.span.start <= self.span.start && self.span.end <= binding.span.end
            && self.outputs.as_slice() == std::slice::from_ref(&binding.declared_type)
            && self.inputs.len() == function.parameter_spans.len() && self.inputs.len() == parameters.len()
            && self.inputs.iter().zip(function.parameter_spans.iter().zip(parameters))
                .all(|((declared, ty), (actual, actual_ty))| declared == actual && ty == actual_ty)
            && self.input_names == function.parameters
    }

    pub(crate) fn authenticates(&self, source: &SourceIdentity, function: &ScalarFunction) -> bool {
        let ScalarType::Callable { parameters, outputs } = &function.signature else { return false; };
        !self.initializer && &self.source == source && &self.enumeration.source == source && self.owner == function.span
            && self.name_span == function.name_span && self.span == function.body.span
            && function.body.items.is_empty() && function.body.final_output_values.is_empty()
            && function.overload_arms.is_empty() && function.generic_parameters.is_empty()
            && self.local.as_ref().is_none_or(|local| local.enumeration == self.enumeration
                && self.span.start <= local.declaration.start && local.declaration.end <= local.initializer.start
                && local.initializer.end <= local.read.start && local.read.end <= self.span.end)
            && self.inputs.len() == function.parameter_spans.len()
            && self.inputs.len() == parameters.len()
            && self.inputs.iter().zip(function.parameter_spans.iter().zip(parameters))
                .all(|((declared, ty), (actual, actual_ty))| declared == actual && ty == actual_ty)
            && outputs.outputs.iter().map(|output| &output.ty).eq(self.outputs.iter())
    }
}

fn typed_selector(source: &SourceIdentity, formals: &Formals, selector: RuntimeSelector<'_>,
    work: &mut usize,
) -> Result<TypedSelector, Diagnostic> {
    let names = |name: &str| formals.get(name).map(|(span, ty)| (ty.clone(), IntegerKind::Formal(*span)));
    match selector {
        RuntimeSelector::Variant { tag, payload: arguments, span } => {
            let payload = arguments.into_iter().map(|(argument, ty)| {
                let typed = primitive::compile(source, argument, ty, &names, work, 0)?;
                let span = typed.span;
                let operand = match typed.kind {
                    IntegerKind::Literal(value) => Operand::Literal(value),
                    IntegerKind::Formal(declaration) => Operand::Formal(declaration),
                    _ => Operand::Expression(typed),
                };
                Ok((span, operand))
            }).collect::<Result<_, _>>()?;
            Ok(TypedSelector::Constructor { tag, span, payload })
        }
        RuntimeSelector::If { condition, then_branch, else_branch } => {
            let predicate = match condition {
                ScalarExpression::Boolean { value, .. } => Predicate::Constant(*value),
                ScalarExpression::Name { name, span } if matches!(formals.get(name), Some((_, ScalarType::Bool))) => {
                    Predicate::Formal { use_span: *span, declaration: formals[name].0 }
                }
                ScalarExpression::Binary { operator, left, right, span }
                    if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual | BinaryOperator::Less
                        | BinaryOperator::LessEqual | BinaryOperator::Greater | BinaryOperator::GreaterEqual) => {
                    let ty = primitive::comparison_type(left, right, formals, work).ok_or_else(||
                        diagnostic(source, *span, "runtime enum comparison needs a typed integer formal"))?;
                    let left = primitive::compile(source, left, &ty, &names, work, 0)?;
                    let right = primitive::compile(source, right, &ty, &names, work, 0)?;
                    Predicate::Compare { operator: operator.clone(), operands: Box::new([left, right]) }
                }
                _ => return Err(diagnostic(source, typecheck::expression_span(condition),
                    "runtime enum selector requires a Boolean literal, immutable Boolean formal, or typed integer comparison")),
            };
            Ok(TypedSelector::If { predicate, yes: Box::new(typed_selector(source, formals, *then_branch, work)?),
                no: Box::new(typed_selector(source, formals, *else_branch, work)?) })
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
        assert_eq!(plan.outputs, [ScalarType::U8]);
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
        assert!(matches!(right.results.as_slice(), [ArmResult::Payload { slot: 0, .. }]));
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

#[cfg(test)]
#[test]
fn prepared_runtime_enum_lexical_binding_is_the_sole_match_input() {
    let text = r#"%%start
        enum copy Region { left(u8); right(u8, u16); }
        enum copy Other { left(u8); right(u8, u16); }
        u8(bool, u8) choose = fn(flag, input) {
            Region selected = if (flag) { Region.left(input) } else { Region.right(9, 256) };
            match (selected) {
                right(value, other) { value }
                left(bound) { bound }
            }
        };
        u8 x = choose(true, 17);
        u8 y = choose(false, 29);
        if (x != 17 || y != 9) { core.system_panic(); };
    %%end"#;
    let source = SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "test".into());
    let prepare = |text: &str, target_layout| {
        let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        prepare_scalar_project(
            vec![(derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout },
        )
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare(text, target_layout).unwrap();
        let analyzed = analyze_scalar_build(&prepared);
        assert!(analyzed.diagnostics().is_empty(), "{:?}", analyzed.diagnostics());
        let ScalarItem::Function(function) = &prepared.project().modules[0].items[0] else {
            panic!("prepared lexical runtime match owner");
        };
        let plan = function.runtime_enum_match.as_deref().unwrap();
        let local = plan.local.as_ref().unwrap();
        let at = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(at(local.declaration), "selected");
        assert_eq!(at(local.read), "selected");
        assert_ne!(local.declaration, local.read);
        assert_eq!(local.enumeration, plan.enumeration);
        assert!(at(local.initializer).starts_with("if (flag)"));
        for emit in [crate::emit_scalar_build_single_module_llvm_text, crate::emit_scalar_build_project_llvm_text] {
            let llvm = emit(&analyzed).unwrap();
            assert_eq!(llvm.matches("enum.local.storage = alloca").count(), 1);
            assert!(llvm.contains("switch i32") && llvm.contains("store i16 256"));
        }
    }
    for (invalid, reason) in [
        (text.replace("Region selected = ", "Other selected = "), "does not match its initializer"),
        (text.replace("Region selected = ", "Region[1] selected = "), "simple same-source enum type"),
        (text.replace("match (selected)", "match (input)"), "must read the preceding"),
        (text.replace("Region selected", "Region input").replace("match (selected)", "match (input)"), "must be fresh"),
        (text.replace("left(bound) { bound }", "left(selected) { selected }"), "binder must be fresh"),
    ] {
        let rejected = prepare(&invalid, ScalarTargetLayout::NATIVE64).err().expect("invalid runtime enum binding");
        assert!(rejected.diagnostics.iter().any(|diagnostic| diagnostic.message.contains(reason)), "{:?}", rejected.diagnostics);
    }
}

#[cfg(test)]
#[test]
fn prepared_runtime_enum_typed_primitive_regions_share_ordinary_integer_lowering() {
    let text = r#"%%start
        enum copy Region { negative(i8, i8); positive(i8); }
        i8(i8) choose = fn(input) {
            Region chosen = if (input < 0) { Region.negative(input +@ -1, 3) } else { Region.positive(input | 2) };
            match (chosen) {
                negative(value, delta) { value +@ delta }
                positive(value) { value ^ 1 }
            }
        };
        i8 negative = choose(-128);
        i8 positive = choose(0);
        i8 expected_negative = -126;
        if (negative != expected_negative || positive != 3) { core.system_panic(); };
    %%end"#;
    let source = SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "test".into());
    let prepare = |text: &str, target_layout| {
        let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        prepare_scalar_project(
            vec![(derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout },
        )
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare(text, target).unwrap();
        let analyzed = analyze_scalar_build(&prepared);
        for emit in [crate::emit_scalar_build_single_module_llvm_text, crate::emit_scalar_build_project_llvm_text] {
            let ir = emit(&analyzed).unwrap();
            assert!(ir.contains("icmp slt i8"));
            assert!(ir.contains("add i8") && ir.contains("or i8") && ir.contains("xor i8"));
            assert!(ir.contains("switch i32") && ir.contains("enum.local.storage"));
        }
    }
    for invalid in [
        text.replace("input +@ -1", "input + -1"),
        text.replace("value +@ delta", "value +@ input"),
        text.replace("input | 2", "input / 2"),
        text.replace("input | 2", "input | 128"),
        text.replace("negative(i8, i8)", "negative(i8, u8)"),
    ] {
        let rejected = prepare(&invalid, ScalarTargetLayout::NATIVE64).err().expect("invalid runtime primitive expression");
        assert!(rejected.diagnostics.iter().any(|diagnostic| diagnostic.code == "B0003"), "{:?}", rejected.diagnostics);
    }
}

#[cfg(test)]
#[test]
fn prepared_runtime_enum_match_returns_authentic_positional_output_sequences() {
    let text = r#"%%start
        enum copy Region { left(u8, u16); right(u16, u8); }
        (u8, u16)(bool, u8) choose = fn(flag, input) {
            Region selected = if (flag) { Region.left(input, 256) } else { Region.right(513, input | 2) };
            match (selected) {
                right(wide, narrow) { narrow ^ 1, wide }
                left(narrow, wide) { narrow, wide }
            }
        };
        u8 a, u16 b = choose(true, 17);
        u8 c, u16 d = choose(false, 24);
        if (a != 17 || b != 256 || c != 27 || d != 513) { core.system_panic(); };
    %%end"#;
    let source = SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "test".into());
    let prepare = |text: &str, target_layout| {
        let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        prepare_scalar_project(
            vec![(derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout },
        )
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare(text, target).unwrap();
        let ScalarItem::Function(function) = &prepared.project().modules[0].items[0] else { panic!("function owner"); };
        let plan = function.runtime_enum_match.as_deref().unwrap();
        assert_eq!(plan.outputs, [ScalarType::U8, ScalarType::U16]);
        assert!(matches!(plan.arms[0].results.as_slice(), [ArmResult::Expression(_), ArmResult::Payload { slot: 0, .. }]));
        let analyzed = analyze_scalar_build(&prepared);
        for emit in [crate::emit_scalar_build_single_module_llvm_text, crate::emit_scalar_build_project_llvm_text] {
            let ir = emit(&analyzed).unwrap();
            assert!(ir.contains("switch i32") && ir.contains("insertvalue { i8, i16 }") && ir.contains("ret { i8, i16 }"));
            assert!(ir.contains("extractvalue { i8, i16 }"));
            assert!(ir.contains("enum.match.continue:"));
            assert!(ir.contains("phi i8") && ir.contains("phi i16"));
            assert_eq!(ir.matches("ret { i8, i16 }").count(), 1);
        }
    }
    for (invalid, reason) in [
        (text.replace("{ narrow, wide }", "{ narrow }"), "output arity"),
        (text.replace("{ narrow, wide }", "{ wide, narrow }"), "differs from the function output"),
        (text.replace("{ narrow, wide }", "{ narrow, wide, 1 }"), "output arity"),
        (text.replace("(u8, u16)(bool", "(u8, *u16)(bool"), "scalar integer results"),
    ] {
        let rejected = prepare(&invalid, ScalarTargetLayout::NATIVE64).err().expect("invalid runtime enum output sequence");
        assert!(rejected.diagnostics.iter().any(|diagnostic| diagnostic.message.contains(reason)), "{:?}", rejected.diagnostics);
    }
}

#[test]
fn prepared_runtime_enum_match_as_ordinary_lexical_initializer() {
    let text = r#"%%start
        enum copy Region { left(u8, u16); right(u16, u8); }
        (u16)(bool, u8) choose = fn(flag, input) {
            u16 selected = match (if (flag) { Region.left(input, 256) } else { Region.right(513, input | 2) }) {
                right(wide, narrow) { wide }
                left(narrow, wide) { wide ^ 1 }
            };
            u16 transformed = selected +@ 2;
            transformed
        };
        u16 a = choose(true, 17);
        u16 b = choose(false, 24);
        if (a != 259 || b != 515) { core.system_panic(); };
    %%end"#;
    let source = SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "test".into());
    let parse = |text: &str| crate::parse_source(source.clone(), text.to_owned(), &[]);
    let parsed = parse(text);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);

    let non_build = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(non_build.diagnostics.iter().any(|diagnostic| diagnostic.message.contains("not yet eligible")),
        "{:?}", non_build.diagnostics);

    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst());
        assert!(declarations.diagnostics.is_empty(), "{:?}", declarations.diagnostics);
        let prepared = prepare_scalar_project(
            vec![(declarations, Vec::new())], vec![source.clone()], ScalarPreparationInputs { target_layout },
        ).unwrap();
        let ScalarItem::Function(function) = &prepared.project().modules[0].items[0] else { panic!("function owner"); };
        assert!(function.runtime_enum_match.is_none(), "the ordinary callable path must own the initializer");
        let [ScalarBlockItem::LocalBinding(binding), ScalarBlockItem::LocalBinding(_), ScalarBlockItem::Expression(_)] =
            &function.body.items[..] else { panic!("ordinary binding/continuation"); };
        let ScalarExpression::RuntimeMatch(site) = &binding.value else { panic!("first-class scalar match"); };
        assert_eq!(site.ty(), &ScalarType::U16);
        assert!(serde_json::to_string(&binding.value).is_err(), "runtime authority is not wire data");
        let analyzed = analyze_scalar_build(&prepared);
        assert!(analyzed.diagnostics().is_empty(), "{:?}", analyzed.diagnostics());
        let cfg = analyzed.reference_cfg(0, 0, None).expect("ordinary callable CFG");
        let point = cfg.points.iter().find(|point| matches!(&point.kind,
            CfgPointKind::Evaluate(ScalarExpression::RuntimeMatch(occurrence)) if occurrence == site))
            .expect("typed initializer evaluates in the original CFG");
        assert!(cfg.points.iter().any(|successor| point.successors.contains(&successor.id)
            && matches!(successor.kind, CfgPointKind::Bind { .. })));
        for emit in [crate::emit_scalar_build_single_module_llvm_text, crate::emit_scalar_build_project_llvm_text] {
            let ir = emit(&analyzed).unwrap();
            assert!(ir.contains("switch i32") && ir.contains("phi i16"));
            let continuation = ir.split("enum.match.continue:").nth(1).expect("ordinary continuation");
            assert!(continuation.contains("store i16") && continuation.contains("add i16"));
        }
    }

    for (invalid, reason) in [
        (text.replace("u16 selected =", "u8 selected ="), "payload result type differs"),
        (text.replace("left(narrow, wide) { wide ^ 1 }", "right(narrow, wide) { wide ^ 1 }"), "duplicate or unknown"),
        (text.replace("u16 selected =", "u8 previous = 1; u16 selected ="), "requires the first ordinary function local"),
        (text.replace("wide ^ 1", "wide / 1"), "total integer operator"),
    ] {
        let parsed = parse(&invalid);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_build_declarations_from_cst(parsed.result.canonical_cst());
        assert!(declarations.diagnostics.iter().any(|diagnostic| diagnostic.message.contains(reason)),
            "{:?}", declarations.diagnostics);
    }
}
