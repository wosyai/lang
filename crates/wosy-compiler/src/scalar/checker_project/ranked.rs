//! Check integer self-use at each leaf of the prepared return tree. The path is
//! a rectangle because every enrolled guard restricts exactly one entry slot.
//! No recursive call is executed here and no conclusion is installed by this pass.
use super::*;
use super::super::{
    checker_flow::charge_call_work,
    reference::{OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity},
    typecheck::expression_span,
};

fn charge(work: &mut usize, cost: usize) -> Result<(), ProjectCheck> {
    charge_call_work(work, cost).ok_or(ProjectCheck::Unsupported)
}

impl<'a> TypedInductionRank<'a> {
    pub(super) fn entries(self) -> impl Iterator<Item = TypedEntryMeasure<'a>> {
        std::iter::once(self.first_parameter).chain(self.second_parameter)
    }

    fn axes(self) -> impl Iterator<Item = TypedEntryMeasure<'a>> {
        self.earlier.into_iter().chain([self.descending])
    }
}

#[derive(Clone)]
struct Rectangle(Vec<IntegerSet>);

impl Rectangle {
    fn entry(
        rank: TypedInductionRank<'_>, filter: &[CheckerPredicate], work: &mut usize,
    ) -> Result<Self, ProjectCheck> {
        let mut axes = rank.entries().map(|(slot, _)| IntegerSet::from_type(&slot.ty)
            .ok_or(ProjectCheck::Unsupported)).collect::<Result<Vec<_>, _>>()?;
        charge(work, filter.len())?;
        for atom in filter {
            match atom {
                CheckerPredicate::Boolean(false) => axes[0] = IntegerSet::default(),
                CheckerPredicate::Boolean(true) => {}
                CheckerPredicate::Integer(p) => {
                    let i = rank.entries().position(|(slot, id)| id == &p.binding && slot.ty == p.ty)
                        .ok_or(ProjectCheck::Unsupported)?;
                    charge(work, axes[i].interval_count())?;
                    axes[i] = axes[i].keep(&p.operator, &p.bound).ok_or(ProjectCheck::Unsupported)?;
                }
                CheckerPredicate::EntryDomain { binding, values } => {
                    let i = rank.entries().position(|(_, id)| id == binding).ok_or(ProjectCheck::Unsupported)?;
                    charge(work, axes[i].interval_count().saturating_add(values.interval_count()))?;
                    match axes[i].intersection(values) {
                        IntegerIntersection::Unchanged => {}
                        IntegerIntersection::Changed(kept) => axes[i] = kept,
                        IntegerIntersection::Empty => axes[i] = IntegerSet::default(),
                    }
                }
                _ => return Err(ProjectCheck::Unsupported),
            }
        }
        let rectangle = Self(axes);
        if rank.sum && rectangle.reachable() {
            let [first, second] = rectangle.0.as_slice() else { return Err(ProjectCheck::Unsupported); };
            let (first_low, first_high) = first.extremes().ok_or(ProjectCheck::Unsupported)?;
            let (second_low, second_high) = second.extremes().ok_or(ProjectCheck::Unsupported)?;
            charge(work, 1)?;
            // Every precondition state (therefore every admitted successor)
            // must evaluate the authored checked sum without overflow.
            let ty = &rank.first_parameter.0.ty;
            if !integer_literal_fits_type(&(first_low + second_low), ty)
                || !integer_literal_fits_type(&(first_high + second_high), ty)
            {
                return Err(ProjectCheck::Unsupported);
            }
        }
        Ok(rectangle)
    }

    fn reachable(&self) -> bool { self.0.iter().all(|domain| !domain.is_empty()) }

