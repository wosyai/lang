//! Check typed self-use at each leaf of the prepared return tree. Integer
//! paths are rectangles; a bounded float rank has one exact IEEE lane.
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
            let ty = &rank.entries().next().ok_or(ProjectCheck::Unsupported)?.0.ty;
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
    FloatLiteral(ranked_float::Atom),
    FloatOffset { add: bool, magnitude: ranked_float::Atom },
}

impl<'a> Coordinate<'a> {
    // The same typed operand grammar is used for authored and runtime arguments.
    // The caller separately binds a runtime expression to its prepared occurrence.
    fn read(
        expr: &'a ScalarExpression, ty: &ScalarType, is_entry: impl Fn(&ScalarExpression) -> bool,
    ) -> Result<Self, ProjectCheck> {
        if matches!(ty, ScalarType::F32 | ScalarType::F64) {
            if is_entry(expr) { return Ok(Self::Entry); }
            if let Some(literal) = ranked_float::Atom::literal(expr, ty) { return Ok(Self::FloatLiteral(literal)); }
            let ScalarExpression::Binary { operator, left, right, .. } = expr else { return Err(ProjectCheck::Unsupported); };
            if is_entry(left) && matches!(operator, BinaryOperator::Add | BinaryOperator::Subtract) {
                let magnitude = ranked_float::Atom::literal(right, ty).ok_or(ProjectCheck::Unsupported)?;
                return Ok(Self::FloatOffset { add: *operator == BinaryOperator::Add, magnitude });
            }
            return Err(ProjectCheck::Unsupported);
        }
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
    name: &str, item: &'a ScalarBlockItem, terminated: bool,
    typed: &TypedProofPostcondition<'_>, rank: TypedInductionRank<'_>,
) -> Result<Vec<Coordinate<'a>>, ProjectCheck> {
    let ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver: Some(core), name: instruction, type_arguments: outer_types, arguments: outer, ..
    }) = item else { return Err(ProjectCheck::Unsupported); };
    let [ScalarExpression::Call { receiver: None, name: cited, arguments, type_arguments, .. }] = outer.as_slice()
        else { return Err(ProjectCheck::Unsupported); };
    if !terminated || core != "core" || instruction != "use"
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
    let mut comparisons = [Ordering::Equal; MAX_RANKED_INTEGER_AXES];
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
            Coordinate::FloatLiteral(_) | Coordinate::FloatOffset { .. } => return Err(ProjectCheck::Unsupported),
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

#[derive(Clone)]
enum RankLane {
    Integers(Rectangle),
    Float(ranked_float::Lane),
}

impl RankLane {
    fn reachable(&self) -> bool {
        match self {
            Self::Integers(rectangle) => rectangle.reachable(),
            Self::Float(lane) => lane.reachable(),
        }
    }

    // An authored show is an obligation, never a filter. Check that its
    // complement has no active entry without changing the incoming lane.
    fn proves_entry_show(
        &self, claim: &ScalarExpression, typed: &TypedProofPostcondition<'_>,
        rank: TypedInductionRank<'_>, work: &mut usize,
    ) -> Result<bool, ProjectCheck> {
        match self {
            Self::Integers(path) => {
                let predicate = proof_input_condition::<true>(claim, &typed.input_names, typed.body, false)?;
                charge(work, path.0.len())?;
                match predicate {
                    CheckerPredicate::Boolean(value) => Ok(value || !path.reachable()),
                    CheckerPredicate::Integer(p) => {
                        let axis = rank.entries().position(|(slot, key)| slot.ty == p.ty && *key == p.binding)
                            .and_then(|slot| path.0.get(slot)).ok_or(ProjectCheck::Unsupported)?;
                        let opposite = IntegerRelation::from_operator(&p.operator)
                            .ok_or(ProjectCheck::Unsupported)?.complement();
                        charge(work, axis.interval_count())?;
                        Ok(!path.reachable() || axis.keep_relation(opposite, &p.bound).is_empty())
                    }
                    _ => Err(ProjectCheck::Unsupported),
                }
            }
            Self::Float(path) => {
                let (axis, _) = rank.float_entry().ok_or(ProjectCheck::Unsupported)?;
                let entry_name = typed.input_names.get(axis.ordinal).ok_or(ProjectCheck::Unsupported)?;
                let guard = ranked_float::Guard::from_expression(claim, &axis.ty, |expr|
                    matches!(expr, ScalarExpression::Name { name, .. } if name == entry_name))
                    .ok_or(ProjectCheck::Unsupported)?;
                Ok(!path.keep(&guard, false, work)?.reachable())
            }
        }
    }

