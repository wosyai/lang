use crate::ScalarEnum;
use super::*;
use super::super::checker_flow::{charge_call_work, verify_partitioned_call_return, PartitionedProofGoal, SymbolicLiteralFailure};

pub(super) struct CaseShows<'a> {
    steps: Vec<Step<'a>>,
    work: usize,
    has_citations: bool,
}

enum Step<'a> {
    Show(ByteSpan, Claim<'a>),
    Use { declaration: u32, equality: &'a BigInt },
    Split(Condition, Vec<Step<'a>>, Vec<Step<'a>>),
    Match(Vec<(IntegerSet, Vec<Step<'a>>)>),
}

pub(super) enum Verdict<F> {
    Established,
    Refuted(F),
    Blocked,
}

enum Claim<'a> {
    Literal(IntegerRelation, Cow<'a, BigInt>),
    Entry,
}

impl Claim<'_> {
    fn supplies(&self, premise: &rewrite::Premise<'_>) -> bool {
        match (self, premise) {
            (Self::Entry, rewrite::Premise::Entry) => true,
            (Self::Literal(relation, bound), rewrite::Premise::Literal(value)) =>
                Some(*relation) == IntegerRelation::from_operator(&BinaryOperator::Equal) && bound.as_ref() == *value,
            _ => false,
        }
    }

    fn holds(&self, input: &BigInt, returned: &BigInt) -> bool {
        match self {
            Self::Literal(relation, bound) => relation.accepts(returned.cmp(bound.as_ref())),
            Self::Entry => returned == input,
        }
    }

    fn goal(&self) -> PartitionedProofGoal<'_> {
        match self {
            Self::Literal(relation, bound) => PartitionedProofGoal::Literal(*relation, bound),
            Self::Entry => PartitionedProofGoal::EntryEquality(0),
        }
    }
}

enum Condition {
    Boolean(bool),
    Partition { yes: IntegerSet, no: IntegerSet },
    Integer { operator: BinaryOperator, relation: IntegerRelation, bound: BigInt },
}

impl Condition {
    fn holds(&self, input: &BigInt) -> bool {
        match self {
            Self::Boolean(value) => *value,
            Self::Partition { yes, .. } => yes.contains(input),
            Self::Integer { relation, bound, .. } => relation.accepts(input.cmp(bound)),
        }
    }
}

impl<'a> CaseShows<'a> {
    pub(super) fn prepare(
        plan: &'a PlannedModuleProof<'a>, catalog: Option<&PublicProofCatalog<'a>>,
    ) -> Result<Self, ProjectCheck> {
        let typed = &plan.typed;
        let TypedProofGoal::Literal { operator, bound, .. } = &typed.goal else { return Err(ProjectCheck::Unsupported); };
        if typed.measure.is_some() || !supports_unary_entry_proof(typed.body, typed.result_type)
            || !typed.body.structure.checker_direct_targets.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        let [input] = typed.input_names.as_slice() else { return Err(ProjectCheck::Unsupported); };
        if is_null_literal_name(input) || is_null_literal_name(&typed.output) {
            return Err(ProjectCheck::InvalidProof);
        }
        let [entry] = typed.body.boundary.checker_parameter_bindings.as_deref()
            .ok_or(ProjectCheck::Unsupported)? else { return Err(ProjectCheck::Unsupported); };
        let mut work = CompileWork::default();
        let goal = rewrite::Goal::new(operator, bound);
        let steps = if let Some(matching) = plan.variant_match {
            vec![matching.compile(plan, entry, catalog, goal, &mut work)?]
        } else {
            compile(&plan.steps, plan, entry, catalog, goal, None, 0, &mut work)?
        };
        Ok(Self { steps, work: work.observation, has_citations: work.has_citations })
    }

    pub(super) fn work(&self) -> usize { self.work }

    pub(super) fn has_citations(&self) -> bool { self.has_citations }

    pub(super) fn check_actual(
        &self, input: &BigInt, returned: &BigInt, cite: impl Fn(u32) -> Option<bool>,
    ) -> Option<Verdict<ByteSpan>> {
        fn check(
            steps: &[Step<'_>], input: &BigInt, returned: &BigInt, cite: &impl Fn(u32) -> Option<bool>,
        ) -> Option<Verdict<ByteSpan>> {
            for step in steps {
                match step {
                    Step::Show(span, claim) if !claim.holds(input, returned) => return Some(Verdict::Refuted(*span)),
                    Step::Use { declaration, .. } => {
                        if !cite(*declaration)? { return Some(Verdict::Blocked); }
                    }
                    Step::Split(condition, yes, no) => {
                        let branch = if condition.holds(input) { yes } else { no };
                        let verdict = check(branch, input, returned, cite)?;
                        if !matches!(verdict, Verdict::Established) { return Some(verdict); }
                    }
                    Step::Match(arms) => {
                        let (_, branch) = arms.iter().find(|(domain, _)| domain.contains(input))?;
                        let verdict = check(branch, input, returned, cite)?;
                        if !matches!(verdict, Verdict::Established) { return Some(verdict); }
                    }
                    _ => {}
                }
            }
            Some(Verdict::Established)
        }
        check(&self.steps, input, returned, &cite)
    }

    pub(super) fn check_partitions<'body>(
        &self, producer: &'body FunctionCallSites, returned: &ActualCallResult, domain: &IntegerSet, work: &mut usize,
        mut cite: impl FnMut(u32, &IntegerSet, &mut usize) -> Option<bool>,
    ) -> Option<Verdict<(SymbolicLiteralFailure<'body>, ByteSpan)>> {
        check_partitions(&self.steps, producer, returned, domain, work, &mut cite)
    }
}

// Only preceding, direct shows and authenticated uses are visible. A split
// inherits these from its enclosing block but cannot export its own facts.
struct VisibleShows<'scope, 'a> {
    local: &'scope [Step<'a>],
    outer: Option<&'scope Self>,
}

