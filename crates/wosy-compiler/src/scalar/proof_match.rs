//! Canonical authored variant matching shared by Build static typing and
//! the ordered checker. This module does not inspect values or prove goals.
use super::{analysis_budget::{charge_call_work, MAX_ACTUAL_CALL_DEPTH}, typecheck::{concrete_integer_literal, expression_span, integer_literal_fits_type, is_integer_type}, *};

pub(super) struct AuthoredMatch {
    selector: ScalarExpression,
    arms: Vec<Arm>,
    span: ByteSpan,
}

struct Arm {
    variant: String,
    body: ScalarBlock,
    binder: Option<String>,
    span: ByteSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Payload {
    Literal(BigInt),
    Entry(String),
}

pub(super) struct MatchArm<'a> {
    #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
    pub(super) tag: u32,
    pub(super) body: &'a ScalarBlock,
    pub(super) binder: Option<(&'a str, &'a ScalarType)>,
}

pub(super) enum Selector<'a> {
    Variant {
        #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
        tag: u32,
        #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
        payload: Option<Payload>,
    },
    If { condition: &'a ScalarExpression, then_branch: Box<Selector<'a>>, else_branch: Box<Selector<'a>> },
}

pub(super) struct ResolvedMatch<'a> {
    pub(super) selector: Selector<'a>,
    pub(super) arms: Vec<MatchArm<'a>>,
}

pub(super) struct MatchError {
    pub(super) span: ByteSpan,
    pub(super) message: &'static str,
    #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
    pub(super) invalid: bool,
}

impl MatchError {
    fn invalid(span: ByteSpan, message: &'static str) -> Self { Self { span, message, invalid: true } }
    fn unsupported(span: ByteSpan, message: &'static str) -> Self { Self { span, message, invalid: false } }
    fn budget(span: ByteSpan) -> Self { Self::unsupported(span, "authored match exceeds static analysis limits") }
}

fn charge(work: &mut usize, cost: usize, span: ByteSpan) -> Result<(), MatchError> {
    charge_call_work(work, cost).ok_or_else(|| MatchError::budget(span))
}

impl AuthoredMatch {
    pub(super) fn prepare(node: &CstNode) -> Result<Self, MatchError> {
        let span = wosy_syntax::byte_span(node);
        let mut work = 0;
        charge(&mut work, u32::from(node.text_range().len()) as usize / 32 + 1, span)?;
        let mut pending = vec![(node.clone(), 1)];
        while let Some((node, depth)) = pending.pop() {
            if depth > MAX_ACTUAL_CALL_DEPTH { return Err(MatchError::budget(span)); }
            for child in node.children() {
                charge(&mut work, 1, span)?;
                pending.push((child, depth + 1));
            }
        }
        let child = |node: &CstNode, kind| node.children().find(|child| child.kind() == kind)
            .ok_or_else(|| MatchError::invalid(wosy_syntax::byte_span(node), "incomplete authored match"));
        let selector = derive_expression(&child(node, SyntaxKind::Expression)?);
        let mut arms = Vec::new();
        for node in node.children().filter(|node| node.kind() == SyntaxKind::ProofMatchArm) {
            let span = wosy_syntax::byte_span(&node);
            let binders = node.children().find(|node| node.kind() == SyntaxKind::Parameters)
                .into_iter().flat_map(|parameters| parameters.children_with_tokens())
                .filter_map(|element| element.into_token())
                .filter(|token| token.kind() == SyntaxKind::Identifier)
                .map(|token| token.text().to_owned()).collect::<Vec<_>>();
            let binder = match binders.as_slice() {
                [] => None,
                [binder] => Some(binder.clone()),
                _ => return Err(MatchError::unsupported(span, "authored match supports at most one scalar payload binder")),
            };
            let variant = direct_token(&node, SyntaxKind::Identifier)
                .ok_or_else(|| MatchError::invalid(span, "missing enum variant pattern"))?.text().to_owned();
            let body = derive_block(&child(&node, SyntaxKind::Block)?);
            arms.push(Arm { variant, body, binder, span });
        }
        Ok(Self { selector, arms, span })
    }

    pub(super) fn resolve<'a>(&'a self, source: &SourceIdentity, enums: &'a [ScalarEnum],
        inputs: &impl Fn(&str, &ScalarType) -> bool, work: &mut usize,
    ) -> Result<ResolvedMatch<'a>, MatchError> {
        let mut resolver = Resolver { enums, source, enumeration: None, work };
        let selector = resolver.resolve(&self.selector, inputs, 0)?;
        let enumeration = resolver.enumeration.ok_or_else(|| MatchError::unsupported(self.span,
            "authored match requires a resolved enum selector"))?;
        charge(work, enumeration.variants().len(), self.span)?;
        if self.arms.len() != enumeration.variants().len() {
            return Err(MatchError::invalid(self.span, "authored match must cover every enum variant exactly once"));
        }
        let mut tags = enumeration.variants().iter().map(|variant| (variant.name.as_str(), variant))
            .collect::<HashMap<_, _>>();
        if tags.len() != enumeration.variants().len() {
            return Err(MatchError::invalid(self.span, "ambiguous enum variant declarations"));
        }
        let mut arms = Vec::with_capacity(self.arms.len());
        for arm in &self.arms {
            let variant = tags.remove(arm.variant.as_str()).ok_or_else(|| MatchError::invalid(arm.span,
                "duplicate or unknown enum variant pattern"))?;
            let binder = match (variant.payload.as_slice(), arm.binder.as_deref()) {
                ([], None) => None,
                ([slot], Some(binder)) if is_integer_type(&slot.ty) => Some((binder, &slot.ty)),
                ([], Some(_)) => return Err(MatchError::unsupported(arm.span, "tag-only variant has no payload binder")),
                ([slot], _) if is_integer_type(&slot.ty) => return Err(MatchError::invalid(arm.span, "variant pattern requires its payload binder")),
                _ => return Err(MatchError::unsupported(arm.span, "authored match supports one scalar integer payload")),
            };
            arms.push(MatchArm { tag: variant.tag, body: &arm.body, binder });
        }
        Ok(ResolvedMatch { selector, arms })
    }
}