    fn transition(
        &self, entry: &Self, rank: TypedInductionRank<'_>, arguments: &[Coordinate<'_>], work: &mut usize,
    ) -> Result<InductionPremise, ProjectCheck> {
        match (self, entry) {
            (Self::Integers(path), Self::Integers(entry)) => transition(rank, entry, path, arguments, work),
            (Self::Float(path), Self::Float(entry)) => match arguments {
                [_] if !path.reachable() => Ok(InductionPremise::Unreachable),
                [Coordinate::Entry] => Ok(InductionPremise::NotStrict),
                [Coordinate::FloatLiteral(value)] => path.literal_transition(entry, *value, work),
                [Coordinate::FloatOffset { add, magnitude }] => path.offset_transition(entry, *magnitude, *add, work),
                _ => Err(ProjectCheck::Unsupported),
            },
            _ => Err(ProjectCheck::Unsupported),
        }
    }
}

/// Captures only authenticated runtime operands, never a proved result.
pub(super) struct CappedTailRecipe {
    pub(super) target: PreparedCallable,
    pub(super) site: OccurrenceOrdinal,
    pub(super) forwarder: Option<(PreparedCallable, OccurrenceOrdinal)>,
    operations: Vec<IntegerTailOperand>,
    rank_axes: Vec<usize>,
    domains: Vec<IntegerSet>,
}

enum IntegerTailOperand { Entry, Offset(bool, BigInt), Literal(BigInt) }

impl CappedTailRecipe {
    pub(super) fn work(&self) -> Option<usize> {
        self.domains.iter().try_fold(self.operations.len(), |n, d| n.checked_add(d.interval_count().checked_add(1)?))
    }

    pub(super) fn accepts(&self, tuple: &[&BigInt]) -> bool {
        self.domains.len() == tuple.len() && self.domains.iter().zip(tuple).all(|(domain, v)| domain.contains(v))
    }

    pub(super) fn compute(
        &self, body: &FunctionCallSites, cursor: &ExactCallCursor<'_>, types: &[ScalarType],
    ) -> Option<Vec<BigInt>> {
        let bindings = body.boundary.checker_parameter_bindings.as_deref()?;
        if self.operations.len() != types.len() || bindings.len() != types.len() || body.boundary.parameters.len() != types.len() {
            return None;
        }
        let old = types.iter().zip(bindings).zip(&body.boundary.parameters).map(|((ty, binding), slot)| {
            if &slot.ty != ty || !is_integer_type(ty) { return None; }
            cursor.contextual_singleton(binding, ty)
        }).collect::<Option<Vec<_>>>()?;
        if !self.accepts(&old) { return None; }
        let next = self.operations.iter().zip(&old).zip(types).map(|((op, &value), ty)| {
            let new = match op {
                IntegerTailOperand::Entry => value.clone(),
                IntegerTailOperand::Offset(true, amount) => value + amount,
                IntegerTailOperand::Offset(false, amount) => value - amount,
                IntegerTailOperand::Literal(value) => value.clone(),
            };
            integer_literal_fits_type(&new, ty).then_some(new)
        }).collect::<Option<Vec<_>>>()?;
        let strict = self.rank_axes.iter().find_map(|&slot| {
            let before = *old.get(slot)?;
            let after = next.get(slot)?;
            (before != after).then(|| after.sign() != num_bigint::Sign::Minus && after < before)
        });
        (strict == Some(true) && self.domains.iter().zip(&next).all(|(domain, value)| domain.contains(value))).then_some(next)
    }
}