impl VisibleShows<'_, '_> {
    fn establishes(&self, premise: rewrite::Premise<'_>, work: &mut usize) -> Result<bool, ProjectCheck> {
        let mut visible = Some(self);
        while let Some(scope) = visible {
            charge_call_work(work, 1).ok_or(ProjectCheck::Unsupported)?;
            for step in scope.local.iter().rev() {
                charge_call_work(work, 1).ok_or(ProjectCheck::Unsupported)?;
                if match step {
                    Step::Show(_, claim) => claim.supplies(&premise),
                    Step::Use { equality, .. } => matches!(&premise, rewrite::Premise::Literal(value) if value == equality),
                    Step::Split(..) | Step::Match(..) => false,
                } { return Ok(true); }
            }
            visible = scope.outer;
        }
        Ok(false)
    }
}

#[derive(Default)]
struct CompileWork {
    preparation: usize,
    observation: usize,
    has_citations: bool,
}

pub(super) struct AuthoredMatch {
    selector: ScalarExpression,
    arms: Vec<(String, ScalarBlock)>,
}

impl AuthoredMatch {
    pub(super) fn prepare(node: &CstNode) -> Result<Self, ProjectCheck> {
        // Before recursively lowering expressions or arm blocks, bound the
        // full source syntax. No work is driven by the enum tag's numeric size.
        let mut work = 0;
        charge_call_work(&mut work, u32::from(node.text_range().len()) as usize / 32 + 1)
            .ok_or(ProjectCheck::Unsupported)?;
        let mut pending = vec![(node.clone(), 1)];
        while let Some((node, depth)) = pending.pop() {
            if depth > MAX_ACTUAL_CALL_DEPTH { return Err(ProjectCheck::Unsupported); }
            for child in node.children() {
                charge_call_work(&mut work, 1).ok_or(ProjectCheck::Unsupported)?;
                pending.push((child, depth + 1));
            }
        }
        let selector = derive_expression(&child(node, SyntaxKind::Expression)?);
        let mut arms = Vec::new();
        for node in node.children().filter(|node| node.kind() == SyntaxKind::ProofMatchArm) {
            if node.children().any(|node| node.kind() == SyntaxKind::Parameters) {
                // Currently only payload-free tags have canonical value layout.
                return Err(ProjectCheck::Unsupported);
            }
            let mut names = identifiers(&node);
            let variant = names.next().ok_or(ProjectCheck::InvalidProof)?;
            if names.next().is_some() { return Err(ProjectCheck::InvalidProof); }
            let arm = derive_block(&child(&node, SyntaxKind::Block)?);
            arms.push((variant, arm));
        }
        Ok(Self { selector, arms })
    }