struct Resolver<'a, 'w> {
    enums: &'a [ScalarEnum],
    source: &'w SourceIdentity,
    enumeration: Option<&'a ScalarEnum>,
    work: &'w mut usize,
}

impl<'a> Resolver<'a, '_> {
    fn resolve(&mut self, expression: &'a ScalarExpression,
        inputs: &impl Fn(&str, &ScalarType) -> bool, depth: usize,
    ) -> Result<Selector<'a>, MatchError> {
        let span = expression_span(expression);
        if depth == MAX_ACTUAL_CALL_DEPTH { return Err(MatchError::budget(span)); }
        charge(self.work, 1, span)?;
        let selected = match expression {
            ScalarExpression::Member { receiver, name, .. } => Some((receiver, name, &[][..])),
            ScalarExpression::Call { receiver: Some(receiver), name, type_arguments, arguments, .. }
                if type_arguments.is_empty() => Some((receiver, name, arguments.as_slice())),
            _ => None,
        };
        if let Some((receiver, name, arguments)) = selected {
            let enumeration = match self.enumeration {
                Some(enumeration) if enumeration.name == *receiver => enumeration,
                Some(_) => return Err(MatchError::invalid(span, "authored match selector mixes enum declarations")),
                None => {
                    charge(self.work, self.enums.len(), span)?;
                    let mut declarations = self.enums.iter().filter(|enumeration| enumeration.name == *receiver);
                    let enumeration = declarations.next().ok_or_else(|| MatchError::unsupported(span,
                        "authored match selector must name a local enum variant"))?;
                    if declarations.next().is_some() || &enumeration.id.source != self.source {
                        return Err(MatchError::invalid(span, "ambiguous authored match enum declaration"));
                    }
                    self.enumeration = Some(enumeration);
                    enumeration
                }
            };
            charge(self.work, enumeration.variants().len(), span)?;
            let mut variants = enumeration.variants().iter().filter(|variant| variant.name == *name);
            let selected = variants.next().ok_or_else(|| MatchError::invalid(span, "unknown authored match selector variant"))?;
            if variants.next().is_some() {
                return Err(MatchError::invalid(span, "ambiguous authored match selector variant"));
            }
            let payload = match (selected.payload.as_slice(), arguments) {
                ([], []) => None,
                ([slot], [value]) if is_integer_type(&slot.ty) => {
                    let value_span = expression_span(value);
                    let payload = if let ScalarExpression::Name { name, .. } = value {
                        if !inputs(name, &slot.ty) {
                            return Err(MatchError::invalid(value_span, "payload input does not have the declared integer type"));
                        }
                        Payload::Entry(name.clone())
                    } else {
                        let literal = concrete_integer_literal(value).ok_or_else(|| MatchError::unsupported(
                            value_span, "authored payload must be an integer literal or proof input"))?;
                        if !integer_literal_fits_type(literal.as_ref(), &slot.ty) {
                            return Err(MatchError::invalid(value_span, "payload literal does not fit the declared integer type"));
                        }
                        Payload::Literal(literal.into_owned())
                    };
                    Some(payload)
                }
                ([], _) => return Err(MatchError::invalid(span, "tag-only constructor takes no payload")),
                ([slot], _) if is_integer_type(&slot.ty) => return Err(MatchError::invalid(span, "enum constructor requires one typed payload")),
                _ => return Err(MatchError::unsupported(span, "authored match supports one scalar integer payload")),
            };
            return Ok(Selector::Variant { tag: selected.tag, payload });
        }
        match expression {
            ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                fn output(block: &ScalarBlock) -> Result<&ScalarExpression, MatchError> {
                    let invalid = || MatchError::unsupported(block.span, "authored selector branches must return only a variant");
                    let ([ScalarBlockItem::Expression(value)], [false], [returned]) = (
                        block.items.as_slice(), block.terminated_items.as_slice(), block.final_output_values.as_slice()
                    ) else { return Err(invalid()); };
                    if value != &returned.value { return Err(invalid()); }
                    Ok(value)
                }
                let then_branch = self.resolve(output(then_branch)?, inputs, depth + 1)?;
                let else_branch = self.resolve(output(else_branch)?, inputs, depth + 1)?;
                Ok(Selector::If { condition, then_branch: Box::new(then_branch), else_branch: Box::new(else_branch) })
            }
            _ => Err(MatchError::unsupported(span, "authored match requires a local enum variant or conditional selector")),
        }
    }
}
