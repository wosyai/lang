use std::cmp::Ordering;

use num_bigint::BigInt;

use super::{typecheck::integer_bounds, BinaryOperator, ScalarType};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Interval {
    first: BigInt,
    last: BigInt,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct IntegerSet(Vec<Interval>);

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

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn every(&self, relation: IntegerRelation, bound: &BigInt) -> bool {
        self.0.iter().all(|interval| {
            (relation.less || &interval.first >= bound)
                && (relation.equal || bound < &interval.first || bound > &interval.last)
                && (relation.greater || &interval.last <= bound)
        })
    }

    pub(super) fn split(&self, operator: &BinaryOperator, bound: &BigInt) -> Option<(Self, Self)> {
        Some(self.partition::<true>(IntegerRelation::from_operator(operator)?, bound))
    }

    pub(super) fn keep(&self, operator: &BinaryOperator, bound: &BigInt) -> Option<Self> {
        Some(self.keep_relation(IntegerRelation::from_operator(operator)?, bound))
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
        let mut union = Self::default();
        for interval in intervals {
            union.append(interval.clone());
        }
        union
    }

    pub(super) fn intersection(&self, other: &Self) -> Self {
        let mut result = Self::default();
        let (mut left, mut right) = (0, 0);
        while let (Some(a), Some(b)) = (self.0.get(left), other.0.get(right)) {
            let first = (&a.first).max(&b.first);
            let last = (&a.last).min(&b.last);
            if first <= last {
                result.0.push(Interval { first: first.clone(), last: last.clone() });
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
        result
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
