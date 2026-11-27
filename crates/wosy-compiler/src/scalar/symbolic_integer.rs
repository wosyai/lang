use std::cmp::Ordering;

use num_bigint::BigInt;

use super::{typecheck::integer_bounds, BinaryOperator, ScalarType};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Interval {
    first: BigInt,
    last: BigInt,
}

#[cfg(feature = "internal-checker")]
fn literal_point<'a>(interval: &'a Interval, relation: IntegerRelation, bound: &'a BigInt) -> Option<&'a BigInt> {
    if relation.accepts(interval.first.cmp(bound)) {
        Some(&interval.first)
    } else if relation.accepts(interval.last.cmp(bound)) {
        Some(&interval.last)
    } else if interval.first <= *bound && *bound <= interval.last && relation.accepts(Ordering::Equal) {
        Some(bound)
    } else {
        None
    }
}

fn overlapping_or_adjacent(first: &BigInt, last: &BigInt) -> bool {
    first <= last || *first == last + 1
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct IntegerSet(Vec<Interval>);

#[cfg(feature = "internal-checker")]
pub(super) enum IntegerSetPartition {
    True,
    False,
    Mixed(IntegerSet, IntegerSet),
}

pub(super) fn consecutive_integer_cuts(lower: &BigInt, upper: &BigInt) -> bool {
    use num_bigint::Sign::Minus;
    let (smaller, larger) = match (lower.sign() == Minus, upper.sign() == Minus) {
        (true, false) => return upper.bits() == 0 && lower.bits() == 1,
        (true, true) => (upper, lower),
        _ => (lower, upper),
    };
    let mut left = smaller.iter_u64_digits();
    let mut right = larger.iter_u64_digits();
    let mut carry = true;
    loop {
        match (left.next(), right.next()) {
            (None, None) => return !carry,
            (Some(a), Some(b)) => {
                let (sum, next) = a.overflowing_add(u64::from(carry));
                if sum != b { return false; }
                carry = next;
            }
            (None, Some(1)) if carry => return right.next().is_none(),
            _ => return false,
        }
    }
}

#[derive(Clone, Copy)]
struct CutCell<'a> {
    lower: (&'a BigInt, bool),
    upper: (&'a BigInt, bool),
    accepts: bool,
}

impl CutCell<'_> {
    fn nonempty(self) -> bool {
        match self.lower.0.cmp(self.upper.0) {
            Ordering::Less => self.lower.1 || self.upper.1 || !consecutive_integer_cuts(self.lower.0, self.upper.0),
            Ordering::Equal => self.lower.1 && self.upper.1,
            Ordering::Greater => false,
        }
    }

    fn materialize(self) -> Interval {
        Interval {
            first: if self.lower.1 { self.lower.0.clone() } else { self.lower.0 + 1 },
            last: if self.upper.1 { self.upper.0.clone() } else { self.upper.0 - 1 },
        }
    }
}

pub(super) struct IntegerCutUnion<'a> {
    cells: [Option<CutCell<'a>>; 5],
    len: usize,
}

impl<'a> IntegerCutUnion<'a> {
    pub(super) fn new(
        first: (IntegerRelation, &'a BigInt), second: (IntegerRelation, &'a BigInt),
        (minimum, maximum): (&'a BigInt, &'a BigInt),
    ) -> Self {
        use Ordering::{Equal, Greater, Less};
        let ((lo_rel, lo), (hi_rel, hi)) = if first.1 <= second.1 { (first, second) } else { (second, first) };
        let at = |lower, upper, a, b| CutCell { lower, upper, accepts: lo_rel.accepts(a) || hi_rel.accepts(b) };
        let mut union = Self { cells: [None; 5], len: 0 };
        union.push(at((minimum, true), (lo, false), Less, Less));
        if lo == hi {
            union.push(at((lo, true), (lo, true), Equal, Equal));
        } else {
            union.push(at((lo, true), (lo, true), Equal, Less));
            union.push(at((lo, false), (hi, false), Greater, Less));
            union.push(at((hi, true), (hi, true), Greater, Equal));
        }
        union.push(at((hi, false), (maximum, true), Greater, Greater));
        union
    }

    fn push(&mut self, cell: CutCell<'a>) {
        if !cell.nonempty() {
            return;
        }
        if let Some(previous) = self.len.checked_sub(1).and_then(|index| self.cells[index].as_mut()) {
            if previous.accepts == cell.accepts {
                previous.upper = cell.upper;
                return;
            }
        }
        self.cells[self.len] = Some(cell);
        self.len += 1;
    }

    fn for_each_clipped<'clip>(
        &'clip self, active: &'clip Interval, cursor: &mut usize, mut visit: impl FnMut(CutCell<'clip>),
    ) {
        while self.cells.get(*cursor).copied().flatten().is_some_and(|cell|
            cell.upper.0 < &active.first || (cell.upper.0 == &active.first && !cell.upper.1))
        {
            *cursor += 1;
        }
        while let Some(cell) = self.cells.get(*cursor).copied().flatten() {
            if cell.lower.0 > &active.last || (cell.lower.0 == &active.last && !cell.lower.1) {
                break;
            }
            let lower = if &active.first > cell.lower.0 { (&active.first, true) } else { cell.lower };
            let upper = if &active.last < cell.upper.0 { (&active.last, true) } else { cell.upper };
            let clipped = CutCell { lower, upper, accepts: cell.accepts };
            if clipped.nonempty() {
                visit(clipped);
            }
            if cell.upper.0 > &active.last {
                break;
            }
            *cursor += 1;
        }
    }

    pub(super) fn max_added_intervals(&self) -> usize {
        self.len.saturating_sub(1) / 2
    }

    pub(super) fn uniform(&self) -> Option<bool> {
        if self.len == 1 { self.cells[0].map(|cell| cell.accepts) } else { None }
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn integer_predicate_pair_implies(
    first: (IntegerRelation, &BigInt), second: (IntegerRelation, &BigInt),
    conclusion: (IntegerRelation, &BigInt), (minimum, maximum): (&BigInt, &BigInt),
) -> bool {
    use std::borrow::Cow;

    #[derive(Clone, Copy)]
    struct Cuts<'a> {
        lower: (&'a BigInt, bool),
        upper: (&'a BigInt, bool),
        excluded: [Option<&'a BigInt>; 3],
    }

    impl<'a> Cuts<'a> {
        fn add(&mut self, (relation, value): (IntegerRelation, &'a BigInt), slot: usize) {
            if !relation.less && (value > self.lower.0 || (value == self.lower.0 && !relation.equal)) {
                self.lower = (value, relation.equal);
            }
            if !relation.greater && (value < self.upper.0 || (value == self.upper.0 && !relation.equal)) {
                self.upper = (value, relation.equal);
            }
            if relation.less && relation.greater && !relation.equal {
                self.excluded[slot] = Some(value);
            }
        }

        fn nonempty(self) -> bool {
            let (lower, lower_inclusive) = self.lower;
            let (upper, upper_inclusive) = self.upper;
            if lower > upper || (lower == upper && (!lower_inclusive || !upper_inclusive)) {
                return false;
            }
            let mut candidate = if lower_inclusive { Cow::Borrowed(lower) } else { Cow::Owned(lower + 1) };
            while self.excluded.contains(&Some(candidate.as_ref())) {
                if candidate.as_ref() >= upper {
                    return false;
                }
                candidate = Cow::Owned(candidate.as_ref() + 1);
            }
            candidate.as_ref() < upper || (upper_inclusive && candidate.as_ref() == upper)
        }
    }

    let mut premise = Cuts { lower: (minimum, true), upper: (maximum, true), excluded: [None; 3] };
    premise.add(first, 0);
    premise.add(second, 1);
    if !premise.nonempty() {
        return false;
    }
    premise.add((conclusion.0.complement(), conclusion.1), 2);
    !premise.nonempty()
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
pub(super) struct OwnedIntegerUnion {
    intervals: Option<IntegerSet>,
    needs_normalization: bool,
}

#[cfg(feature = "internal-checker")]
impl OwnedIntegerUnion {
    pub(super) fn push(&mut self, mut domain: IntegerSet) {
        if let Some(current) = self.intervals.as_mut() {
            current.0.append(&mut domain.0);
            self.needs_normalization = true;
        } else {
            self.intervals = Some(domain);
        }
    }

    pub(super) fn finish(self) -> Option<IntegerSet> {
        let mut intervals = self.intervals?;
        if self.needs_normalization {
            IntegerSet::normalize_owned_intervals(&mut intervals.0);
        }
        Some(intervals)
    }
}

#[cfg(feature = "internal-checker")]
pub(super) struct IndexedIntegerOrigins<T> {
    owners: Vec<T>,
    intervals: Vec<(Interval, usize, usize)>,
}

#[cfg(feature = "internal-checker")]
impl<T> Default for IndexedIntegerOrigins<T> {
    fn default() -> Self { Self { owners: Vec::new(), intervals: Vec::new() } }
}

#[cfg(feature = "internal-checker")]
impl<T> IndexedIntegerOrigins<T> {
    pub(super) fn push(&mut self, domain: IntegerSet, owner: T) {
        let id = self.owners.len();
        self.owners.push(owner);
        self.intervals.extend(domain.0.into_iter().map(|interval| (interval, id, 0)));
    }

    pub(super) fn finish(&mut self) -> Option<IntegerSet> {
        if self.intervals.is_empty() { return None; }
        // One sort gives both the exact merged exclusion set and a bounded
        // predecessor query into the original contributing intervals.
        self.intervals.sort_unstable_by(|a, b| a.0.first.cmp(&b.0.first).then(a.1.cmp(&b.1)));
        let mut prefix_max = 0;
        let mut union: Vec<Interval> = Vec::new();
        for i in 0..self.intervals.len() {
            if self.intervals[i].0.last > self.intervals[prefix_max].0.last { prefix_max = i; }
            self.intervals[i].2 = prefix_max;
            let interval = &self.intervals[i].0;
            if let Some(prior) = union.last_mut().filter(|prior| overlapping_or_adjacent(&interval.first, &prior.last)) {
                if interval.last > prior.last { prior.last.clone_from(&interval.last); }
            } else {
                union.push(interval.clone());
            }
        }
        Some(IntegerSet(union))
    }

    pub(super) fn interval_count(&self) -> usize { self.intervals.len() }

    pub(super) fn containing_origin(&self, value: &BigInt) -> Option<&T> {
        let prior = self.intervals.partition_point(|(range, ..)| range.first <= *value).checked_sub(1)?;
        let (range, id, _) = self.intervals.get(self.intervals[prior].2)?;
        (range.last >= *value).then(|| &self.owners[*id])
    }
}

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

#[cfg(feature = "internal-checker")]
#[derive(Default)]
pub(super) struct IntegerShowConstraints<'a> {
    lower: Option<(std::borrow::Cow<'a, BigInt>, bool)>,
    upper: Option<(std::borrow::Cow<'a, BigInt>, bool)>,
    excluded: Vec<std::borrow::Cow<'a, BigInt>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> IntegerShowConstraints<'a> {
    pub(super) fn add(&mut self, relation: IntegerRelation, bound: std::borrow::Cow<'a, BigInt>) {
        let stricter = |candidate: &BigInt, current: &BigInt, open: bool, current_open: bool, lower| {
            let order = candidate.cmp(current);
            order == (if lower { Ordering::Greater } else { Ordering::Less })
                || (order == Ordering::Equal && open && !current_open)
        };
        let open = !relation.equal;
        let lower = !relation.less && self.lower.as_ref().is_none_or(|(current, was_open)|
            stricter(&bound, current, open, *was_open, true));
        let upper = !relation.greater && self.upper.as_ref().is_none_or(|(current, was_open)|
            stricter(&bound, current, open, *was_open, false));
        match (lower, upper) {
            (true, true) => {
                self.lower = Some((bound.clone(), open));
                self.upper = Some((bound, open));
            }
            (true, false) => self.lower = Some((bound, open)),
            (false, true) => self.upper = Some((bound, open)),
            (false, false) if relation.less && relation.greater && open => self.excluded.push(bound),
            (false, false) => {}
        }
    }

    pub(super) fn implied_by(
        &self, premise: (IntegerRelation, &BigInt),
        mut implies: impl FnMut(IntegerRelation, &BigInt) -> bool,
    ) -> bool {
        self.lower.as_ref().is_none_or(|(bound, open)| {
            implies(IntegerRelation { less: false, equal: !open, greater: true }, bound)
        }) && self.upper.as_ref().is_none_or(|(bound, open)| {
            implies(IntegerRelation { less: true, equal: !open, greater: false }, bound)
        }) && self.excluded.iter().all(|hole| !premise.0.accepts(hole.as_ref().cmp(premise.1)))
    }

    pub(super) fn normalize(&mut self) {
        self.excluded.sort_unstable();
        self.excluded.dedup();
    }

    fn endpoints_hold(&self, first: &BigInt, last: &BigInt) -> bool {
        self.lower.as_ref().is_none_or(|(bound, open)| first > bound.as_ref() || (!open && first == bound.as_ref()))
            && self.upper.as_ref().is_none_or(|(bound, open)| last < bound.as_ref() || (!open && last == bound.as_ref()))
    }

    pub(super) fn first_violation_in_interval<'b>(&'b self, low: &'b BigInt, high: &'b BigInt) -> Option<&'b BigInt> {
        if !self.endpoints_hold(low, low) { return Some(low); }
        if !self.endpoints_hold(high, high) { return Some(high); }
        self.excluded.get(self.excluded.partition_point(|hole| hole.as_ref() < low))
            .map(std::borrow::Cow::as_ref).filter(|hole| *hole <= high)
    }

    pub(super) fn interval_lookup_work(&self) -> usize {
        (usize::BITS - self.excluded.len().leading_zeros()) as usize + 2
    }

    pub(super) fn holds_value(&self, value: &BigInt) -> bool {
        self.endpoints_hold(value, value) && self.excluded.binary_search_by(|hole| hole.as_ref().cmp(value)).is_err()
    }

    pub(super) fn holds_set(&self, values: &IntegerSet) -> bool {
        let Some((first, last)) = values.extremes() else { return true; };
        if !self.endpoints_hold(first, last) {
            return false;
        }
        let mut holes = self.excluded[self.excluded.partition_point(|hole| hole.as_ref() < first)..].iter().peekable();
        for interval in &values.0 {
            while holes.peek().is_some_and(|hole| hole.as_ref() < &interval.first) {
                holes.next();
            }
            match holes.peek() {
                None => break,
                Some(hole) if hole.as_ref() <= &interval.last => return false,
                _ => {}
            }
        }
        true
    }

    pub(super) fn holds_set_avoiding(&self, values: &IntegerSet, forbidden: &IntegerSet) -> bool {
        let Some((first, last)) = values.extremes() else { return true; };
        if forbidden.extremes().is_none_or(|(start, end)| last < start || end < first) {
            return self.holds_set(values);
        }
        if !self.endpoints_hold(first, last) {
            return false;
        }
        let mut holes = self.excluded[self.excluded.partition_point(|hole| hole.as_ref() < first)..].iter().peekable();
        let mut forbidden = forbidden.0[forbidden.0.partition_point(|interval| &interval.last < first)..].iter().peekable();
        for output in &values.0 {
            while holes.peek().is_some_and(|hole| hole.as_ref() < &output.first) {
                holes.next();
            }
            if holes.peek().is_some_and(|hole| hole.as_ref() <= &output.last) {
                return false;
            }
            while forbidden.peek().is_some_and(|interval| interval.last < output.first) {
                forbidden.next();
            }
            if forbidden.peek().is_some_and(|interval| interval.first <= output.last) {
                return false;
            }
            if holes.peek().is_none() && forbidden.peek().is_none() {
                break;
            }
        }
        true
    }
}

impl IntegerSet {
    #[cfg(feature = "internal-checker")]
    pub(super) fn ordinary_bool_domain() -> Self {
        Self(vec![Interval { first: 0u8.into(), last: 1u8.into() }])
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn ordinary_character_domain() -> Self {
        // Unicode scalar values exclude the surrogate range.
        Self(vec![
            Interval { first: 0u32.into(), last: 0xd7ffu32.into() },
            Interval { first: 0xe000u32.into(), last: u32::from(char::MAX).into() },
        ])
    }

    pub(super) fn from_type(ty: &ScalarType) -> Option<Self> {
        let (first, last) = integer_bounds(ty)?;
        Some(Self(vec![Interval { first, last }]))
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn closed_interval(first: BigInt, last: BigInt) -> Option<Self> {
        (first <= last).then(|| Self(vec![Interval { first, last }]))
    }

    pub(super) fn singleton(value: &BigInt) -> Self {
        Self(vec![Interval { first: value.clone(), last: value.clone() }])
    }

    pub(super) fn affine_unit(&self, reflected: bool, offset: &BigInt, inverse: bool) -> Self {
        if reflected {
            Self(self.0.iter().rev().map(|interval| Interval {
                first: offset - &interval.last,
                last: offset - &interval.first,
            }).collect())
        } else if inverse {
            Self(self.0.iter().map(|interval| Interval {
                first: &interval.first - offset,
                last: &interval.last - offset,
            }).collect())
        } else {
            Self(self.0.iter().map(|interval| Interval {
                first: &interval.first + offset,
                last: &interval.last + offset,
            }).collect())
        }
    }

    pub(super) fn singleton_value(&self) -> Option<&BigInt> {
        match self.0.as_slice() {
            [interval] if interval.first == interval.last => Some(&interval.first),
            _ => None,
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn into_singleton(mut self) -> Option<BigInt> {
        self.singleton_value()?;
        self.0.pop().map(|interval| interval.first)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn into_extremum(self, lower: bool) -> Option<BigInt> {
        let mut intervals = self.0.into_iter();
        if lower {
            intervals.next().map(|interval| interval.first)
        } else {
            intervals.next_back().map(|interval| interval.last)
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn first_comparison_point<'a>(
        &'a self, relation: IntegerRelation, bound: &'a BigInt,
    ) -> Option<&'a BigInt> {
        self.0.iter().find_map(|interval| literal_point(interval, relation, bound))
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn first_guarded_failure(
        &self, guard: (IntegerRelation, &BigInt), nonfailure: &IntegerCutUnion<'_>,
    ) -> (bool, Option<BigInt>) {
        if let Some(passes) = nonfailure.uniform() {
            return if passes {
                (self.first_comparison_point(guard.0, guard.1).is_some(), None)
            } else {
                let failed = self.0.first().map(|interval| interval.first.clone());
                (failed.is_some(), failed)
            };
        }
        let mut applicable = false;
        let mut cursor = 0;
        for interval in &self.0 {
            if !applicable {
                applicable = literal_point(interval, guard.0, guard.1).is_some();
            }
            let mut failed = None;
            nonfailure.for_each_clipped(interval, &mut cursor, |part| {
                if !part.accepts && failed.is_none() {
                    failed = Some(if part.lower.1 { part.lower.0.clone() } else { part.lower.0 + 1 });
                }
            });
            if failed.is_some() {
                return (true, failed);
            }
        }
        (applicable, None)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn intervals(&self) -> impl ExactSizeIterator<Item = (&BigInt, &BigInt)> {
        self.0.iter().map(|interval| (&interval.first, &interval.last))
    }

    pub(super) fn interval_count(&self) -> usize {
        self.0.len()
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn first_intersection_point<'a>(&'a self, low: &'a BigInt, high: &BigInt) -> Option<&'a BigInt> {
        let interval = self.0.get(self.0.partition_point(|part| &part.last < low))?;
        (&interval.first <= high).then_some(std::cmp::max(&interval.first, low))
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

    #[cfg(feature = "internal-checker")]
    pub(super) fn first_uncovered_in_interval(&self, lower: &BigInt, upper: &BigInt) -> Option<BigInt> {
        let next = self.0.partition_point(|range| &range.last < lower);
        let Some(interval) = self.0.get(next).filter(|range| &range.first <= lower) else {
            return Some(lower.clone());
        };
        (&interval.last < upper).then(|| &interval.last + 1u8)
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
    pub(super) fn complement_in_type(self, ty: &ScalarType) -> Option<Self> {
        let (minimum, maximum) = integer_bounds(ty)?;
        let mut next = minimum;
        let mut missing = Vec::with_capacity(self.0.len().checked_add(1)?);
        for interval in self.0 {
            if interval.first < next || interval.last > maximum { return None; }
            if next < interval.first {
                missing.push(Interval { first: next, last: &interval.first - 1u8 });
            }
            next = interval.last + 1u8;
        }
        if next <= maximum {
            missing.push(Interval { first: next, last: maximum });
        }
        Some(Self(missing))
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn first_uncovered_from(&self, allowed: &Self) -> Option<BigInt> {
        let mut enclosing = allowed.0.iter().peekable();
        for range in &self.0 {
            while enclosing.peek().is_some_and(|allowed| allowed.last < range.first) {
                enclosing.next();
            }
            let Some(interval) = enclosing.peek().filter(|allowed| allowed.first <= range.first) else {
                return Some(range.first.clone());
            };
            if interval.last < range.last {
                return Some(&interval.last + 1u8);
            }
        }
        None
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn is_disjoint(&self, other: &Self) -> bool {
        let (Some((first, last)), Some((other_first, other_last))) = (self.extremes(), other.extremes()) else {
            return true;
        };
        if last < other_first || other_last < first {
            return true;
        }

        let (mut left, mut right) = (self.0.iter(), other.0.iter());
        let (mut a, mut b) = (left.next(), right.next());
        while let (Some(x), Some(y)) = (a, b) {
            if x.last < y.first {
                a = left.next();
            } else if y.last < x.first {
                b = right.next();
            } else {
                return false;
            }
        }
        true
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

    #[cfg(feature = "internal-checker")]
    pub(super) fn split_relation(&self, relation: IntegerRelation, bound: &BigInt) -> (Self, Self) {
        self.partition::<true>(relation, bound)
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
        Self::normalize_owned_intervals(&mut intervals);
        Some(Self(intervals))
    }

    #[cfg(feature = "internal-checker")]
    fn normalize_owned_intervals(intervals: &mut Vec<Interval>) {
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

    #[cfg(feature = "internal-checker")]
    pub(super) fn partition_by(&self, accepted: &IntegerCutUnion<'_>) -> IntegerSetPartition {
        if let Some(side) = accepted.uniform() {
            return if side { IntegerSetPartition::True } else { IntegerSetPartition::False };
        }
        let mut uniform = None;
        let mut outputs: Option<[Vec<Interval>; 2]> = None;
        let mut cursor = 0;
        for (index, active) in self.0.iter().enumerate() {
            let mut parts = [None; 5];
            let mut len = 0;
            accepted.for_each_clipped(active, &mut cursor, |clipped| {
                parts[len] = Some(clipped);
                len += 1;
            });
            if len == 1 {
                let side = parts[0].expect("single partition segment").accepts as usize;
                if outputs.is_none() && uniform.is_none_or(|prior| prior == side) {
                    uniform = Some(side);
                    continue;
                }
            }
            let ranges = outputs.get_or_insert_with(|| {
                let mut ranges = [Vec::new(), Vec::new()];
                if let Some(side) = uniform {
                    ranges[side].extend_from_slice(&self.0[..index]);
                }
                ranges
            });
            for part in parts[..len].iter().flatten() {
                ranges[part.accepts as usize].push(part.materialize());
            }
        }
        match outputs {
            Some([no, yes]) => IntegerSetPartition::Mixed(Self(yes), Self(no)),
            None if uniform == Some(1) => IntegerSetPartition::True,
            None => IntegerSetPartition::False,
        }
    }

    pub(super) fn filter_by(&self, cuts: &IntegerCutUnion<'_>, keep_passing: bool) -> IntegerIntersection {
        let mut cursor = 0;
        let mut matched = 0;
        let mut changed: Option<Vec<Interval>> = None;
        for (index, active) in self.0.iter().enumerate() {
            cuts.for_each_clipped(active, &mut cursor, |clipped| {
                if clipped.accepts != keep_passing {
                    return;
                }
                if let Some(intervals) = &mut changed {
                    intervals.push(clipped.materialize());
                } else if index == matched && clipped.lower == (&active.first, true) && clipped.upper == (&active.last, true) {
                    matched += 1;
                } else {
                    let mut intervals = self.0[..matched].to_vec();
                    intervals.push(clipped.materialize());
                    changed = Some(intervals);
                }
            });
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
    fn normalized_partition_matches_every_passing_and_failing_point_of_disconnected_inputs() {
        let domain = |bits: u8| {
            let mut result = IntegerSet::default();
            for point in 0_i32..6 {
                if bits & (1 << point) != 0 {
                    let point = BigInt::from(point - 3);
                    result.append(Interval { first: point.clone(), last: point });
                }
            }
            result
        };
        let points = [-3, -2, -1, 0, 1, 2].map(BigInt::from);
        let relations = [
            BinaryOperator::Equal, BinaryOperator::NotEqual, BinaryOperator::Less,
            BinaryOperator::LessEqual, BinaryOperator::Greater, BinaryOperator::GreaterEqual,
        ].map(|operator| IntegerRelation::from_operator(&operator).unwrap());
        for left in relations {
            for right in relations {
                for left_bound in &points {
                    for right_bound in &points {
                        let accepted = IntegerCutUnion::new((left, left_bound), (right, right_bound), (&points[0], &points[5]));
                        let mask = points.iter().enumerate().fold(0_u8, |bits, (at, point)| {
                            if left.accepts(point.cmp(left_bound)) || right.accepts(point.cmp(right_bound)) {
                                bits | (1 << at)
                            } else {
                                bits
                            }
                        });
                        for input in 1_u8..64 {
                            let yes = input & mask;
                            let no = input & !mask;
                            let original = domain(input);
                            for (keep, expected) in [(true, yes), (false, no)] {
                                match original.filter_by(&accepted, keep) {
                                    IntegerIntersection::Empty => assert_eq!(expected, 0),
                                    IntegerIntersection::Unchanged => assert_eq!(expected, input),
                                    IntegerIntersection::Changed(filtered) => {
                                        assert!(expected != 0 && expected != input);
                                        assert_eq!(filtered, domain(expected));
                                    }
                                }
                            }
                            match original.partition_by(&accepted) {
                                IntegerSetPartition::True => assert_eq!((yes, no), (input, 0)),
                                IntegerSetPartition::False => assert_eq!((yes, no), (0, input)),
                                IntegerSetPartition::Mixed(yes_set, no_set) => {
                                    assert!(yes != 0 && no != 0);
                                    assert_eq!(yes_set, domain(yes));
                                    assert_eq!(no_set, domain(no));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn paired_literal_entailment_matches_all_predicates_on_a_closed_integer_domain() {
        let values = [-2, -1, 0, 1, 2].map(BigInt::from);
        let atoms: Vec<_> = [
            BinaryOperator::Equal, BinaryOperator::NotEqual, BinaryOperator::Less,
            BinaryOperator::LessEqual, BinaryOperator::Greater, BinaryOperator::GreaterEqual,
        ].iter().flat_map(|operator| {
            let relation = IntegerRelation::from_operator(operator).unwrap();
            values.iter().map(move |bound| (relation, bound))
        }).collect();
        for &first in &atoms {
            for &second in &atoms {
                for &goal in &atoms {
                    let mut admitted = false;
                    let mut valid = true;
                    for value in &values {
                        if first.0.accepts(value.cmp(first.1)) && second.0.accepts(value.cmp(second.1)) {
                            admitted = true;
                            valid &= goal.0.accepts(value.cmp(goal.1));
                        }
                    }
                    assert_eq!(integer_predicate_pair_implies(first, second, goal, (&values[0], &values[4])),
                        admitted && valid);
                }
            }
        }
    }

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
#[cfg(feature = "internal-checker")]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SignedRadixProjection {
    pub(super) first: [BigInt; 3],
    pub(super) last: [BigInt; 3],
    pub(super) quotient: IntegerSet,
    pub(super) remainder: IntegerSet,
}

/// Exact projections onto quotient/remainder and feasible extremal triples.
/// The input *enclosure* between those triples is not an exact set: callers
/// must retain this relation when narrowing or selecting a witness.
#[cfg(feature = "internal-checker")]
pub(super) fn signed_radix_projection(
    input: &IntegerSet, quotient: &IntegerSet, remainder: &IntegerSet, radix: &BigInt, work: &mut usize,
) -> Option<Option<SignedRadixProjection>> {
    use super::checker_flow::charge_call_work;
    use num_bigint::Sign;
    if radix.sign() != Sign::Plus { return None; }
    let mut first = None::<[BigInt; 3]>;
    let mut last = None::<[BigInt; 3]>;
    let mut quotient_components = Vec::new();
    let mut remainder_components = Vec::new();
    let zero = BigInt::from(0u8);
    let minus_one = BigInt::from(-1i8);
    let residue_max = radix - 1u8;
    for negative in [false, true] {
        let (operator, boundary) =
            if negative { (BinaryOperator::LessEqual, &minus_one) } else { (BinaryOperator::GreaterEqual, &zero) };
        charge_call_work(work, input.interval_count().checked_add(quotient.interval_count())?
            .checked_add(remainder.interval_count())?.checked_add(1)?)?;
        let input = input.keep(&operator, boundary)?.affine_unit(negative, &zero, false);
        let quotient = quotient.affine_unit(negative, &zero, false);
        let residues = remainder.affine_unit(negative, &zero, false);
        let bounded = IntegerSet::closed_interval(zero.clone(), residue_max.clone())?;
        let residues = match residues.intersection(&bounded) {
            IntegerIntersection::Unchanged => residues,
            IntegerIntersection::Changed(set) => set,
            IntegerIntersection::Empty => continue,
        };
        let mut q_component = Vec::new();
        let mut r_component = Vec::new();
        for (input_lo, input_hi) in input.intervals() {
            charge_call_work(work, 1)?;
            let (ql0, qh0) = (input_lo / radix, input_hi / radix);
            for (allowed_lo, allowed_hi) in quotient.intervals() {
                let lookup = residues.interval_count().checked_ilog2().unwrap_or(0) as usize + 1;
                charge_call_work(work, lookup.checked_mul(4)?.checked_add(1)?)?;
                let mut ql = ql0.clone().max(allowed_lo.clone());
                let mut qh = qh0.clone().min(allowed_hi.clone());
                if ql > qh { continue; }
                let window = |q: &BigInt| {
                    let base = q * radix;
                    let low = (&base).max(input_lo) - &base;
                    let top = &base + &residue_max;
                    let high = (&top).min(input_hi) - base;
                    (low, high)
                };
                let (mut low, mut high) = window(&ql);
                if residues.first_intersection_point(&low, &high).is_none() {
                    ql += 1u8;
                    if ql > qh { continue; }
                    (low, high) = window(&ql);
                }
                let Some(rl) = residues.first_intersection_point(&low, &high).cloned() else { continue; };
                let (mut last_low, mut last_high) = window(&qh);
                if residues.first_intersection_point(&last_low, &last_high).is_none() {
                    qh -= 1u8;
                    if ql > qh { continue; }
                    (last_low, last_high) = window(&qh);
                }
                let rh = last_intersection_point(&residues, &last_low, &last_high)?.clone();
                q_component.push(IntegerSet::closed_interval(ql.clone(), qh.clone())?);

                let q_distance = &qh - &ql;
                if q_distance >= BigInt::from(2u8) {
                    charge_call_work(work, residues.interval_count())?;
                    r_component.push(residues.clone());
                } else {
                    for (low, high) in [(low, high), (last_low, last_high)]
                        .into_iter().take(if q_distance.sign() == Sign::NoSign { 1 } else { 2 }) {
                        charge_call_work(work, residues.interval_count())?;
                        match residues.intersection(&IntegerSet::closed_interval(low, high)?) {
                            IntegerIntersection::Unchanged => r_component.push(residues.clone()),
                            IntegerIntersection::Changed(set) => r_component.push(set),
                            IntegerIntersection::Empty => return None,
                        }
                    }
                }
                let first_abs = [(&ql * radix) + &rl, ql, rl];
                let last_abs = [(&qh * radix) + &rh, qh, rh];
                let (low, high) = if negative { (last_abs.map(|x| -x), first_abs.map(|x| -x)) } else { (first_abs, last_abs) };
                if first.as_ref().is_none_or(|first| low[0] < first[0]) { first = Some(low); }
                if last.as_ref().is_none_or(|last| high[0] > last[0]) { last = Some(high); }
            }
        }
        quotient_components.push(metered_radix_union(&q_component, work)?.affine_unit(negative, &zero, false));
        remainder_components.push(metered_radix_union(&r_component, work)?.affine_unit(negative, &zero, false));
    }
    let (Some(first), Some(last)) = (first, last) else { return Some(None); };
    Some(Some(SignedRadixProjection {
        first, last,
        quotient: metered_radix_union(&quotient_components, work)?,
        remainder: metered_radix_union(&remainder_components, work)?,
    }))
}

#[cfg(feature = "internal-checker")]
fn last_intersection_point<'a>(set: &'a IntegerSet, low: &BigInt, high: &'a BigInt) -> Option<&'a BigInt> {
    let index = set.0.partition_point(|part| &part.first <= high).checked_sub(1)?;
    let interval = set.0.get(index)?;
    (low <= &interval.last).then_some(std::cmp::min(&interval.last, high))
}

#[cfg(feature = "internal-checker")]
fn metered_radix_union(components: &[IntegerSet], work: &mut usize) -> Option<IntegerSet> {
    let n = components.iter().try_fold(0usize, |n, component| n.checked_add(component.interval_count()))?;
    super::checker_flow::charge_call_work(work, n.checked_mul(n.checked_ilog2().unwrap_or(0) as usize + 1)?.checked_add(1)?)?;
    Some(IntegerSet::union(components))
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn signed_radix_projection_matches_exact_signed_pairs() {
    let range = |low, high| IntegerSet::closed_interval(BigInt::from(low), BigInt::from(high)).unwrap();
    let several = |terms: &[(i32, i32)]| IntegerSet::union(terms.iter().map(|&(low, high)| range(low, high)).collect::<Vec<_>>().iter());
    for radix in [1, 2, 3, 4, 7] {
        for input in [several(&[(-31, 32)]), several(&[(-31, -19), (-4, 3), (22, 32)]), range(-11, -2), range(4, 17)] {
            for quotient in [range(-32, 32), range(-4, -2), range(0, 2), several(&[(-4, -3), (0, 0), (3, 5)])] {
                for residue in [range(-32, 32), range(-2, 1), range(-4, -1), several(&[(-3, -2), (1, 2)])] {
                    let expected = (-31..=32).filter_map(|x| {
                        let values = [x, x / radix, x % radix].map(BigInt::from);
                        (input.contains(&values[0]) && quotient.contains(&values[1]) && residue.contains(&values[2])).then_some(values)
                    }).collect::<Vec<_>>();
                    let mut work = 0;
                    let found = signed_radix_projection(&input, &quotient, &residue, &BigInt::from(radix), &mut work).unwrap();
                    if expected.is_empty() { assert!(found.is_none()); continue; }
                    let found = found.unwrap();
                    assert_eq!(Some(&found.first), expected.first());
                    assert_eq!(Some(&found.last), expected.last());
                    for value in -32..=32 {
                        let value = BigInt::from(value);
                        assert_eq!(found.quotient.contains(&value), expected.iter().any(|values| values[1] == value));
                        assert_eq!(found.remainder.contains(&value), expected.iter().any(|values| values[2] == value));
                    }
                }
            }
        }
    }
    let input = range(i32::MIN, i32::MAX);
    let quotient = range(1, i32::MAX);
    let residue = range(2, 2);
    let mut work = 0;
    let found = signed_radix_projection(&input, &quotient, &residue, &BigInt::from(16), &mut work).unwrap().unwrap();
    assert_eq!(found.first, [18, 1, 2].map(BigInt::from));
    assert_eq!(found.last, [2_147_483_634, 134_217_727, 2].map(BigInt::from));
    assert!(work < 200);
    for (divisor, q) in [(1, i32::MIN), (16, -134_217_728)] {
        let mut work = 0;
        let expected = [i32::MIN, q, 0].map(BigInt::from);
        let exact = signed_radix_projection(&input, &range(q, q), &range(0, 0), &BigInt::from(divisor), &mut work)
            .unwrap().unwrap();
        assert_eq!(exact.first, expected);
        assert_eq!(exact.last, expected);
        assert!(work < 200);
    }
    assert!(signed_radix_projection(&input, &quotient, &residue, &BigInt::from(-1), &mut 0).is_none());
}
