use std::cmp::Ordering;

use num_bigint::BigInt;

use super::{typecheck::integer_bounds, BinaryOperator, ScalarType};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Interval {
    first: BigInt,
    last: BigInt,
}

fn overlapping_or_adjacent(first: &BigInt, last: &BigInt) -> bool {
    first <= last || *first == last + 1
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct IntegerSet(Vec<Interval>);

#[derive(Debug, Eq, PartialEq)]
pub(super) enum IntegerIntersection {
    Unchanged,
    Empty,
    Changed(IntegerSet),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntegerRelation {
    less: bool,
    equal: bool,
    greater: bool,
}

impl IntegerRelation {
    pub(super) fn from_operator(operator: &BinaryOperator) -> Option<Self> {
        let (less, equal, greater) = match operator {
            BinaryOperator::Equal => (false, true, false),
            BinaryOperator::NotEqual => (true, false, true),
            BinaryOperator::Less => (true, false, false),
            BinaryOperator::LessEqual => (true, true, false),
            BinaryOperator::Greater => (false, false, true),
            BinaryOperator::GreaterEqual => (false, true, true),
            _ => return None,
        };
        Some(Self { less, equal, greater })
    }

    pub(super) fn complement(self) -> Self {
        Self { less: !self.less, equal: !self.equal, greater: !self.greater }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn reversed(self) -> Self {
        Self { less: self.greater, equal: self.equal, greater: self.less }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn intersection(self, other: Self) -> Self {
        Self { less: self.less && other.less, equal: self.equal && other.equal, greater: self.greater && other.greater }
    }

    pub(super) fn accepts_closed(self, lower: &BigInt, upper: &BigInt, bound: &BigInt) -> bool {
        (self.less || lower >= bound)
            && (self.equal || bound < lower || bound > upper)
            && (self.greater || upper <= bound)
    }

    pub(super) fn accepts(self, comparison: Ordering) -> bool {
        match comparison {
            Ordering::Less => self.less,
            Ordering::Equal => self.equal,
            Ordering::Greater => self.greater,
        }
    }
}

impl IntegerSet {
    pub(super) fn from_type(ty: &ScalarType) -> Option<Self> {
        let (first, last) = integer_bounds(ty)?;
        Some(Self(vec![Interval { first, last }]))
    }

    pub(super) fn singleton(value: &BigInt) -> Self {
        Self(vec![Interval { first: value.clone(), last: value.clone() }])
    }

    pub(super) fn singleton_value(&self) -> Option<&BigInt> {
        match self.0.as_slice() {
            [interval] if interval.first == interval.last => Some(&interval.first),
            _ => None,
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn intervals(&self) -> impl ExactSizeIterator<Item = (&BigInt, &BigInt)> {
        self.0.iter().map(|interval| (&interval.first, &interval.last))
    }

    pub(super) fn extremes(&self) -> Option<(&BigInt, &BigInt)> {
        Some((&self.0.first()?.first, &self.0.last()?.last))
    }

    pub(super) fn contains(&self, value: &BigInt) -> bool {
        self.0.iter().any(|interval| &interval.first <= value && value <= &interval.last)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn covers_closed(&self, lower: &BigInt, upper: &BigInt) -> bool {
        let next = self.0.partition_point(|range| &range.last < lower);
        self.0.get(next).is_some_and(|range| &range.first <= lower && upper <= &range.last)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn is_subset_of(&self, allowed: &Self) -> bool {
        let mut enclosing = allowed.0.iter().peekable();
        self.0.iter().all(|range| {
            while enclosing.peek().is_some_and(|allowed| allowed.last < range.first) {
                enclosing.next();
            }
            enclosing.peek().is_some_and(|allowed| allowed.first <= range.first && allowed.last >= range.last)
        })
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn every_at_extremes(&self, relation: IntegerRelation, bound: &BigInt) -> bool {
        let Some((lower, upper)) = self.extremes() else { return true; };
        (relation.less || lower >= bound)
            && (relation.greater || upper <= bound)
            && (relation.equal || bound < lower || bound > upper || !self.covers_closed(bound, bound))
    }

    pub(super) fn every(&self, relation: IntegerRelation, bound: &BigInt) -> bool {
        self.0.iter().all(|interval| relation.accepts_closed(&interval.first, &interval.last, bound))
    }

    pub(super) fn split(&self, operator: &BinaryOperator, bound: &BigInt) -> Option<(Self, Self)> {
        Some(self.partition::<true>(IntegerRelation::from_operator(operator)?, bound))
    }

    pub(super) fn keep(&self, operator: &BinaryOperator, bound: &BigInt) -> Option<Self> {
        Some(self.keep_relation(IntegerRelation::from_operator(operator)?, bound))
    }

    pub(super) fn comparison_union(
        ty: &ScalarType, left: (IntegerRelation, &BigInt), right: (IntegerRelation, &BigInt),
    ) -> Option<Self> {
        let domain = Self::from_type(ty)?;
        Some(domain.keep_relation(left.0, left.1).join(domain.keep_relation(right.0, right.1)))
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn comparison_alternatives<'a>(
        ty: &ScalarType, clauses: impl IntoIterator<Item = &'a [(IntegerRelation, BigInt)]>,
    ) -> Option<Self> {
        let (minimum, maximum) = integer_bounds(ty)?;
        let mut intervals = Vec::new();
        for clause in clauses {
            Self::append_comparison_conjunction(&mut intervals, &minimum, &maximum, clause);
        }
        intervals.sort_unstable_by(|left, right| left.first.cmp(&right.first));
        intervals.dedup_by(|following, prior| {
            if following.last <= prior.last {
                return true;
            }
            if overlapping_or_adjacent(&following.first, &prior.last) {
                std::mem::swap(&mut prior.last, &mut following.last);
                true
            } else {
                false
            }
        });
        Some(Self(intervals))
    }

    #[cfg(feature = "internal-checker")]
    fn append_comparison_conjunction(
        intervals: &mut Vec<Interval>, minimum: &BigInt, maximum: &BigInt,
        atoms: &[(IntegerRelation, BigInt)],
    ) {
        let mut lower = (minimum, true);
        let mut upper = (maximum, true);
        let mut has_holes = false;
        for (relation, bound) in atoms {
            if !relation.less && !relation.equal && !relation.greater {
                return;
            }
            if !relation.less && (bound > lower.0 || (bound == lower.0 && !relation.equal)) {
                lower = (bound, relation.equal);
            }
            if !relation.greater && (bound < upper.0 || (bound == upper.0 && !relation.equal)) {
                upper = (bound, relation.equal);
            }
            has_holes |= relation.less && relation.greater && !relation.equal;
        }
        let mut start = if lower.1 { lower.0.clone() } else { lower.0 + 1 };
        let end = if upper.1 { upper.0.clone() } else { upper.0 - 1 };
        if start > end {
            return;
        }
        if has_holes {
            for (relation, hole) in atoms {
                if !relation.less || !relation.greater || relation.equal || hole < &start {
                    continue;
                }
                if hole > &end {
                    break;
                }
                if hole > &start {
                    intervals.push(Interval { first: start, last: hole - 1 });
                }
                start = hole + 1;
            }
        }
        if start <= end {
            intervals.push(Interval { first: start, last: end });
        }
    }

    pub(super) fn keep_relation(&self, relation: IntegerRelation, bound: &BigInt) -> Self {
        self.partition::<false>(relation, bound).0
    }

    fn partition<const KEEP_COMPLEMENT: bool>(&self, relation: IntegerRelation, bound: &BigInt) -> (Self, Self) {
        let mut yes = Self::default();
        let mut no = Self::default();
        let mut append = |selected, first, last| {
            if selected {
                yes.append(Interval { first, last });
            } else {
                no.append(Interval { first, last });
            }
        };
        for interval in &self.0 {
            if (KEEP_COMPLEMENT || relation.less) && &interval.first < bound {
                let last = if &interval.last < bound {
                    interval.last.clone()
                } else {
                    bound - 1
                };
                append(relation.less, interval.first.clone(), last);
            }
            if (KEEP_COMPLEMENT || relation.equal) && &interval.first <= bound && bound <= &interval.last {
                append(relation.equal, bound.clone(), bound.clone());
            }
            if (KEEP_COMPLEMENT || relation.greater) && &interval.last > bound {
                let first = if &interval.first > bound {
                    interval.first.clone()
                } else {
                    bound + 1
                };
                append(relation.greater, first, interval.last.clone());
            }
        }
        (yes, no)
    }

    pub(super) fn join(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }

        let mut left = self.0.into_iter().peekable();
        let mut right = other.0.into_iter().peekable();
        let mut joined = Self::default();

        while let (Some(a), Some(b)) = (left.peek(), right.peek()) {
            let interval = match a.first.cmp(&b.first) {
                Ordering::Less | Ordering::Equal => left.next(),
                Ordering::Greater => right.next(),
            };
            joined.append(interval.expect("nonempty input interval stream"));
        }
        for interval in left.chain(right) {
            joined.append(interval);
        }
        joined
    }

    pub(super) fn union<'a>(sets: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut intervals: Vec<_> = sets.into_iter().flat_map(|set| &set.0).collect();
        intervals.sort_unstable_by(|left, right| left.first.cmp(&right.first));
        let mut intervals = intervals.into_iter();
        let Some(first) = intervals.next() else { return Self::default(); };
        let mut first_bound = &first.first;
        let mut last_bound = &first.last;
        let mut union = Self::default();
        for interval in intervals {
            if &interval.last <= last_bound {
                continue;
            }
            if overlapping_or_adjacent(&interval.first, last_bound) {
                last_bound = &interval.last;
            } else {
                union.0.push(Interval { first: first_bound.clone(), last: last_bound.clone() });
                first_bound = &interval.first;
                last_bound = &interval.last;
            }
        }
        union.0.push(Interval { first: first_bound.clone(), last: last_bound.clone() });
        union
    }

    pub(super) fn intersection(&self, other: &Self) -> IntegerIntersection {
        let mut changed = None::<Vec<Interval>>;
        let mut matched = 0;
        let (mut left, mut right) = (0, 0);
        while let (Some(a), Some(b)) = (self.0.get(left), other.0.get(right)) {
            let first = (&a.first).max(&b.first);
            let last = (&a.last).min(&b.last);
            if first <= last {
                if let Some(changed) = changed.as_mut() {
                    changed.push(Interval { first: first.clone(), last: last.clone() });
                } else if self.0.get(matched).is_some_and(|prior| &prior.first == first && &prior.last == last) {
                    matched += 1;
                } else {
                    let mut intervals = self.0[..matched].to_vec();
                    intervals.push(Interval { first: first.clone(), last: last.clone() });
                    changed = Some(intervals);
                }
            }
            match a.last.cmp(&b.last) {
                Ordering::Less => left += 1,
                Ordering::Greater => right += 1,
                Ordering::Equal => {
                    left += 1;
                    right += 1;
                }
            }
        }
        match changed {
            Some(intervals) => IntegerIntersection::Changed(Self(intervals)),
            None if matched == 0 => IntegerIntersection::Empty,
            None if matched == self.0.len() => IntegerIntersection::Unchanged,
            None => IntegerIntersection::Changed(Self(self.0[..matched].to_vec())),
        }
    }

    fn append(&mut self, interval: Interval) {
        if let Some(previous) = self
            .0
            .last_mut()
            .filter(|previous| interval.first <= &previous.last + 1)
        {
            if interval.last > previous.last {
                previous.last = interval.last;
            }
        } else {
            self.0.push(interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "internal-checker")]
    #[test]
    fn comparison_alternatives_materializes_exact_normalized_clause_domains() {
        let atom = |operator, value| (IntegerRelation::from_operator(&operator).unwrap(), BigInt::from(value));
        let clauses = vec![
            vec![atom(BinaryOperator::GreaterEqual, -8), atom(BinaryOperator::NotEqual, -4), atom(BinaryOperator::Less, 0)],
            vec![atom(BinaryOperator::Greater, 1), atom(BinaryOperator::LessEqual, 4)],
            vec![atom(BinaryOperator::GreaterEqual, 4), atom(BinaryOperator::LessEqual, 8)],
            vec![atom(BinaryOperator::Greater, 127)],
            vec![atom(BinaryOperator::Less, -128)],
            vec![atom(BinaryOperator::Equal, 1), atom(BinaryOperator::Greater, 1)],
        ];
        let expected = clauses.iter().fold(IntegerSet::default(), |prior, atoms| {
            prior.join(atoms.iter().fold(IntegerSet::from_type(&ScalarType::I8).unwrap(), |domain, (relation, bound)| {
                domain.keep_relation(*relation, bound)
            }))
        });
        let exact = IntegerSet::comparison_alternatives(&ScalarType::I8, clauses.iter().map(Vec::as_slice)).unwrap();
        assert_eq!(exact, expected);
        assert_eq!(exact.0.len(), 3);
        assert!(!exact.contains(&BigInt::from(-4)));
        assert!(!exact.contains(&BigInt::from(1)));
    }

    #[test]
    fn union_materializes_only_normalized_disjoint_ranges() {
        let wide = BigInt::from(1u8) << 300usize;
        let at = |first: i32, last: i32| Interval { first: -&wide + first, last: -&wide + last };
        let left = IntegerSet(vec![at(-6, -3), at(4, 5), at(20, 24)]);
        let right = IntegerSet(vec![at(-9, -7), at(-5, 1), at(3, 4), at(22, 25)]);
        let covered = IntegerSet(vec![at(-9, -9), at(-2, 0), at(20, 21)]);
        assert_eq!(
            IntegerSet::union([&right, &left, &covered, &left, &right]),
            IntegerSet(vec![at(-9, 1), at(3, 5), at(20, 25)]),
        );
        assert_eq!(IntegerSet::union([]), IntegerSet::default());
    }

    #[test]
    fn intersection_materializes_only_changed_nonempty_intervals() {
        let isolated = |value: u8| IntegerSet::singleton(&BigInt::from(value));
        let original = isolated(1).join(isolated(4)).join(isolated(7));
        let superset = isolated(1).join(isolated(2)).join(isolated(4)).join(isolated(7));
        assert_eq!(original.intersection(&superset), IntegerIntersection::Unchanged);
        assert_eq!(original.intersection(&isolated(2)), IntegerIntersection::Empty);
        assert_eq!(original.intersection(&isolated(1).join(isolated(4))),
            IntegerIntersection::Changed(isolated(1).join(isolated(4))));
        assert_eq!(original.intersection(&isolated(1).join(isolated(7))),
            IntegerIntersection::Changed(isolated(1).join(isolated(7))));
    }

    #[test]
    fn branch_condition_proves_local_bound_and_join_keeps_only_common_facts() {
        let input = IntegerSet::from_type(&ScalarType::U8).unwrap();
        let (yes, no) = input.split(&BinaryOperator::Greater, &10.into()).unwrap();
        let (passing, failing) = yes.split(&BinaryOperator::GreaterEqual, &11.into()).unwrap();
        assert!(!passing.is_empty());
        assert!(failing.is_empty());

        let joined = yes.join(no);
        assert_eq!(joined, input);
        let (passing, failing) = joined.split(&BinaryOperator::GreaterEqual, &11.into()).unwrap();
        assert!(!passing.is_empty());
        assert!(!failing.is_empty());
        let (_, failing) = joined.split(&BinaryOperator::LessEqual, &255.into()).unwrap();
        assert!(failing.is_empty());
    }

    #[test]
    fn disequality_keeps_the_hole_and_refines_against_it() {
        let input = IntegerSet::from_type(&ScalarType::U8).unwrap();
        let (excluding, matching) = input.split(&BinaryOperator::NotEqual, &10.into()).unwrap();
        let (impossible, unchanged) = excluding.split(&BinaryOperator::Equal, &10.into()).unwrap();
        assert!(impossible.is_empty());
        assert_eq!(unchanged, excluding);
        let not_equal = IntegerRelation::from_operator(&BinaryOperator::NotEqual).unwrap();
        assert!(excluding.every(not_equal, &10.into()));
        assert!(!input.every(not_equal, &10.into()));
        assert_eq!(excluding.join(matching), input);
    }

    #[test]
    fn comparison_partitions_match_concrete_u8_values() {
        let input = IntegerSet::from_type(&ScalarType::U8).unwrap();
        let cases: [(BinaryOperator, fn(i32, i32) -> bool); 6] = [
            (BinaryOperator::Equal, |value, bound| value == bound),
            (BinaryOperator::NotEqual, |value, bound| value != bound),
            (BinaryOperator::Less, |value, bound| value < bound),
            (BinaryOperator::LessEqual, |value, bound| value <= bound),
            (BinaryOperator::Greater, |value, bound| value > bound),
            (BinaryOperator::GreaterEqual, |value, bound| value >= bound),
        ];
        for (operator, relation) in cases {
            for bound in [-1, 0, 1, 126, 254, 255, 256] {
                let (yes, no) = input.split(&operator, &bound.into()).unwrap();
                assert_eq!(input.keep(&operator, &bound.into()), Some(yes.clone()));
                let abstract_relation = IntegerRelation::from_operator(&operator).unwrap();
                assert_eq!(input.every(abstract_relation, &bound.into()), no.is_empty());
                assert!(yes.every(abstract_relation, &bound.into()));
                assert_eq!(no.every(abstract_relation, &bound.into()), no.is_empty());
                for value in 0..=255 {
                    let contains = |set: &IntegerSet| {
                        let value = BigInt::from(value);
                        set.0
                            .iter()
                            .any(|interval| interval.first <= value && value <= interval.last)
                    };
                    assert_eq!(contains(&yes), relation(value, bound));
                    assert_eq!(contains(&no), !relation(value, bound));
                }
                assert_eq!(yes.join(no), input);
            }
        }
    }

    #[test]
    fn extreme_bounds_do_not_overflow() {
        let signed = IntegerSet::from_type(&ScalarType::I128).unwrap();
        let (impossible, unchanged) = signed
            .split(&BinaryOperator::Less, &i128::MIN.into())
            .unwrap();
        assert!(impossible.is_empty());
        assert_eq!(unchanged, signed);

        let unsigned = IntegerSet::from_type(&ScalarType::U128).unwrap();
        let (impossible, unchanged) = unsigned
            .split(&BinaryOperator::Greater, &u128::MAX.into())
            .unwrap();
        assert!(impossible.is_empty());
        assert_eq!(unchanged, unsigned);
    }
}
