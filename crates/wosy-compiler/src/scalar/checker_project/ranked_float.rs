//! Explicitly bounded finite IEEE key unions; reuse the same prepared
//! ranked return-tree walk as the integer rectangular lane.
use super::*;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Atom {
    Single(u32),
    Double(u64),
}

impl Atom {
    pub(super) fn literal(expr: &ScalarExpression, ty: &ScalarType) -> Option<Self> {
        let value = match expr {
            ScalarExpression::Float { value, .. } => *value,
            ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } => {
                let ScalarExpression::Float { value, .. } = operand.as_ref() else { return None; };
                -*value
            }
            _ => return None,
        };
        // Matches LLVM's source-f64 constant conversion to the checked type.
        let rounded = match ty {
            ScalarType::F32 => Self::Single((value as f32).to_bits()),
            ScalarType::F64 => Self::Double(value.to_bits()),
            _ => return None,
        };
        rounded.finite().then_some(rounded)
    }

    fn finite(self) -> bool {
        match self {
            Self::Single(bits) => f32::from_bits(bits).is_finite(),
            Self::Double(bits) => f64::from_bits(bits).is_finite(),
        }
    }

    fn compare(self, other: Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Single(a), Self::Single(b)) => f32::from_bits(a).partial_cmp(&f32::from_bits(b)),
            (Self::Double(a), Self::Double(b)) => f64::from_bits(a).partial_cmp(&f64::from_bits(b)),
            _ => None,
        }
    }

    pub(super) fn offset(self, rhs: Self, add: bool) -> Option<Self> {
        let result = match (self, rhs) {
            (Self::Single(a), Self::Single(b)) => {
                let (a, b) = (f32::from_bits(a), f32::from_bits(b));
                Self::Single(if add { a + b } else { a - b }.to_bits())
            }
            (Self::Double(a), Self::Double(b)) => {
                let (a, b) = (f64::from_bits(a), f64::from_bits(b));
                Self::Double(if add { a + b } else { a - b }.to_bits())
            }
            _ => return None,
        };
        result.finite().then_some(result)
    }

    pub(super) fn strictly_less(self, current: Self) -> bool {
        self.finite() && current.finite() && self.compare(current) == Some(Ordering::Less)
    }

    fn key(self) -> u64 {
        match self {
            Self::Single(bits) => u64::from(if bits >> 31 == 0 { bits ^ (1 << 31) } else { !bits }),
            Self::Double(bits) => if bits >> 63 == 0 { bits ^ (1 << 63) } else { !bits },
        }
    }

    fn with_key(self, key: u64) -> Option<Self> {
        Some(match self {
            Self::Single(_) => {
                let key = u32::try_from(key).ok()?;
                Self::Single(if key >> 31 == 0 { !key } else { key ^ (1 << 31) })
            }
            Self::Double(_) => Self::Double(if key >> 63 == 0 { !key } else { key ^ (1 << 63) }),
        })
    }

    fn zero(self, negative: bool) -> Self {
        match self {
            Self::Single(_) => Self::Single(if negative { (-0.0f32).to_bits() } else { 0 }),
            Self::Double(_) => Self::Double(if negative { (-0.0f64).to_bits() } else { 0 }),
        }
    }

    fn spacing_segment_end(self) -> u64 {
        let key = self.key();
        let (fraction_mask, positive) = match self {
            Self::Single(bits) => ((1 << 23) - 1, bits >> 31 == 0),
            Self::Double(bits) => ((1 << 52) - 1, bits >> 63 == 0),
        };
        // A positive power of two has the predecessor spacing of the
        // *previous* bin. Treat it and both signed zeros as singletons.
        if self == self.zero(false) || self == self.zero(true) || (positive && key & fraction_mask == 0) {
            return key;
        }
        let end = key | fraction_mask;
        if !positive { end.min(self.zero(true).key() - 1) } else { end }
    }

    fn range_key(self, lower: bool) -> u64 {
        // Numeric comparisons equate +0 and -0, but the actual operations
        // still receive each representation.
        if self.compare(self.zero(false)) == Some(Ordering::Equal) {
            self.zero(lower).key()
        } else {
            self.key()
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum Guard {
    Boolean(bool),
    Cut(IntegerRelation, Atom),
}

impl Guard {
    pub(super) fn from_expression(
        expr: &ScalarExpression, ty: &ScalarType, is_entry: impl Fn(&ScalarExpression) -> bool,
    ) -> Option<Self> {
        if let ScalarExpression::Boolean { value, .. } = expr { return Some(Self::Boolean(*value)); }
        let ScalarExpression::Binary { operator, left, right, .. } = expr else { return None; };
        if !is_entry(left) { return None; }
        Some(Self::Cut(IntegerRelation::from_operator(operator)?, Atom::literal(right, ty)?))
    }

    #[cfg(test)]
    fn accepts(&self, point: Atom) -> bool {
        match self {
            Self::Boolean(value) => *value,
            Self::Cut(relation, cut) => point.compare(*cut).is_some_and(|order| relation.accepts(order)),
        }
    }

    pub(super) fn checked_runtime(
        calls: &FunctionCallSites, ordinal: OccurrenceOrdinal, expression: &ScalarExpression,
        ty: &ScalarType, entry: &ReferenceBindingId,
    ) -> Option<Self> {
        let structure = &calls.structure;
        let checked_occurrence = |node: OccurrenceOrdinal, expected: &ScalarType, expr: &ScalarExpression|
            structure.checked_types.expressions.get(&node) == Some(expected)
                && calls.occurrences.get(node.0).is_some_and(|node| node.span == super::super::typecheck::expression_span(expr));
        if !checked_occurrence(ordinal, &ScalarType::Bool, expression) { return None; }
        if let ScalarExpression::Binary { left, right, .. } = expression {
            let parent = super::super::reference::OccurrenceParent::Expression(ordinal);
            use super::super::reference::{OccurrenceEdge, OccurrenceNode, PlainNameIdentity};
            let Some(OccurrenceNode::Expression(lhs)) = structure.edges.get(&(parent, OccurrenceEdge::BinaryLeft)) else { return None; };
            let Some(OccurrenceNode::Expression(rhs)) = structure.edges.get(&(parent, OccurrenceEdge::BinaryRight)) else { return None; };
            if !checked_occurrence(*lhs, ty, left) || !checked_occurrence(*rhs, ty, right)
                || !matches!(calls.prepared_plain_name(*lhs), Some(PlainNameIdentity::Lexical(binding)) if binding == entry)
            {
                return None;
            }
        }
        Self::from_expression(expression, ty, |expr| matches!(expr, ScalarExpression::Name { .. }))
    }
}

#[derive(Clone)]
pub(super) struct Lane {
    sample: Atom,
    keys: Vec<(u64, u64)>,
}

fn charge(work: &mut usize, units: usize) -> Result<(), ProjectCheck> {
    super::super::checker_flow::charge_call_work(work, units).ok_or(ProjectCheck::Unsupported)
}

impl Lane {
    fn atom(&self, key: u64) -> Result<Atom, ProjectCheck> {
        self.sample.with_key(key).filter(|point| point.finite()).ok_or(ProjectCheck::Unsupported)
    }

    pub(super) fn reachable(&self) -> bool { !self.keys.is_empty() }

    pub(super) fn keep(&self, guard: &Guard, yes: bool, work: &mut usize) -> Result<Self, ProjectCheck> {
        let mut keys: Vec<(u64, u64)> = Vec::new();
        charge(work, self.keys.len().checked_mul(4).ok_or(ProjectCheck::Unsupported)?)?;
        let mut push = |lo: u64, hi: u64| {
            if lo > hi { return; }
            if let Some(last) = keys.last_mut().filter(|last| last.1.checked_add(1) == Some(lo)) {
                last.1 = hi;
            } else {
                keys.push((lo, hi));
            }
        };
        for &(lo, hi) in &self.keys {
            match guard {
                Guard::Boolean(value) if *value == yes => push(lo, hi),
                Guard::Boolean(_) => {}
                Guard::Cut(relation, cut) => {
                    let low = cut.range_key(true);
                    let high = cut.range_key(false);
                    if lo < low && relation.accepts(Ordering::Less) == yes { push(lo, hi.min(low - 1)); }
                    if relation.accepts(Ordering::Equal) == yes { push(lo.max(low), hi.min(high)); }
                    if hi > high && relation.accepts(Ordering::Greater) == yes { push(lo.max(high + 1), hi); }
                }
            }
        }
        Ok(Self { sample: self.sample, keys })
    }

    pub(super) fn literal_transition(
        &self, entry: &Self, successor: Atom, work: &mut usize,
    ) -> Result<InductionPremise, ProjectCheck> {
        let Some(&(lowest, _)) = self.keys.first() else { return Ok(InductionPremise::Unreachable); };
        if !successor.strictly_less(self.atom(lowest)?) { return Ok(InductionPremise::NotStrict); }
        if !entry.contains(successor.key(), work)? { return Ok(InductionPremise::CalleeWhereUnproved); }
        Ok(InductionPremise::Established)
    }

    fn contains(&self, key: u64, work: &mut usize) -> Result<bool, ProjectCheck> {
        charge(work, (usize::BITS - self.keys.len().leading_zeros()) as usize + 1)?;
        let pos = self.keys.partition_point(|&(lo, _)| lo <= key);
        Ok(pos > 0 && key <= self.keys[pos - 1].1)
    }

    pub(super) fn offset_transition(
        &self, entry: &Self, magnitude: Atom, add: bool, work: &mut usize,
    ) -> Result<InductionPremise, ProjectCheck> {
        if !self.reachable() { return Ok(InductionPremise::Unreachable); }
        let shift = |key| Ok(self.atom(key)?.offset(magnitude, add));
        for &(lo, hi) in &self.keys {
            charge(work, 2)?;
            // Same-width rounding of a fixed translation is monotone. For
            // a strict decrease the literal displacement is nonzero; then
            // both input zeros map to the same nonzero result. Numeric
            // monotonicity therefore also preserves IEEE signed-zero keys.
            let (Some(first), Some(last)) = (shift(lo)?, shift(hi)?) else { return Ok(InductionPremise::NotStrict); };
            let mut at = lo;
            loop {
                charge(work, 3)?;
                let representative = self.atom(at)?;
                let end = hi.min(representative.spacing_segment_end());
                // Within one predecessor-spacing region, x rounds back
                // to itself iff the decrement is smaller than half the
                // predecessor gap, or is exactly half and x is even.
                // Two consecutive bit keys exercise both parities with
                // the very same target-width operation (no real widening).
                if !representative.offset(magnitude, add).is_some_and(|next| next.strictly_less(representative)) {
                    return Ok(InductionPremise::NotStrict);
                }
                if at < end {
                    let opposite = self.atom(at + 1)?;
                    if !opposite.offset(magnitude, add).is_some_and(|next| next.strictly_less(opposite)) {
                        return Ok(InductionPremise::NotStrict);
                    }
                }
                if end == hi { break; }
                at = end + 1;
            }
            let (first, last) = (first.key(), last.key());
            if first > last || !entry.contains(first, work)? || !entry.contains(last, work)? {
                return Ok(InductionPremise::CalleeWhereUnproved);
            }
            for hole in entry.keys.windows(2) {
                charge(work, 1)?;
                let (hole_lo, hole_hi) = (hole[0].1 + 1, hole[1].0 - 1);
                if hole_hi < first || last < hole_lo { continue; }
                // Endpoints alone cannot decide closure for a gapped guard.
                // Find first source key whose actual rounded result is at
                // least hole_lo. A hole jumped over by rounding is harmless.
                let (mut l, mut r) = (lo, hi);
                while l < r {
                    charge(work, 1)?;
                    let mid = l + (r - l) / 2;
                    let output = shift(mid)?.ok_or(ProjectCheck::Unsupported)?.key();
                    if output < hole_lo { l = mid + 1; } else { r = mid; }
                }
                charge(work, 1)?;
                if shift(l)?.ok_or(ProjectCheck::Unsupported)?.key() <= hole_hi {
                    return Ok(InductionPremise::CalleeWhereUnproved);
                }
            }
        }
        Ok(InductionPremise::Established)
    }
}

pub(super) fn guarded_entry(plan: &PlannedModuleProof<'_>, ty: &ScalarType, work: &mut usize) -> Result<Lane, ProjectCheck> {
    let Some(where_node) = plan.proof.children().find(|node| node.kind() == SyntaxKind::ProofWhere) else {
        return Err(ProjectCheck::Unsupported);
    };
    if where_node.descendants().take(MAX_ACTUAL_CALL_DEPTH + 1).count() > MAX_ACTUAL_CALL_DEPTH {
        return Err(ProjectCheck::Unsupported);
    }
    let typed = &plan.typed;
    let guard = typed_proof_where(plan.proof, plan.proof_source, typed.target.target_layout,
        &typed.input_names, typed.body)?.ok_or(ProjectCheck::Unsupported)?;
    let [entry] = typed.input_names.as_slice() else { return Err(ProjectCheck::Unsupported); };
    let (sample, low, high) = match ty {
        ScalarType::F32 => (Atom::Single(0), Atom::Single((-f32::MAX).to_bits()), Atom::Single(f32::MAX.to_bits())),
        ScalarType::F64 => (Atom::Double(0), Atom::Double((-f64::MAX).to_bits()), Atom::Double(f64::MAX.to_bits())),
        _ => return Err(ProjectCheck::Unsupported),
    };
    let mut lane = Lane { sample, keys: vec![(low.key(), high.key())] };
    let mut stack = vec![&guard];
    let (mut lower, mut upper) = (false, false);
    while let Some(node) = stack.pop() {
        charge(work, 1)?;
        if let ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } = node {
            stack.push(right);
            stack.push(left);
            continue;
        }
        let atom = Guard::from_expression(node, ty, |expr|
            matches!(expr, ScalarExpression::Name { name, .. } if name == entry))
            .ok_or(ProjectCheck::Unsupported)?;
        if matches!(atom, Guard::Boolean(false)) { return Ok(Lane { sample, keys: Vec::new() }); }
        if let Guard::Cut(relation, _) = atom {
            lower |= !relation.accepts(Ordering::Less);
            upper |= !relation.accepts(Ordering::Greater);
        }
        lane = lane.keep(&atom, true, work)?;
    }
    // Both finite bounds are required: an unbounded or unordered domain is
    // never silently narrowed by the checker.
    if !lower || !upper { return Err(ProjectCheck::Unsupported); }
    charge(work, lane.keys.len())?;
    Ok(lane)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(first: Atom, last: Atom) -> Lane {
        Lane { sample: first, keys: vec![(first.key(), last.key())] }
    }

    #[test]
    fn ieee_intervals_match_the_point_oracle_through_spacing_edges() {
        let centers = [
            Atom::Single((-f32::MAX).to_bits()), Atom::Single((-2.0f32).to_bits()),
            Atom::Single((-f32::MIN_POSITIVE).to_bits()), Atom::Single((-0.0f32).to_bits()),
            Atom::Single(f32::MIN_POSITIVE.to_bits()), Atom::Single(1.0f32.to_bits()),
            Atom::Single(2.0f32.to_bits()), Atom::Single(f32::MAX.to_bits()),
            Atom::Double((-f64::MAX).to_bits()), Atom::Double((-2.0f64).to_bits()),
            Atom::Double((-f64::MIN_POSITIVE).to_bits()), Atom::Double((-0.0f64).to_bits()),
            Atom::Double(f64::MIN_POSITIVE.to_bits()), Atom::Double(1.0f64.to_bits()),
            Atom::Double(2.0f64.to_bits()), Atom::Double(f64::MAX.to_bits()),
        ];
        for center in centers {
            let (first, last, shifts) = match center {
                Atom::Single(_) => (
                    Atom::Single((-f32::MAX).to_bits()), Atom::Single(f32::MAX.to_bits()),
                    [Atom::Single(1), Atom::Single((f32::EPSILON / 2.0).to_bits()),
                     Atom::Single(f32::EPSILON.to_bits()), Atom::Single(1.0f32.to_bits())],
                ),
                Atom::Double(_) => (
                    Atom::Double((-f64::MAX).to_bits()), Atom::Double(f64::MAX.to_bits()),
                    [Atom::Double(1), Atom::Double((f64::EPSILON / 2.0).to_bits()),
                     Atom::Double(f64::EPSILON.to_bits()), Atom::Double(1.0f64.to_bits())],
                ),
            };
            let entire = lane(first, last);
            let lo = first.key().max(center.key().saturating_sub(4));
            let hi = last.key().min(center.key().saturating_add(4));
            for begin in lo..=hi {
                for end in begin..=hi {
                    let path = Lane { sample: center, keys: vec![(begin, end)] };
                    for shift in shifts {
                        for add in [false, true] {
                            let point_oracle = (begin..=end).all(|key| {
                                let point = center.with_key(key).unwrap();
                                point.offset(shift, add).is_some_and(|next| next.strictly_less(point))
                            });
                            let result = path.offset_transition(&entire, shift, add, &mut 0).unwrap();
                            assert_eq!(result == InductionPremise::Established, point_oracle,
                                "{center:?} {begin}..={end}, {shift:?}, add={add}");
                        }
                    }
                }
            }
            let guard = Guard::Cut(IntegerRelation::from_operator(&BinaryOperator::NotEqual).unwrap(), center);
            for yes in [false, true] {
                let clipped = Lane { sample: center, keys: vec![(lo, hi)] }.keep(&guard, yes, &mut 0).unwrap();
                assert_eq!(clipped.keys.iter().flat_map(|&(a, b)| a..=b).collect::<Vec<_>>(),
                    (lo..=hi).filter(|&key| guard.accepts(center.with_key(key).unwrap()) == yes).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn ieee_intervals_check_rounded_guard_holes_without_treating_image_as_contiguous() {
        let one = Atom::Single(1.0f32.to_bits());
        let two = Atom::Single(2.0f32.to_bits());
        let first = one.key();
        let start = two.key();
        let entry = Lane { sample: one, keys: vec![(first, first), (first + 4, first + 4), (start, start + 2)] };
        let path = Lane { sample: two, keys: vec![(start, start + 2)] };
        assert_eq!(path.offset_transition(&entry, one, false, &mut 0).unwrap(), InductionPremise::CalleeWhereUnproved);
        let entry = Lane { sample: one, keys: vec![(first, first), (first + 2, first + 2),
            (first + 4, first + 4), (start, start + 2)] };
        assert_eq!(path.offset_transition(&entry, one, false, &mut 0).unwrap(), InductionPremise::Established);
    }

    #[test]
    fn bounded_ieee_atoms_refuse_unordered_nonfinite_and_equal_rounding() {
        let one = Atom::Single(1.0f32.to_bits());
        let tiny = Atom::Single(1.0e-12f32.to_bits());
        let eps = Atom::Single(f32::EPSILON.to_bits());
        assert!(one.offset(eps, false).unwrap().strictly_less(one));
        assert!(!one.offset(tiny, false).unwrap().strictly_less(one));
        assert!(!Atom::Single(f32::NAN.to_bits()).strictly_less(one));
        assert!(!Atom::Single(f32::NEG_INFINITY.to_bits()).strictly_less(one));
        let max = Atom::Single(f32::MAX.to_bits());
        assert!(max.offset(max, true).is_none());
        let low_zero = Atom::Double((-0.0f64).to_bits());
        let high_zero = Atom::Double(0.0f64.to_bits());
        assert!(low_zero.key() < high_zero.key());
        assert!(!low_zero.strictly_less(high_zero));
        assert!(!high_zero.strictly_less(low_zero));
    }
}
