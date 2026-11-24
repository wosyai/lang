use super::*;
use super::super::checker_flow::{charge_call_work, verify_partitioned_call_return, PartitionedProofGoal, SymbolicLiteralFailure};

pub(super) struct CaseShows<'a> {
    steps: Vec<Step<'a>>,
    work: usize,
}

enum Step<'a> {
    Show(ByteSpan, Claim<'a>),
    Split(Condition, Vec<Step<'a>>, Vec<Step<'a>>),
}

enum Claim<'a> {
    Literal(IntegerRelation, Cow<'a, BigInt>),
    Entry,
}

impl Claim<'_> {
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
    Integer { operator: BinaryOperator, relation: IntegerRelation, bound: BigInt },
}

impl Condition {
    fn holds(&self, input: &BigInt) -> bool {
        match self {
            Self::Boolean(value) => *value,
            Self::Integer { relation, bound, .. } => relation.accepts(input.cmp(bound)),
        }
    }
}

impl<'a> CaseShows<'a> {
    pub(super) fn prepare(plan: &'a PlannedModuleProof<'a>) -> Result<Self, ProjectCheck> {
        let typed = &plan.typed;
        if typed.measure.is_some() || !matches!(typed.goal, TypedProofGoal::Literal { .. })
            || !supports_unary_entry_proof(typed.body, typed.result_type)
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
        let mut work = 0;
        let steps = compile(&plan.steps, typed, entry, 0, &mut work)?;
        Ok(Self { steps, work })
    }

    pub(super) fn work(&self) -> usize { self.work }

    pub(super) fn check_actual(&self, input: &BigInt, returned: &BigInt) -> Option<ByteSpan> {
        fn first_failure(steps: &[Step<'_>], input: &BigInt, returned: &BigInt) -> Option<ByteSpan> {
            for step in steps {
                match step {
                    Step::Show(span, claim) if !claim.holds(input, returned) => return Some(*span),
                    Step::Split(condition, yes, no) => {
                        let branch = if condition.holds(input) { yes } else { no };
                        if let Some(span) = first_failure(branch, input, returned) { return Some(span); }
                    }
                    _ => {}
                }
            }
            None
        }
        first_failure(&self.steps, input, returned)
    }

    pub(super) fn check_partitions<'body>(
        &self, producer: &'body FunctionCallSites, returned: &ActualCallResult, domain: &IntegerSet, work: &mut usize,
    ) -> Option<Option<(SymbolicLiteralFailure<'body>, ByteSpan)>> {
        check_partitions(&self.steps, producer, returned, domain, work)
    }
}

fn compile<'a>(
    block: &'a ScalarBlock, typed: &TypedProofPostcondition<'_>, entry: &ReferenceBindingId, depth: usize, work: &mut usize,
) -> Result<Vec<Step<'a>>, ProjectCheck> {
    if depth == MAX_ACTUAL_CALL_DEPTH || block.terminated_items.len() != block.items.len() {
        return Err(ProjectCheck::Unsupported);
    }
    charge_call_work(work, block.items.len().checked_add(1).ok_or(ProjectCheck::Unsupported)?)
        .ok_or(ProjectCheck::Unsupported)?;
    let condition = |expression: &ScalarExpression| {
        match proof_input_condition::<true>(expression, &typed.input_names, typed.body, false)? {
            CheckerPredicate::Boolean(value) => Ok(Condition::Boolean(value)),
            CheckerPredicate::Integer(guard) if &guard.binding == entry => {
                let relation = IntegerRelation::from_operator(&guard.operator).ok_or(ProjectCheck::Unsupported)?;
                Ok(Condition::Integer { operator: guard.operator, relation, bound: guard.bound })
            }
            _ => Err(ProjectCheck::Unsupported),
        }
    };
    let mut steps = Vec::with_capacity(block.items.len());
    for (item, &terminated) in block.items.iter().zip(&block.terminated_items) {
        let step = match item {
            ScalarBlockItem::Expression(ScalarExpression::If { condition: predicate, then_branch, else_branch, .. }) =>
                Step::Split(condition(predicate)?, compile(then_branch, typed, entry, depth + 1, work)?,
                    compile(else_branch, typed, entry, depth + 1, work)?),
            ScalarBlockItem::Expression(ScalarExpression::UnitIf { condition: predicate, then_branch, .. }) =>
                Step::Split(condition(predicate)?, compile(then_branch, typed, entry, depth + 1, work)?, Vec::new()),
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
        steps.push(step);
    }
    Ok(steps)
}

fn check_partitions<'body>(
    steps: &[Step<'_>], producer: &'body FunctionCallSites, returned: &ActualCallResult,
    domain: &IntegerSet, work: &mut usize,
) -> Option<Option<(SymbolicLiteralFailure<'body>, ByteSpan)>> {
    if domain.is_empty() { return Some(None); }
    for step in steps {
        charge_call_work(work, 1)?;
        let failure = match step {
            Step::Show(span, claim) => verify_partitioned_call_return(producer, returned, domain, false, claim.goal(), Some(work))?
                .failure.map(|failure| (failure, *span)),
            Step::Split(Condition::Boolean(guard), yes, no) =>
                check_partitions(if *guard { yes } else { no }, producer, returned, domain, work)?,
            Step::Split(Condition::Integer { operator, bound, .. }, yes, no) => {
                charge_call_work(work, domain.interval_count())?;
                let (admitted, excluded) = domain.split(operator, bound)?;
                if let Some(failure) = check_partitions(yes, producer, returned, &admitted, work)? {
                    return Some(Some(failure));
                }
                check_partitions(no, producer, returned, &excluded, work)?
            }
        };
        if failure.is_some() { return Some(failure); }
    }
    Some(None)
}