    fn compile<'a>(
        &'a self, plan: &'a PlannedModuleProof<'a>, entry: &ReferenceBindingId,
        catalog: Option<&PublicProofCatalog<'a>>, goal: rewrite::Goal<'a>, work: &mut CompileWork,
    ) -> Result<Step<'a>, ProjectCheck> {
        let owner = plan.typed.target;
        if &owner.source != plan.proof_source { return Err(ProjectCheck::Unsupported); }
        let [parameter] = plan.typed.body.boundary.parameters.as_slice() else { return Err(ProjectCheck::Unsupported); };
        let universe = IntegerSet::from_type(&parameter.ty).ok_or(ProjectCheck::Unsupported)?;
        let mut selector = SelectorCompiler { owner, plan, entry, enumeration: None, domains: HashMap::new(), work };
        selector.visit(&self.selector, universe, 0)?;
        let enumeration = selector.enumeration.ok_or(ProjectCheck::Unsupported)?;
        let mut domains = selector.domains;
        charge_call_work(&mut work.preparation, enumeration.variants.len()).ok_or(ProjectCheck::Unsupported)?;
        if self.arms.len() != enumeration.variants.len() { return Err(ProjectCheck::InvalidProof); }
        let mut tags = enumeration.variants.iter().map(|variant| (variant.name.as_str(), variant.tag))
            .collect::<HashMap<_, _>>();
        if tags.len() != enumeration.variants.len() { return Err(ProjectCheck::InvalidProof); }
        let mut arms = Vec::with_capacity(self.arms.len());
        for (name, arm) in &self.arms {
            let tag = tags.remove(name.as_str()).ok_or(ProjectCheck::InvalidProof)?;
            let domain = domains.remove(&tag).unwrap_or_default();
            charge_call_work(&mut work.observation, domain.interval_count().checked_add(1)
                .ok_or(ProjectCheck::Unsupported)?).ok_or(ProjectCheck::Unsupported)?;
            arms.push((domain, compile(arm, plan, entry, catalog, goal.clone(), None, 1, work)?));
        }
        if !tags.is_empty() || !domains.is_empty() { return Err(ProjectCheck::InvalidProof); }
        Ok(Step::Match(arms))
    }
}

// Resolve a single enum declaration, then partition the authenticated unary
// input by the selected canonical variant tag. Tags never stand for proof
// integers and are not transferred into the ordinary evaluator.
struct SelectorCompiler<'a, 'w> {
    owner: &'a ScalarModule,
    plan: &'a PlannedModuleProof<'a>,
    entry: &'a ReferenceBindingId,
    enumeration: Option<&'a ScalarEnum>,
    domains: HashMap<u32, IntegerSet>,
    work: &'w mut CompileWork,
}