    fn split(
        mut self, rank: TypedInductionRank<'_>, guard: &CheckerPredicate, work: &mut usize,
    ) -> Result<(Self, Self), ProjectCheck> {
        // Pay for partitioning one axis and transporting its unchanged companions.
        for axis in &self.0 { charge(work, axis.interval_count().saturating_add(1))?; }
        let (slot, yes, no) = match guard {
            CheckerPredicate::Boolean(value) => {
                let full = std::mem::take(&mut self.0[0]);
                if *value { (0, full, IntegerSet::default()) } else { (0, IntegerSet::default(), full) }
            }
            CheckerPredicate::Integer(p) => {
                let i = rank.entries().position(|(slot, id)| id == &p.binding && slot.ty == p.ty)
                    .ok_or(ProjectCheck::Unsupported)?;
                let (yes, no) = std::mem::take(&mut self.0[i]).split(&p.operator, &p.bound)
                    .ok_or(ProjectCheck::Unsupported)?;
                (i, yes, no)
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        let mut other = self.clone(); // split axis is empty here
        self.0[slot] = yes;
        other.0[slot] = no;
        Ok((self, other))
    }
}

#[derive(PartialEq)]
enum Coordinate<'a> {
    Entry,
    Offset { add: bool, magnitude: &'a BigInt },
    Literal(Cow<'a, BigInt>),
}

impl<'a> Coordinate<'a> {
    // The same typed operand grammar is used for authored and runtime arguments.
    // The caller separately binds a runtime expression to its prepared occurrence.
    fn read(
        expr: &'a ScalarExpression, ty: &ScalarType, is_entry: impl Fn(&ScalarExpression) -> bool,
    ) -> Result<Self, ProjectCheck> {
        if let Some(literal) = concrete_integer_literal(expr) {
            if integer_literal_fits_type(&literal, ty) { return Ok(Self::Literal(literal)); }
        } else if is_entry(expr) {
            return Ok(Self::Entry);
        } else if let ScalarExpression::Binary { operator, left, right, .. } = expr {
            if let ScalarExpression::Integer { value, .. } = right.as_ref() {
                if matches!(operator, BinaryOperator::Add | BinaryOperator::Subtract)
                    && value.sign() != num_bigint::Sign::Minus && integer_literal_fits_type(value, ty) && is_entry(left)
                {
                    return Ok(Self::Offset { add: *operator == BinaryOperator::Add, magnitude: value });
                }
            }
        }
        Err(ProjectCheck::Unsupported)
    }
}

fn self_use<'a>(
    name: &str, steps: &'a ScalarBlock, typed: &TypedProofPostcondition<'_>, rank: TypedInductionRank<'_>,
) -> Result<Vec<Coordinate<'a>>, ProjectCheck> {
    let [ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver: Some(core), name: instruction, type_arguments: outer_types, arguments: outer, ..
    })] = steps.items.as_slice() else { return Err(ProjectCheck::Unsupported); };
    let [ScalarExpression::Call { receiver: None, name: cited, arguments, type_arguments, .. }] = outer.as_slice()
        else { return Err(ProjectCheck::Unsupported); };
    if steps.terminated_items.as_slice() != [true] || core != "core" || instruction != "use"
        || cited != name || !type_arguments.is_empty() || !outer_types.is_empty()
        || arguments.len() != typed.input_names.len() || arguments.len() != rank.entries().count()
    {
        return Err(ProjectCheck::Unsupported);
    }
    rank.entries().map(|(slot, _)| {
        let entry = typed.input_names.get(slot.ordinal).ok_or(ProjectCheck::Unsupported)?;
        Coordinate::read(&arguments[slot.ordinal], &slot.ty,
            |expr| matches!(expr, ScalarExpression::Name { name, .. } if name == entry))
    }).collect()
}