pub(super) fn verify_capped_wide<'a>(
    plan: &'a PlannedModuleProof<'_>, rank: TypedInductionRank<'_>,
) -> Result<CappedTailRecipe, ProjectCheck> {
    if !(3..=MAX_RANKED_INTEGER_AXES).contains(&rank.entries().count()) || rank.float_entry().is_some() || rank.sum {
        return Err(ProjectCheck::Unsupported);
    }
    let mut captured = None;
    if !matches!(verify_impl(plan, rank, Some(&mut captured))?, RankedInductionOutcome::Premise(InductionPremise::Established)) {
        return Err(ProjectCheck::Unsupported);
    }
    captured.ok_or(ProjectCheck::Unsupported)
}

pub(super) fn verify<'a>(
    plan: &'a PlannedModuleProof<'_>, rank: TypedInductionRank<'_>,
) -> Result<RankedInductionOutcome<'a>, ProjectCheck> {
    verify_impl(plan, rank, None)
}

fn verify_impl<'a>(
    plan: &'a PlannedModuleProof<'_>, rank: TypedInductionRank<'_>,
    mut capture: Option<&mut Option<CappedTailRecipe>>,
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
    let entry = if let Some((slot, _)) = rank.float_entry() {
        RankLane::Float(ranked_float::guarded_entry(plan, &slot.ty, &mut work)?)
    } else {
        let entry = Rectangle::entry(rank, plan.filter.as_deref().unwrap_or_default(), &mut work)?;
        for axis in &entry.0 { charge(&mut work, axis.interval_count())?; }
        RankLane::Integers(entry)
    };
    let mut pending = vec![(plan.steps.as_ref(), &plan.function.body, structure.root, entry.clone())];
    let mut matched_calls = 0;
    let mut has_base = false;
    let mut failure = None;
    let mut premise = InductionPremise::Unreachable;
    while let Some((proof, runtime, block, path)) = pending.pop() {
        charge(&mut work, 1)?;
        let output = ranked_output_at(body, block, typed.result_type).ok_or(ProjectCheck::Unsupported)?;
        if proof.items.len() != proof.terminated_items.len() { return Err(ProjectCheck::Unsupported); }
        let item = proof.items.first().ok_or(ProjectCheck::Unsupported)?;
        if let Some(ConditionalRegion::Value { condition, then_block, else_block, .. }) = body.conditionals.get(&output) {
            let branches = runtime.tail_conditional().ok_or(ProjectCheck::Unsupported)?;
            if body.occurrences.get(output.0).map(|node| node.span) != Some(branches.span) {
                return Err(ProjectCheck::Unsupported);
            }
            let (yes_proof, no_proof, proof_condition) = match item {
                ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. })
                    if proof.items.len() == 1 => (then_branch, else_branch, Some(condition.as_ref())),
                // An unbranched show is an obligation on every actual returning leaf.
                ScalarBlockItem::Expression(ScalarExpression::Call { receiver: Some(core), name, .. })
                    if core == "core" && name == "show" => (proof, proof, None),
                _ => return Err(ProjectCheck::Unsupported),
            };
            let (yes, no) = match path {
                RankLane::Integers(path) => {
                    let guard = structure.checker_branches.get(condition).ok_or(ProjectCheck::Unsupported)?;
                    if let Some(proof_condition) = proof_condition {
                        if &proof_input_condition::<true>(proof_condition, &typed.input_names, body, false)? != guard {
                            return Err(ProjectCheck::Unsupported);
                        }
                    }
                    let (yes, no) = path.split(rank, guard, &mut work)?;
                    (RankLane::Integers(yes), RankLane::Integers(no))
                }
                RankLane::Float(points) => {
                    let (axis, binding) = rank.float_entry().ok_or(ProjectCheck::Unsupported)?;
                    let guard = ranked_float::Guard::checked_runtime(body, *condition, branches.condition, &axis.ty, binding)
                        .ok_or(ProjectCheck::Unsupported)?;
                    if let Some(proof_condition) = proof_condition {
                        let authored = ranked_float::Guard::from_expression(proof_condition, &axis.ty, |expr|
                            matches!(expr, ScalarExpression::Name { name, .. } if name == &typed.input_names[axis.ordinal]))
                            .ok_or(ProjectCheck::Unsupported)?;
                        if authored != guard { return Err(ProjectCheck::Unsupported); }
                    }
                    let yes = points.keep(&guard, true, &mut work)?;
                    let no = points.keep(&guard, false, &mut work)?;
                    (RankLane::Float(yes), RankLane::Float(no))
                }
            };
            pending.push((no_proof, branches.else_branch, *else_block, no));
            pending.push((yes_proof, branches.then_branch, *then_block, yes));
        } else if let Some(value) = structure.checker_integer_literals.get(&output) {
            has_base = true;
            charge(&mut work, proof.items.len())?;
            let mut rejected = None;
            let mut rewritten = None;
            let mut literal_equality_shown = false;
            let equality = IntegerRelation::from_operator(&BinaryOperator::Equal).ok_or(ProjectCheck::Unsupported)?;
            for (item, &terminated) in proof.items.iter().zip(&proof.terminated_items) {
                if let Some((premise, context)) = rewrite::arguments(item) {
                    // A matcher visits only the two typed terms and the single
                    // hole in a bounded binary equality/comparison.
                    charge(&mut work, 6)?;
                    if !terminated { return Err(ProjectCheck::Unsupported); }
                    // A prior refutation is already the first failed step.
                    // It is not a premise and we must not derive past it.
                    if rejected.is_some() && path.reachable() { break; }
                    let TypedProofGoal::Literal { operator, bound, .. } = &typed.goal else {
                        return Err(ProjectCheck::Unsupported);
                    };
                    let goal = rewritten.get_or_insert_with(|| rewrite::Goal::new(operator, bound));
                    goal.apply(premise, context, &typed.output, typed.result_type, None, |premise| {
                        Ok(matches!(premise, rewrite::Premise::Literal(shown) if literal_equality_shown && shown == value))
                    })?;
                    continue;
                }
                let claim = IntegerComparison::from_expression(proof_show_expression(item, terminated)?)
                    .ok_or(ProjectCheck::Unsupported)?;
                let (_, relation, bound) = checked_literal_output_goal_borrowed(
                    claim, &typed.output, &typed.input_names, typed.result_type,
                )?;
                if relation.accepts(value.cmp(&bound)) {
                    literal_equality_shown |= relation == equality;
                } else if rejected.is_none() {
                    let ScalarBlockItem::Expression(show) = item else { return Err(ProjectCheck::Unsupported); };
                    rejected = Some(RankedBaseRejection::AuthoredShow(expression_span(show)));
                }
            }
            if failure.is_none() && path.reachable() {
                if rejected.is_none() {
                    let accepted = match rewritten {
                        Some(goal) => goal.accepts_literal_output(value)?,
                        None => relation.accepts(value.cmp(bound)),
                    };
                    if !accepted {
                        let goal = child(&child(plan.proof, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
                        rejected = Some(RankedBaseRejection::Goal(wosy_syntax::byte_span(&goal)));
                    }
                }
                let Some(rejected) = rejected else { continue; };
                let entry = match path {
                    RankLane::Integers(path) => {
                        charge(&mut work, path.0.len())?;
                        let coordinates = path.0.iter().map(|axis| axis.extremes()
                            .map(|(first, _)| first.clone()).ok_or(ProjectCheck::Unsupported))
                            .collect::<Result<Vec<_>, _>>()?;
                        RankedBaseEntry::Integer(coordinates)
                    }
                    RankLane::Float(path) => RankedBaseEntry::Ieee(path.first(&mut work)?),
                };
                failure = Some(RankedBaseWitness {
                    proof_source: plan.proof_source, return_source: &typed.target.source,
                    entry, returned: value,
                    return_span: body.occurrences.get(output.0).ok_or(ProjectCheck::Unsupported)?.span, rejected,
                });
            }
        } else {
            charge(&mut work, proof.items.len())?;
            let (mut steps, mut terminations) = (proof.items.as_slice(), proof.terminated_items.as_slice());
            let (last, before) = steps.split_last().ok_or(ProjectCheck::Unsupported)?;
            let (&terminated, earlier) = terminations.split_last().ok_or(ProjectCheck::Unsupported)?;
            let rewrite = rewrite::arguments(last);
            if rewrite.is_some() {
                charge(&mut work, 6)?;
                if !terminated { return Err(ProjectCheck::Unsupported); }
                steps = before;
                terminations = earlier;
            }
            let (self_step, prefix) = steps.split_last().ok_or(ProjectCheck::Unsupported)?;
            let (&self_terminated, prefix_terminations) = terminations.split_last().ok_or(ProjectCheck::Unsupported)?;
            for (show, &terminated) in prefix.iter().zip(prefix_terminations) {
                let claim = proof_show_expression(show, terminated)?;
                if !path.proves_entry_show(claim, typed, rank, &mut work)? {
                    // No actual returned value is known at this recursive leaf.
                    return Err(ProjectCheck::Unsupported);
                }
            }
            let args = self_use(plan.name, self_step, self_terminated, typed, rank)?;
            check_tail(plan, rank, output, runtime, &args)?;
            matched_calls += 1;
            let next = path.transition(&entry, rank, &args, &mut work)?;
            if let Some(capture) = &mut capture {
                let RankLane::Integers(entry) = &entry else { return Err(ProjectCheck::Unsupported); };
                if capture.is_some() { return Err(ProjectCheck::Unsupported); }
                charge(&mut work, args.len())?;
                for domain in &entry.0 { charge(&mut work, domain.interval_count())?; }
                let operations = args.iter().map(|op| Ok(match op {
                    Coordinate::Entry => IntegerTailOperand::Entry,
                    Coordinate::Offset { add, magnitude } => IntegerTailOperand::Offset(*add, (*magnitude).clone()),
                    Coordinate::Literal(v) => IntegerTailOperand::Literal(v.clone().into_owned()),
                    _ => return Err(ProjectCheck::Unsupported),
                })).collect::<Result<Vec<_>, _>>()?;
                **capture = Some(CappedTailRecipe { target: plan.target, site: output, operations, forwarder: None,
                    rank_axes: rank.axes().map(|(axis, _)| axis.ordinal).collect(), domains: entry.0.clone() });
            }
            if let Some((equation, context)) = rewrite {
                let TypedProofGoal::Literal { operator: operator @ BinaryOperator::Equal, bound, .. } = &typed.goal
                    else { return Err(ProjectCheck::Unsupported); };
                if !matches!(next, InductionPremise::Established) { return Err(ProjectCheck::Unsupported); }
                // This output is the exact authenticated smaller call's return.
                // Only its original induction equality is available here; no
                // actual value is produced, cached or reported by this step.
                let mut goal = rewrite::Goal::new(operator, bound);
                goal.apply(equation, context, &typed.output, typed.result_type, None, |premise| {
                    Ok(matches!(premise, rewrite::Premise::Literal(value) if value == bound))
                })?;
                if !goal.accepts_literal_output(bound)? { return Err(ProjectCheck::InvalidProof); }
            }
            match next {
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
    let ([item], [terminated]) = (else_branch.items.as_slice(), else_branch.terminated_items.as_slice())
        else { return Err(ProjectCheck::Unsupported); };
    let args = self_use(plan.name, item, *terminated, &plan.typed, rank)?;
    transition(rank, &entry, &recursive, &args, &mut work)
}