impl SelectorCompiler<'_, '_> {
    fn visit(&mut self, expression: &ScalarExpression, domain: IntegerSet, depth: usize) -> Result<(), ProjectCheck> {
        if depth == MAX_ACTUAL_CALL_DEPTH { return Err(ProjectCheck::Unsupported); }
        charge_call_work(&mut self.work.preparation, 1).ok_or(ProjectCheck::Unsupported)?;
        match expression {
            ScalarExpression::Member { receiver, name, .. } => {
                let enumeration = match self.enumeration {
                    Some(enumeration) if enumeration.name == *receiver => enumeration,
                    Some(_) => return Err(ProjectCheck::InvalidProof),
                    None => {
                        charge_call_work(&mut self.work.preparation, self.owner.enums.len())
                            .ok_or(ProjectCheck::Unsupported)?;
                        let mut declarations = self.owner.enums.iter().filter(|enumeration| enumeration.name == *receiver);
                        let enumeration = declarations.next().ok_or(ProjectCheck::Unsupported)?;
                        if declarations.next().is_some() || enumeration.id.source != self.owner.source {
                            return Err(ProjectCheck::InvalidProof);
                        }
                        self.enumeration = Some(enumeration);
                        enumeration
                    }
                };
                charge_call_work(&mut self.work.preparation, enumeration.variants.len()).ok_or(ProjectCheck::Unsupported)?;
                let mut variants = enumeration.variants.iter().filter(|variant| variant.name == *name);
                let selected = variants.next().ok_or(ProjectCheck::InvalidProof)?;
                if variants.next().is_some() { return Err(ProjectCheck::InvalidProof); }
                let previous = self.domains.entry(selected.tag).or_default();
                let cost = previous.interval_count().checked_add(domain.interval_count()).ok_or(ProjectCheck::Unsupported)?;
                charge_call_work(&mut self.work.preparation, cost).ok_or(ProjectCheck::Unsupported)?;
                *previous = std::mem::take(previous).join(domain);
                Ok(())
            }
            ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                fn output(block: &ScalarBlock) -> Result<&ScalarExpression, ProjectCheck> {
                    let ([ScalarBlockItem::Expression(value)], [false], [returned]) = (
                        block.items.as_slice(), block.terminated_items.as_slice(), block.final_output_values.as_slice()
                    ) else { return Err(ProjectCheck::Unsupported); };
                    if value != &returned.value { return Err(ProjectCheck::Unsupported); }
                    Ok(value)
                }
                let guard = compile_condition(condition, self.plan, self.entry, self.work)?;
                let (yes, no) = match guard {
                    Condition::Boolean(true) => (domain, IntegerSet::default()),
                    Condition::Boolean(false) => (IntegerSet::default(), domain),
                    Condition::Integer { relation, bound, .. } => {
                        charge_call_work(&mut self.work.preparation, domain.interval_count()).ok_or(ProjectCheck::Unsupported)?;
                        domain.split_relation(relation, &bound)
                    }
                    Condition::Partition { yes, no } => {
                        let cost = domain.interval_count().checked_mul(2)
                            .and_then(|cost| cost.checked_add(yes.interval_count()))
                            .and_then(|cost| cost.checked_add(no.interval_count())).ok_or(ProjectCheck::Unsupported)?;
                        charge_call_work(&mut self.work.preparation, cost).ok_or(ProjectCheck::Unsupported)?;
                        (intersect(&domain, &yes), intersect(&domain, &no))
                    }
                };
                // All selectors must resolve to the same declaration even
                // when an arm of the selector is statically unreachable.
                self.visit(output(then_branch)?, yes, depth + 1)?;
                self.visit(output(else_branch)?, no, depth + 1)
            }
            _ => Err(ProjectCheck::Unsupported),
        }
    }
}

fn compile_condition(
    expression: &ScalarExpression, plan: &PlannedModuleProof<'_>, entry: &ReferenceBindingId,
    work: &mut CompileWork,
) -> Result<Condition, ProjectCheck> {
    let typed = &plan.typed;
    let leaf = |expression: &ScalarExpression| {
        match proof_input_condition::<true>(expression, &typed.input_names, typed.body, false)? {
            CheckerPredicate::Boolean(value) => Ok(Condition::Boolean(value)),
            CheckerPredicate::Integer(guard) if &guard.binding == entry => {
                let relation = IntegerRelation::from_operator(&guard.operator).ok_or(ProjectCheck::Unsupported)?;
                Ok(Condition::Integer { operator: guard.operator, relation, bound: guard.bound })
            }
            _ => Err(ProjectCheck::Unsupported),
        }
    };
    if !matches!(expression, ScalarExpression::Unary { operator: UnaryOperator::LogicalNot, .. }
        | ScalarExpression::Binary { operator: BinaryOperator::And | BinaryOperator::Or, .. })
    {
        return leaf(expression);
    }
    let [parameter] = typed.body.boundary.parameters.as_slice() else { return Err(ProjectCheck::Unsupported); };
    let universe = IntegerSet::from_type(&parameter.ty).ok_or(ProjectCheck::Unsupported)?;
    let (yes, no) = compile_partition(expression, &universe, &leaf, 0, &mut work.preparation)?;
    charge_call_work(&mut work.observation, yes.interval_count()).ok_or(ProjectCheck::Unsupported)?;
    Ok(Condition::Partition { yes, no })
}

fn intersect(left: &IntegerSet, right: &IntegerSet) -> IntegerSet {
    match left.intersection(right) {
        IntegerIntersection::Unchanged => left.clone(),
        IntegerIntersection::Empty => IntegerSet::default(),
        IntegerIntersection::Changed(result) => result,
    }
}