fn check_tail(
    plan: &PlannedModuleProof<'_>, rank: TypedInductionRank<'_>, call: OccurrenceOrdinal,
    runtime: &ScalarBlock, proof_args: &[Coordinate<'_>],
) -> Result<(), ProjectCheck> {
    let body = plan.typed.body;
    let structure = &body.structure;
    let mut outputs = runtime.final_expressions();
    let Some((expr @ ScalarExpression::Call { arguments, .. }, _)) = outputs.next() else {
        return Err(ProjectCheck::Unsupported);
    };
    let signature = body.checked_call_signatures.get(&call).ok_or(ProjectCheck::Unsupported)?;
    if outputs.next().is_some() || body.occurrences.get(call.0).map(|node| node.span) != Some(expression_span(expr))
        || body.resolved_call_site(call).map(|site| &site.target) != Some(plan.canonical_target)
        || signature.parameters.len() != proof_args.len() || arguments.len() != proof_args.len()
        || !matches!(signature.outputs.outputs.as_slice(), [output] if &output.ty == plan.typed.result_type)
    {
        return Err(ProjectCheck::Unsupported);
    }
    let operand = |parent, edge, expr: &ScalarExpression, ty: &ScalarType| {
        let OccurrenceNode::Expression(child) = structure.edges.get(&(OccurrenceParent::Expression(parent), edge))?
            else { return None; };
        (body.occurrences.get(child.0)?.span == expression_span(expr)
            && structure.checked_types.expressions.get(child) == Some(ty)).then_some(*child)
    };
    for ((slot, binding), expected) in rank.entries().zip(proof_args) {
        if signature.parameters[slot.ordinal] != slot.ty { return Err(ProjectCheck::Unsupported); }
        let expr = &arguments[slot.ordinal];
        let arg = operand(call, OccurrenceEdge::CallArgument(slot.ordinal), expr, &slot.ty).ok_or(ProjectCheck::Unsupported)?;
        let name = if let ScalarExpression::Binary { left, right, .. } = expr {
            operand(arg, OccurrenceEdge::BinaryRight, right, &slot.ty).ok_or(ProjectCheck::Unsupported)?;
            operand(arg, OccurrenceEdge::BinaryLeft, left, &slot.ty).ok_or(ProjectCheck::Unsupported)?
        } else { arg };
        let actual = Coordinate::read(expr, &slot.ty, |expr| matches!(expr, ScalarExpression::Name { .. })
            && matches!(body.prepared_plain_name(name), Some(PlainNameIdentity::Lexical(id)) if id == binding))?;
        if &actual != expected { return Err(ProjectCheck::Unsupported); }
    }
    Ok(())
}

// Each expression consumes only its own coordinate. Thus whenever an earlier
// axis can tie, *all* values in the next axis are possible on that tie.
fn transition(
    rank: TypedInductionRank<'_>, entry: &Rectangle, path: &Rectangle, arguments: &[Coordinate<'_>], work: &mut usize,
) -> Result<InductionPremise, ProjectCheck> {
    if !path.reachable() { return Ok(InductionPremise::Unreachable); }
    use std::cmp::Ordering;
    let mut comparisons = [Ordering::Equal; 2];
    let mut sum_delta = rank.sum.then(BigInt::default);
    // Authenticate every actual call argument and its guard, even if a
    // coordinate is not ranked or an earlier coordinate already descends.
    for (i, ((slot, _), arg)) in rank.entries().zip(arguments).enumerate() {
        let domain = &entry.0[i];
        let active = &path.0[i];
        let (low, high) = active.extremes().ok_or(ProjectCheck::Unsupported)?;
        comparisons[i] = match arg {
            Coordinate::Entry => Ordering::Equal,
            Coordinate::Literal(value) => {
                charge(work, domain.interval_count())?;
                if !domain.contains(value) { return Ok(InductionPremise::CalleeWhereUnproved); }
                if let Some(delta) = &mut sum_delta {
                    charge(work, 1)?;
                    *delta += value.as_ref() - low;
                }
                value.as_ref().cmp(low)
            }
            Coordinate::Offset { add, magnitude } => {
                let moved = |x: &BigInt| if *add { x + *magnitude } else { x - *magnitude };
                let (min, max) = integer_bounds(&slot.ty).ok_or(ProjectCheck::Unsupported)?;
                if moved(low) < min || moved(high) > max { return Ok(InductionPremise::NotStrict); }
                let probes = (usize::BITS - domain.interval_count().leading_zeros()) as usize + 1;
                charge(work, active.interval_count().saturating_mul(probes))?;
                if !active.intervals().all(|(lo, hi)| domain.covers_closed(&moved(lo), &moved(hi))) {
                    return Ok(InductionPremise::CalleeWhereUnproved);
                }
                if let Some(delta) = &mut sum_delta {
                    charge(work, 1)?;
                    if *add { *delta += *magnitude; } else { *delta -= *magnitude; }
                }
                moved(low).cmp(low)
            }
        };
    }
    if let Some(max_delta) = sum_delta {
        // These coordinate changes use independent input axes, so their
        // maxima are simultaneously attainable at a guarded path point.
        return Ok(if max_delta.sign() == num_bigint::Sign::Minus { InductionPremise::Established }
            else { InductionPremise::NotStrict });
    }
    // With independent axes, if an earlier coordinate can tie, all values
    // in each suffix coordinate are possible on that tie.
    for (slot, _) in rank.axes() {
        match comparisons[slot.ordinal] {
            Ordering::Greater => return Ok(InductionPremise::NotStrict),
            Ordering::Less => return Ok(InductionPremise::Established),
            Ordering::Equal => {}
        }
    }
    Ok(InductionPremise::NotStrict)
}

