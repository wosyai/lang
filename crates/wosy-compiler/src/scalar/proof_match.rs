//! Canonical authored variant matching shared by Build static typing and
//! the ordered checker. This module does not inspect values or prove goals.
use super::{analysis_budget::{charge_call_work, MAX_ACTUAL_CALL_DEPTH}, typecheck::expression_span, *};

pub(super) struct AuthoredMatch {
    selector: ScalarExpression,
    arms: Vec<Arm>,
    span: ByteSpan,
}

struct Arm {
    variant: String,
    body: ScalarBlock,
    span: ByteSpan,
}

pub(super) enum Selector<'a> {
    Variant(#[cfg_attr(not(feature = "internal-checker"), allow(dead_code))] u32),
    If { condition: &'a ScalarExpression, then_branch: Box<Selector<'a>>, else_branch: Box<Selector<'a>> },
}

pub(super) struct ResolvedMatch<'a> {
    pub(super) selector: Selector<'a>,
    pub(super) arms: Vec<(u32, &'a ScalarBlock)>,
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
            if node.children().any(|node| node.kind() == SyntaxKind::Parameters) {
                return Err(MatchError::unsupported(span, "authored match payload bindings are not supported"));
            }
            let variant = direct_token(&node, SyntaxKind::Identifier)
                .ok_or_else(|| MatchError::invalid(span, "missing enum variant pattern"))?.text().to_owned();
            let body = derive_block(&child(&node, SyntaxKind::Block)?);
            arms.push(Arm { variant, body, span });
        }
        Ok(Self { selector, arms, span })
    }

    pub(super) fn resolve<'a>(&'a self, source: &SourceIdentity, enums: &'a [ScalarEnum],
        work: &mut usize,
    ) -> Result<ResolvedMatch<'a>, MatchError> {
        let mut resolver = Resolver { enums, source, enumeration: None, work };
        let selector = resolver.resolve(&self.selector, 0)?;
        let enumeration = resolver.enumeration.ok_or_else(|| MatchError::unsupported(self.span,
            "authored match requires a resolved enum selector"))?;
        charge(work, enumeration.variants.len(), self.span)?;
        if self.arms.len() != enumeration.variants.len() {
            return Err(MatchError::invalid(self.span, "authored match must cover every enum variant exactly once"));
        }
        let mut tags = enumeration.variants.iter().map(|variant| (variant.name.as_str(), variant.tag))
            .collect::<HashMap<_, _>>();
        if tags.len() != enumeration.variants.len() {
            return Err(MatchError::invalid(self.span, "ambiguous enum variant declarations"));
        }
        let mut arms = Vec::with_capacity(self.arms.len());
        for arm in &self.arms {
            let tag = tags.remove(arm.variant.as_str()).ok_or_else(|| MatchError::invalid(arm.span,
                "duplicate or unknown enum variant pattern"))?;
            arms.push((tag, &arm.body));
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
    fn resolve(&mut self, expression: &'a ScalarExpression, depth: usize) -> Result<Selector<'a>, MatchError> {
        let span = expression_span(expression);
        if depth == MAX_ACTUAL_CALL_DEPTH { return Err(MatchError::budget(span)); }
        charge(self.work, 1, span)?;
        match expression {
            ScalarExpression::Member { receiver, name, .. } => {
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
                charge(self.work, enumeration.variants.len(), span)?;
                let mut variants = enumeration.variants.iter().filter(|variant| variant.name == *name);
                let selected = variants.next().ok_or_else(|| MatchError::invalid(span, "unknown authored match selector variant"))?;
                if variants.next().is_some() {
                    return Err(MatchError::invalid(span, "ambiguous authored match selector variant"));
                }
                Ok(Selector::Variant(selected.tag))
            }
            ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                fn output(block: &ScalarBlock) -> Result<&ScalarExpression, MatchError> {
                    let invalid = || MatchError::unsupported(block.span, "authored selector branches must return only a variant");
                    let ([ScalarBlockItem::Expression(value)], [false], [returned]) = (
                        block.items.as_slice(), block.terminated_items.as_slice(), block.final_output_values.as_slice()
                    ) else { return Err(invalid()); };
                    if value != &returned.value { return Err(invalid()); }
                    Ok(value)
                }
                let then_branch = self.resolve(output(then_branch)?, depth + 1)?;
                let else_branch = self.resolve(output(else_branch)?, depth + 1)?;
                Ok(Selector::If { condition, then_branch: Box::new(then_branch), else_branch: Box::new(else_branch) })
            }
            _ => Err(MatchError::unsupported(span, "authored match requires a local enum variant or conditional selector")),
        }
    }
}