// Both sides describe the same typed universe. Boolean operations combine
// exact complementary domains, not independent approximations of truth.
fn compile_partition(
    expression: &ScalarExpression, universe: &IntegerSet,
    leaf: &impl Fn(&ScalarExpression) -> Result<Condition, ProjectCheck>,
    depth: usize, work: &mut usize,
) -> Result<(IntegerSet, IntegerSet), ProjectCheck> {
    if depth == MAX_ACTUAL_CALL_DEPTH { return Err(ProjectCheck::Unsupported); }
    charge_call_work(work, 1).ok_or(ProjectCheck::Unsupported)?;
    match expression {
        ScalarExpression::Unary { operator: UnaryOperator::LogicalNot, operand, .. } => {
            let (yes, no) = compile_partition(operand, universe, leaf, depth + 1, work)?;
            Ok((no, yes))
        }
        ScalarExpression::Binary { operator: operator @ (BinaryOperator::And | BinaryOperator::Or), left, right, .. } => {
            // Validate both sides even when a literal makes one side unreachable.
            let (left_yes, left_no) = compile_partition(left, universe, leaf, depth + 1, work)?;
            let (right_yes, right_no) = compile_partition(right, universe, leaf, depth + 1, work)?;
            for set in [&left_yes, &left_no, &right_yes, &right_no] {
                charge_call_work(work, set.interval_count()).ok_or(ProjectCheck::Unsupported)?;
            }
            if *operator == BinaryOperator::And {
                Ok((intersect(&left_yes, &right_yes), left_no.join(right_no)))
            } else {
                Ok((left_yes.join(right_yes), intersect(&left_no, &right_no)))
            }
        }
        _ => match leaf(expression)? {
            Condition::Boolean(true) => Ok((universe.clone(), IntegerSet::default())),
            Condition::Boolean(false) => Ok((IntegerSet::default(), universe.clone())),
            Condition::Integer { relation, bound, .. } => Ok(universe.split_relation(relation, &bound)),
            Condition::Partition { .. } => Err(ProjectCheck::Unsupported),
        }
    }
}

fn compile<'a>(
    block: &'a ScalarBlock, plan: &'a PlannedModuleProof<'a>, entry: &ReferenceBindingId,
    catalog: Option<&PublicProofCatalog<'a>>, mut current: rewrite::Goal<'a>,
    outer: Option<&VisibleShows<'_, 'a>>, depth: usize, work: &mut CompileWork,
) -> Result<Vec<Step<'a>>, ProjectCheck> {
    let typed = &plan.typed;
    if depth == MAX_ACTUAL_CALL_DEPTH || block.terminated_items.len() != block.items.len() {
        return Err(ProjectCheck::Unsupported);
    }
    charge_call_work(&mut work.preparation, block.items.len().checked_add(1).ok_or(ProjectCheck::Unsupported)?)
        .ok_or(ProjectCheck::Unsupported)?;
    charge_call_work(&mut work.observation, 1).ok_or(ProjectCheck::Unsupported)?;
    let mut steps = Vec::with_capacity(block.items.len());
    for (item, &terminated) in block.items.iter().zip(&block.terminated_items) {
        let visible = VisibleShows { local: &steps, outer };
        if let Some((equation, context)) = rewrite::arguments(item) {
            if !terminated { return Err(ProjectCheck::Unsupported); }
            let [formal] = typed.input_names.as_slice() else { return Err(ProjectCheck::Unsupported); };
            current.apply(equation, context, &typed.output, typed.result_type, Some(formal), |premise| {
                visible.establishes(premise, &mut work.preparation)
            })?;
            // The preceding show still gets verified on these returned
            // states. Under it, both goal spellings are equivalent, so
            // neither return observation nor terminal checking is replayed.
            continue;
        }
        let cited = terminated.then(|| plan.peer_use_step(item)).flatten();
        let step = match item {
            ScalarBlockItem::Expression(ScalarExpression::If { condition: predicate, then_branch, else_branch, .. }) =>
                Step::Split(compile_condition(predicate, plan, entry, work)?,
                    compile(then_branch, plan, entry, catalog, current.clone(), Some(&visible), depth + 1, work)?,
                    compile(else_branch, plan, entry, catalog, current.clone(), Some(&visible), depth + 1, work)?),
            ScalarBlockItem::Expression(ScalarExpression::UnitIf { condition: predicate, then_branch, .. }) =>
                Step::Split(compile_condition(predicate, plan, entry, work)?,
                    compile(then_branch, plan, entry, catalog, current.clone(), Some(&visible), depth + 1, work)?, Vec::new()),
            _ if cited.is_some() => {
                let name = cited.ok_or(ProjectCheck::Unsupported)?;
                let peer = catalog.and_then(|catalog| catalog.resolve(plan.proof_source, name))
                    .ok_or(ProjectCheck::Unsupported)?;
                let (relation, equality) = peer.peer_literal_goal().ok_or(ProjectCheck::Unsupported)?;
                if !peer.has_no_authored_steps() || !plan.same_peer_target(peer)
                    || Some(relation) != IntegerRelation::from_operator(&BinaryOperator::Equal)
                {
                    return Err(ProjectCheck::Unsupported);
                }
                work.has_citations = true;
                Step::Use { declaration: wosy_syntax::byte_span(peer.proof).start, equality }
            }
            ScalarBlockItem::Expression(expression) => {
                let claim = proof_show_expression(item, terminated)?;
                let (output, names, ty) = (&typed.output, &typed.input_names, typed.result_type);
                let claim = if let Some(comparison) = IntegerComparison::from_expression(claim) {
                    let (_, relation, bound) = checked_literal_output_goal_borrowed(comparison, output, names, ty)?;
                    Claim::Literal(relation, bound)
                } else if let Some(old) = checked_old_output_relation(claim, output, names, typed.body, ty) {
                    let (slot, relation) = old?;
                    if slot != 0 || Some(relation) != IntegerRelation::from_operator(&BinaryOperator::Equal) {
                        return Err(ProjectCheck::Unsupported);
                    }
                    Claim::Entry
                } else {
                    let [formal] = names.as_slice() else { return Err(ProjectCheck::Unsupported); };
                    if !matches!(claim, ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. }
                        if matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output)
                        && matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == formal))
                    {
                        return Err(ProjectCheck::Unsupported);
                    }
                    Claim::Entry
                };
                Step::Show(super::super::typecheck::expression_span(expression), claim)
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        charge_call_work(&mut work.observation, 1).ok_or(ProjectCheck::Unsupported)?;
        steps.push(step);
    }
    Ok(steps)
}