pub(super) fn verify<'a>(
    plan: &'a PlannedModuleProof<'_>, rank: TypedInductionRank<'_>,
) -> Result<RankedInductionOutcome<'a>, ProjectCheck> {
    let typed = &plan.typed;
    let body = typed.body;
    let structure = &body.structure;
    if typed.input_names.len() != rank.entries().count() || body.call_occurrences.is_empty()
        || !body.assignments.is_empty() || !body.allocation_length_calls.is_empty()
        || !structure.checker_directives.is_empty() || !body.arms.is_empty()
        || structure.has_ordinary_contextual_places() || structure.checker_has_bool_contextual_declaration
    {
        return Err(ProjectCheck::Unsupported);
    }
    let (relation, bound) = match &typed.goal {
        TypedProofGoal::Literal { relation, bound, .. } | TypedProofGoal::Universal { relation, bound } => (relation, bound),
        _ => return Err(ProjectCheck::Unsupported),
    };
    let mut work = 0;
    let entry = Rectangle::entry(rank, plan.filter.as_deref().unwrap_or_default(), &mut work)?;
    for axis in &entry.0 { charge(&mut work, axis.interval_count())?; }
    let mut pending = vec![(plan.steps.as_ref(), &plan.function.body, structure.root, entry.clone())];
    let mut matched_calls = 0;
    let mut has_base = false;
    let mut failure = None;
    let mut premise = InductionPremise::Unreachable;
    while let Some((proof, runtime, block, path)) = pending.pop() {
        charge(&mut work, 1)?;
        let output = ranked_output_at(body, block, typed.result_type).ok_or(ProjectCheck::Unsupported)?;
        let [item] = proof.items.as_slice() else { return Err(ProjectCheck::Unsupported); };
        if let Some(ConditionalRegion::Value { condition, then_block, else_block, .. }) = body.conditionals.get(&output) {
            let guard = structure.checker_branches.get(condition).ok_or(ProjectCheck::Unsupported)?;
            let branches = runtime.tail_conditional().ok_or(ProjectCheck::Unsupported)?;
            if body.occurrences.get(output.0).map(|node| node.span) != Some(branches.span) {
                return Err(ProjectCheck::Unsupported);
            }
            let (yes_proof, no_proof) = match item {
                ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. }) => {
                    let asserted = proof_input_condition::<true>(condition, &typed.input_names, body, false)?;
                    if &asserted != guard { return Err(ProjectCheck::Unsupported); }
                    (then_branch, else_branch)
                }
                // An unbranched show is an obligation on every actual returning leaf.
                ScalarBlockItem::Expression(ScalarExpression::Call { receiver: Some(core), name, .. })
                    if core == "core" && name == "show" => (proof, proof),
                _ => return Err(ProjectCheck::Unsupported),
            };
            let (yes, no) = path.split(rank, guard, &mut work)?;
            pending.push((no_proof, branches.else_branch, *else_block, no));
            pending.push((yes_proof, branches.then_branch, *then_block, yes));
        } else if let Some(value) = structure.checker_integer_literals.get(&output) {
            has_base = true;
            let (claim, show_span) = prepare_ranked_base_show(proof, typed)?;
            if failure.is_none() && path.reachable() && (claim.as_ref() != value || !relation.accepts(value.cmp(bound))) {
                let rejected = if claim.as_ref() != value {
                    RankedBaseRejection::AuthoredShow(show_span)
                } else {
                    let goal = child(&child(plan.proof, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
                    RankedBaseRejection::Goal(wosy_syntax::byte_span(&goal))
                };
                let point = |i: usize| path.0[i].extremes().expect("reachable axis").0.clone();
                failure = Some(RankedBaseWitness {
                    proof_source: plan.proof_source, return_source: &typed.target.source,
                    entry_type: rank.second_parameter.unwrap_or(rank.first_parameter).0.ty.clone(), earlier: rank.second_parameter.map(|_| point(0)),
                    entry: point(path.0.len() - 1), returned: value,
                    return_span: body.occurrences.get(output.0).ok_or(ProjectCheck::Unsupported)?.span, rejected,
                });
            }
        } else {
            let args = self_use(plan.name, proof, typed, rank)?;
            check_tail(plan, rank, output, runtime, &args)?;
            matched_calls += 1;
            match transition(rank, &entry, &path, &args, &mut work)? {
                InductionPremise::Unreachable => {}
                next if matches!(premise, InductionPremise::Unreachable | InductionPremise::Established) => premise = next,
                _ => {}
            }
        }
    }
    // Every visited call occupies a distinct, direct final-output slot in the
    // prepared tree. No additional call in a condition or argument is covered.
    if !has_base || matched_calls != body.call_occurrences.len() { return Err(ProjectCheck::Unsupported); }
    Ok(if !entry.reachable() { RankedInductionOutcome::NoApplicable }
        else if let Some(base) = failure { RankedInductionOutcome::Refuted(base) }
        else { RankedInductionOutcome::Premise(premise) })
}

#[cfg(test)]
pub(super) fn audit_premise(
    plan: &PlannedModuleProof<'_>, rank: TypedInductionRank<'_>,
) -> Result<InductionPremise, ProjectCheck> {
    let [ScalarBlockItem::Expression(ScalarExpression::If { condition, else_branch, .. })] = plan.steps.items.as_slice()
        else { return Err(ProjectCheck::Unsupported); };
    let mut work = 0;
    let entry = Rectangle::entry(rank, plan.filter.as_deref().unwrap_or_default(), &mut work)?;
    let guard = proof_input_condition::<true>(condition, &plan.typed.input_names, plan.typed.body, false)?;
    let (_, recursive) = entry.clone().split(rank, &guard, &mut work)?;
    let args = self_use(plan.name, else_branch, &plan.typed, rank)?;
    transition(rank, &entry, &recursive, &args, &mut work)
}
