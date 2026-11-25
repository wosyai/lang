//! Small explicitly bounded IEEE rank lanes; used by the same prepared
//! ranked return-tree walk as the integer rectangular lane.
use super::*;
use std::cmp::Ordering;

// An authored guard may cover only this many *representations*, not real
// numbers. The checker also charges the shared ranked work budget.
const MAX_POINTS: u64 = 1_024;

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

    pub(super) fn accepts(&self, point: Atom) -> bool {
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

pub(super) fn guarded_entry(plan: &PlannedModuleProof<'_>, ty: &ScalarType, work: &mut usize) -> Result<Vec<Atom>, ProjectCheck> {
    let charge = |work: &mut usize, units| super::super::checker_flow::charge_call_work(work, units).ok_or(ProjectCheck::Unsupported);
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
    let mut atoms = Vec::new();
    let mut stack = vec![&guard];
    let (mut lower, mut upper) = (None::<u64>, None::<u64>);
    let mut sample = None;
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
        if matches!(atom, Guard::Boolean(false)) { return Ok(Vec::new()); }
        if let Guard::Cut(relation, boundary) = atom {
            sample = Some(boundary);
            if !relation.accepts(Ordering::Less) {
                let key = boundary.range_key(true);
                lower = Some(lower.map_or(key, |old| old.max(key)));
            }
            if !relation.accepts(Ordering::Greater) {
                let key = boundary.range_key(false);
                upper = Some(upper.map_or(key, |old| old.min(key)));
            }
            atoms.push(Guard::Cut(relation, boundary));
        }
    }
    // Both finite bounds are required: an unbounded or unordered domain is
    // never silently narrowed by the checker.
    let (Some(lower), Some(upper), Some(sample)) = (lower, upper, sample) else { return Err(ProjectCheck::Unsupported); };
    if upper < lower { return Ok(Vec::new()); }
    let count = upper.checked_sub(lower).and_then(|n| n.checked_add(1))
        .filter(|count| *count <= MAX_POINTS).ok_or(ProjectCheck::Unsupported)?;
    charge(work, (count as usize).checked_mul(atoms.len().saturating_add(1)).ok_or(ProjectCheck::Unsupported)?)?;
    let mut points = Vec::with_capacity(count as usize);
    for key in lower..=upper {
        let point = sample.with_key(key).filter(|point| point.finite()).ok_or(ProjectCheck::Unsupported)?;
        if atoms.iter().all(|atom| atom.accepts(point)) { points.push(point); }
    }
    Ok(points)
}

pub(super) fn contains(points: &[Atom], atom: Atom) -> bool {
    points.binary_search_by_key(&atom.key(), |point| point.key()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::Atom;

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