fn check_partitions<'body>(
    steps: &[Step<'_>], producer: &'body FunctionCallSites, returned: &ActualCallResult,
    domain: &IntegerSet, work: &mut usize, cite: &mut impl FnMut(u32, &IntegerSet, &mut usize) -> Option<bool>,
) -> Option<Verdict<(SymbolicLiteralFailure<'body>, ByteSpan)>> {
    if domain.is_empty() { return Some(Verdict::Established); }
    for step in steps {
        charge_call_work(work, 1)?;
        let verdict = match step {
            Step::Show(span, claim) => verify_partitioned_call_return(producer, returned, domain, false, claim.goal(), Some(work))?
                .failure.map_or(Verdict::Established, |failure| Verdict::Refuted((failure, *span))),
            Step::Use { declaration, .. } => {
                if cite(*declaration, domain, work)? { Verdict::Established } else { Verdict::Blocked }
            }
            Step::Match(arms) => {
                for (arm, steps) in arms {
                    let cost = domain.interval_count().checked_add(arm.interval_count())?.checked_add(1)?;
                    charge_call_work(work, cost)?;
                    let active = intersect(domain, arm);
                    let verdict = check_partitions(steps, producer, returned, &active, work, cite)?;
                    if !matches!(verdict, Verdict::Established) { return Some(verdict); }
                }
                Verdict::Established
            }
            Step::Split(Condition::Boolean(guard), yes, no) =>
                check_partitions(if *guard { yes } else { no }, producer, returned, domain, work, cite)?,
            Step::Split(Condition::Partition { yes: positive, no: negative }, yes, no) => {
                let cost = domain.interval_count().checked_mul(2)?
                    .checked_add(positive.interval_count())?.checked_add(negative.interval_count())?;
                charge_call_work(work, cost)?;
                let admitted = intersect(domain, positive);
                let verdict = check_partitions(yes, producer, returned, &admitted, work, cite)?;
                if !matches!(verdict, Verdict::Established) { return Some(verdict); }
                let excluded = intersect(domain, negative);
                check_partitions(no, producer, returned, &excluded, work, cite)?
            }
            Step::Split(Condition::Integer { operator, bound, .. }, yes, no) => {
                charge_call_work(work, domain.interval_count())?;
                let (admitted, excluded) = domain.split(operator, bound)?;
                let verdict = check_partitions(yes, producer, returned, &admitted, work, cite)?;
                if !matches!(verdict, Verdict::Established) { return Some(verdict); }
                check_partitions(no, producer, returned, &excluded, work, cite)?
            }
        };
        if !matches!(verdict, Verdict::Established) { return Some(verdict); }
    }
    Some(Verdict::Established)
}
